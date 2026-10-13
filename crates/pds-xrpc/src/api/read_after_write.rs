//! Read-after-write (`docs/SPEC.md` §4.5).
//!
//! Six reads of the AppView show an account its own posts and its own
//! profile. The AppView says in `Atproto-Repo-Rev` how far it has indexed
//! the repository. When the account wrote after that revision, the server
//! adds what it wrote to the answer, as `read-after-write/` of the
//! Reference does, and says in `Atproto-Upstream-Lag` how old the oldest
//! added record is.

use std::future::Future;
use std::pin::Pin;

use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use pds_storage::{ActorDb, Availability, RecordRow};
use serde_json::{Map, Value, json};

use super::proxy;
use crate::error::{XrpcError, json_response};
use crate::state::AppState;

const GET_PROFILE: &str = "app.bsky.actor.getProfile";
const GET_PROFILES: &str = "app.bsky.actor.getProfiles";
const GET_TIMELINE: &str = "app.bsky.feed.getTimeline";
const GET_AUTHOR_FEED: &str = "app.bsky.feed.getAuthorFeed";
const GET_ACTOR_LIKES: &str = "app.bsky.feed.getActorLikes";
const GET_POST_THREAD: &str = "app.bsky.feed.getPostThread";
const PROFILE: &str = "app.bsky.actor.profile";
const POST: &str = "app.bsky.feed.post";
const THREAD_VIEW: &str = "app.bsky.feed.defs#threadViewPost";

/// The largest answer the server reads into memory to change it.
const RESPONSE_LIMIT: usize = 10 * 1024 * 1024;

/// Whether the method is one of the six.
pub fn applies(lxm: &str) -> bool {
    matches!(
        lxm,
        GET_PROFILE
            | GET_PROFILES
            | GET_TIMELINE
            | GET_AUTHOR_FEED
            | GET_ACTOR_LIKES
            | GET_POST_THREAD
    )
}

/// A record the account wrote after the AppView's revision.
#[derive(Debug, Clone)]
struct Described {
    uri: String,
    cid: String,
    indexed_at: String,
    record: Value,
}

/// What the account wrote after a revision, and how it looks now.
#[derive(Debug, Clone, Default)]
struct Local {
    count: usize,
    profile: Option<Described>,
    posts: Vec<Described>,
    /// The account as the author of a post: its handle, and the name and
    /// the picture of its profile record.
    author: Option<Value>,
}

impl Local {
    /// The age of the oldest record, in milliseconds.
    fn lag_ms(&self, now_ms: i64) -> Option<i64> {
        self.profile
            .iter()
            .chain(&self.posts)
            .map(|record| record.indexed_at.as_str())
            .min()
            .and_then(|oldest| chrono::DateTime::parse_from_rfc3339(oldest).ok())
            .map(|oldest| now_ms - oldest.timestamp_millis())
    }
}

fn blob_cid(blob: &Value) -> Option<&str> {
    blob.get("ref")
        .and_then(|link| link.get("$link"))
        .or_else(|| blob.get("cid"))
        .and_then(Value::as_str)
}

/// The URL of an image: the CDN's when one is configured, or `getBlob`
/// here.
fn image_url(state: &AppState, preset: &str, did: &str, cid: &str) -> String {
    let config = state.config();
    match &config.appview_cdn_url_pattern {
        Some(pattern) => [preset, did, cid]
            .iter()
            .fold(pattern.clone(), |url, part| url.replacen("%s", part, 1)),
        None => format!(
            "https://{}/xrpc/com.atproto.sync.getBlob?did={did}&cid={cid}",
            config.hostname
        ),
    }
}

fn set(view: &mut Map<String, Value>, key: &str, value: Option<Value>) {
    match value {
        Some(value) => view.insert(key.to_owned(), value),
        None => view.remove(key),
    };
}

/// How much of a profile view the record replaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Detail {
    Basic,
    Detailed,
}

/// Puts the name, the picture and, in a detailed view, the description and
/// the banner of the profile record into a view of the account.
fn update_profile(state: &AppState, view: &mut Value, record: &Value, did: &str, detail: Detail) {
    let Some(view) = view.as_object_mut() else {
        return;
    };
    let image = |key: &str, preset: &str| {
        record
            .get(key)
            .and_then(blob_cid)
            .map(|cid| Value::String(image_url(state, preset, did, cid)))
    };
    set(view, "displayName", record.get("displayName").cloned());
    set(view, "avatar", image("avatar", "avatar"));
    if detail == Detail::Detailed {
        set(view, "description", record.get("description").cloned());
        set(view, "banner", image("banner", "banner"));
    }
}

fn load(state: &AppState, did: &str, rev: &str) -> Result<Local, XrpcError> {
    let store = ActorDb::open(&state.layout().actor_db(did))?;
    let rows = store.records_since_rev(rev)?;
    let mut local = Local {
        count: rows.len(),
        ..Local::default()
    };
    if rows.is_empty() {
        return Ok(local);
    }
    for recent in rows {
        let pds_storage::RecentRecord {
            uri,
            cid,
            indexed_at,
            content,
        } = recent;
        let mut parts = uri.rsplitn(3, '/');
        let (rkey, collection) = (parts.next(), parts.next());
        let own_profile = collection == Some(PROFILE) && rkey == Some("self");
        if !own_profile && collection != Some(POST) {
            continue;
        }
        let row = RecordRow {
            uri: uri.clone(),
            cid: cid.clone(),
            content,
            takedown_ref: None,
        };
        let described = Described {
            uri,
            cid,
            indexed_at,
            record: super::repo::record_value(&row)?,
        };
        if own_profile {
            local.profile = Some(described);
        } else {
            local.posts.push(described);
        }
    }
    let actor = state.layout().accounts()?.actor(did, Availability::ANY)?;
    if let Some(actor) = actor {
        let mut author = json!({
            "did": did,
            "handle": actor.handle.unwrap_or_else(|| "handle.invalid".to_owned()),
        });
        let uri = format!("at://{did}/{PROFILE}/self");
        if let Some(row) = store.record(&uri, None)? {
            let record = super::repo::record_value(&row)?;
            update_profile(state, &mut author, &record, did, Detail::Basic);
        }
        local.author = Some(author);
    }
    Ok(local)
}

/// A `GET` to the AppView as the account. `None` when the AppView does
/// not answer with JSON.
async fn appview_get(
    state: &AppState,
    did: &str,
    lxm: &str,
    query: &[(&str, &str)],
) -> Option<Value> {
    let config = state.config();
    let (url, aud) = config
        .appview_url
        .as_ref()
        .zip(config.appview_did.as_ref())?;
    let token = proxy::account_jwt(state, did, aud, Some(lxm), None)
        .await
        .ok()?;
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(query)
        .finish();
    let response = state
        .http()
        .get(format!("{url}/xrpc/{lxm}?{query}"))
        .timeout(config.proxy_timeout)
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    serde_json::from_slice(&response.bytes().await.ok()?).ok()
}

fn images_view(state: &AppState, did: &str, embed: &Value) -> Value {
    let images: Vec<Value> = embed
        .get("images")
        .and_then(Value::as_array)
        .map(|images| {
            images
                .iter()
                .map(|image| {
                    let cid = image.get("image").and_then(blob_cid).unwrap_or_default();
                    let mut view = json!({
                        "thumb": image_url(state, "feed_thumbnail", did, cid),
                        "fullsize": image_url(state, "feed_fullsize", did, cid),
                        "alt": image.get("alt").cloned().unwrap_or(Value::Null),
                    });
                    if let Some(ratio) = image.get("aspectRatio") {
                        view["aspectRatio"] = ratio.clone();
                    }
                    view
                })
                .collect()
        })
        .unwrap_or_default();
    json!({ "$type": "app.bsky.embed.images#view", "images": images })
}

fn external_view(state: &AppState, did: &str, embed: &Value) -> Value {
    let external = embed.get("external").cloned().unwrap_or(Value::Null);
    let mut view = json!({
        "uri": external.get("uri").cloned().unwrap_or(Value::Null),
        "title": external.get("title").cloned().unwrap_or(Value::Null),
        "description": external.get("description").cloned().unwrap_or(Value::Null),
    });
    if let Some(cid) = external.get("thumb").and_then(blob_cid) {
        view["thumb"] = Value::String(image_url(state, "feed_thumbnail", did, cid));
    }
    json!({ "$type": "app.bsky.embed.external#view", "external": view })
}

/// The view of a quoted record, which the AppView has.
async fn record_view(state: &AppState, did: &str, embed: &Value) -> Value {
    let uri = embed
        .get("record")
        .and_then(|record| record.get("uri"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let collection = uri.split('/').nth(3).unwrap_or_default();
    let view = match collection {
        POST => appview_get(state, did, "app.bsky.feed.getPosts", &[("uris", uri)])
            .await
            .and_then(|found| found.get("posts")?.get(0).cloned())
            .map(|post| {
                let mut view = json!({
                    "$type": "app.bsky.embed.record#viewRecord",
                    "uri": post["uri"],
                    "cid": post["cid"],
                    "author": post["author"],
                    "value": post["record"],
                    "indexedAt": post["indexedAt"],
                });
                if let Some(labels) = post.get("labels") {
                    view["labels"] = labels.clone();
                }
                if let Some(embed) = post.get("embed") {
                    view["embeds"] = json!([embed]);
                }
                view
            }),
        "app.bsky.feed.generator" => appview_get(
            state,
            did,
            "app.bsky.feed.getFeedGenerator",
            &[("feed", uri)],
        )
        .await
        .and_then(|found| found.get("view").cloned())
        .map(|mut view| {
            view["$type"] = json!("app.bsky.feed.defs#generatorView");
            view
        }),
        "app.bsky.graph.list" => {
            appview_get(state, did, "app.bsky.graph.getList", &[("list", uri)])
                .await
                .and_then(|found| found.get("list").cloned())
                .map(|mut view| {
                    view["$type"] = json!("app.bsky.graph.defs#listView");
                    view
                })
        }
        _ => None,
    };
    let record = view.unwrap_or_else(
        || json!({ "$type": "app.bsky.embed.record#viewNotFound", "uri": uri, "notFound": true }),
    );
    json!({ "$type": "app.bsky.embed.record#view", "record": record })
}

async fn embed_view(state: &AppState, did: &str, embed: &Value) -> Option<Value> {
    match embed.get("$type").and_then(Value::as_str)? {
        "app.bsky.embed.images" => Some(images_view(state, did, embed)),
        "app.bsky.embed.external" => Some(external_view(state, did, embed)),
        "app.bsky.embed.record" => Some(record_view(state, did, embed).await),
        "app.bsky.embed.recordWithMedia" => {
            let media = embed.get("media")?;
            let media = match media.get("$type").and_then(Value::as_str)? {
                "app.bsky.embed.images" => images_view(state, did, media),
                "app.bsky.embed.external" => external_view(state, did, media),
                _ => return None,
            };
            let record = record_view(state, did, embed.get("record")?).await;
            Some(json!({
                "$type": "app.bsky.embed.recordWithMedia#view",
                "record": record,
                "media": media,
            }))
        }
        _ => None,
    }
}

/// A post of the account as the AppView would show it just after it was
/// written: no likes, replies, reposts or quotes yet.
async fn post_view(state: &AppState, did: &str, local: &Local, post: &Described) -> Option<Value> {
    let mut view = json!({
        "uri": post.uri,
        "cid": post.cid,
        "likeCount": 0,
        "replyCount": 0,
        "repostCount": 0,
        "quoteCount": 0,
        "author": local.author.clone()?,
        "record": post.record,
        "indexedAt": post.indexed_at,
    });
    if let Some(embed) = post.record.get("embed")
        && let Some(embed) = embed_view(state, did, embed).await
    {
        view["embed"] = embed;
    }
    Some(view)
}

fn indexed_at(item: &Value) -> &str {
    item.get("post")
        .and_then(|post| post.get("indexedAt"))
        .and_then(Value::as_str)
        .unwrap_or_default()
}

/// Puts the account's new posts into a feed, each before the first item
/// that is older. A post older than the last item of the page belongs to a
/// later page.
async fn insert_posts(state: &AppState, did: &str, local: &Local, feed: &mut Vec<Value>) {
    let last = feed
        .last()
        .map_or("1970-01-01T00:00:00.000Z", indexed_at)
        .to_owned();
    for post in local.posts.iter().rev() {
        if post.indexed_at.as_str() <= last.as_str() {
            continue;
        }
        let Some(view) = post_view(state, did, local, post).await else {
            continue;
        };
        let item = json!({ "post": view });
        match feed
            .iter()
            .position(|other| indexed_at(other) < post.indexed_at.as_str())
        {
            Some(at) => feed.insert(at, item),
            None => feed.push(item),
        }
    }
}

/// Puts the profile record into the author of each feed item that the
/// account wrote.
fn update_authors(state: &AppState, did: &str, profile: &Described, feed: &mut [Value]) {
    for item in feed {
        if let Some(author) = item.get_mut("post").and_then(|post| post.get_mut("author"))
            && author.get("did").and_then(Value::as_str) == Some(did)
        {
            update_profile(state, author, &profile.record, did, Detail::Basic);
        }
    }
}

/// Whether a page of `getAuthorFeed` is the account's own feed: its first
/// item is a post or a repost of the account.
fn is_own_feed(feed: &[Value], did: &str) -> bool {
    let Some(first) = feed.first() else {
        return false;
    };
    let text = |value: &Value, path: [&str; 2]| {
        value
            .get(path[0])
            .and_then(|inner| inner.get(path[1]))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    match first.get("reason") {
        None => {
            first
                .get("post")
                .and_then(|post| text(post, ["author", "did"]))
                .as_deref()
                == Some(did)
        }
        Some(reason) => {
            reason.get("$type").and_then(Value::as_str) == Some("app.bsky.feed.defs#reasonRepost")
                && text(reason, ["by", "did"]).as_deref() == Some(did)
        }
    }
}

fn reply_uri<'a>(record: &'a Value, which: &str) -> Option<&'a str> {
    record.get("reply")?.get(which)?.get("uri")?.as_str()
}

fn is_thread_view(value: &Value) -> bool {
    value.get("$type").and_then(Value::as_str) == Some(THREAD_VIEW)
}

type Boxed<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// Puts a reply of the account under its parent, wherever in the thread
/// the parent is.
fn insert_reply<'a>(
    state: &'a AppState,
    did: &'a str,
    local: &'a Local,
    view: &'a mut Value,
    reply: &'a Described,
) -> Boxed<'a> {
    Box::pin(async move {
        let uri = view
            .get("post")
            .and_then(|post| post.get("uri"))
            .and_then(Value::as_str);
        if reply_uri(&reply.record, "parent") == uri {
            let Some(post) = post_view(state, did, local, reply).await else {
                return;
            };
            let mut replies = vec![json!({ "$type": THREAD_VIEW, "post": post })];
            if let Some(Value::Array(rest)) = view.get_mut("replies").map(Value::take) {
                replies.extend(rest);
            }
            view["replies"] = Value::Array(replies);
            return;
        }
        if let Some(Value::Array(replies)) = view.get_mut("replies") {
            for child in replies.iter_mut().filter(|child| is_thread_view(child)) {
                insert_reply(state, did, local, child, reply).await;
            }
        }
    })
}

/// Puts the account's new posts of this thread into it.
async fn add_to_thread(
    state: &AppState,
    did: &str,
    local: &Local,
    thread: &mut Value,
    posts: &[&Described],
) {
    let post = thread.get("post").cloned().unwrap_or(Value::Null);
    let own_uri = post.get("uri").and_then(Value::as_str).map(str::to_owned);
    let own_root = post
        .get("record")
        .and_then(|record| reply_uri(record, "root"))
        .map(str::to_owned);
    for reply in posts {
        let Some(root) = reply_uri(&reply.record, "root") else {
            continue;
        };
        if Some(root) == own_uri.as_deref() || Some(root) == own_root.as_deref() {
            insert_reply(state, did, local, thread, reply).await;
        }
    }
}

/// The thread of a post that the AppView does not have yet, when the post
/// is the account's own.
async fn thread_not_found(
    state: &AppState,
    did: &str,
    rev: &str,
    query: Option<&str>,
) -> Result<Option<Response>, XrpcError> {
    let params: Vec<(String, String)> =
        url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
            .into_owned()
            .collect();
    let param = |name: &str| {
        params
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };
    let Some(uri) = param("uri") else {
        return Ok(None);
    };
    let Some(rest) = uri.strip_prefix("at://") else {
        return Ok(None);
    };
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let (owned_did, host, rev) = (did.to_owned(), host.to_owned(), rev.to_owned());
    let (resolved, local) = state
        .blocking(move |state| {
            let resolved = if host.starts_with("did:") {
                host
            } else {
                state
                    .layout()
                    .accounts()?
                    .actor(&host, Availability::ANY)?
                    .map_or(host, |actor| actor.did)
            };
            if resolved != owned_did {
                return Ok((resolved, Local::default()));
            }
            Ok((resolved, load(state, &owned_did, &rev)?))
        })
        .await?;
    let uri = format!("at://{resolved}/{path}");
    let Some(found) = local.posts.iter().find(|post| post.uri == uri) else {
        return Ok(None);
    };
    let Some(post) = post_view(state, did, &local, found).await else {
        return Ok(None);
    };
    let mut thread = json!({ "$type": THREAD_VIEW, "post": post });
    let rest: Vec<&Described> = local.posts.iter().filter(|post| post.uri != uri).collect();
    add_to_thread(state, did, &local, &mut thread, &rest).await;
    // The posts above it are the AppView's.
    if let Some(parent) = reply_uri(&found.record, "parent") {
        let mut query = vec![("uri", parent), ("depth", "0")];
        if let Some(height) = param("parentHeight") {
            query.push(("parentHeight", height));
        }
        if let Some(parents) = appview_get(state, did, GET_POST_THREAD, &query).await
            && let Some(parent) = parents.get("thread")
        {
            thread["parent"] = parent.clone();
        }
    }
    Ok(Some(munged(
        &json!({ "thread": thread }),
        local.lag_ms(state.now_ms()),
    )))
}

fn munged(body: &Value, lag_ms: Option<i64>) -> Response {
    let mut response = json_response(StatusCode::OK, body);
    if let Some(value) = lag_ms.and_then(|lag| HeaderValue::from_str(&lag.to_string()).ok()) {
        response.headers_mut().insert("atproto-upstream-lag", value);
    }
    response
}

/// Changes the answer of one of the six methods with what the account
/// wrote.
async fn munge(state: &AppState, lxm: &str, did: &str, local: &Local, body: &mut Value) {
    let own = |view: &Value| view.get("did").and_then(Value::as_str) == Some(did);
    match lxm {
        GET_PROFILE => {
            if let Some(profile) = &local.profile
                && own(body)
            {
                update_profile(state, body, &profile.record, did, Detail::Detailed);
            }
        }
        GET_PROFILES => {
            if let (Some(profile), Some(Value::Array(profiles))) =
                (&local.profile, body.get_mut("profiles"))
            {
                for view in profiles.iter_mut().filter(|view| own(view)) {
                    update_profile(state, view, &profile.record, did, Detail::Detailed);
                }
            }
        }
        GET_TIMELINE => {
            if let Some(Value::Array(feed)) = body.get_mut("feed") {
                insert_posts(state, did, local, feed).await;
            }
        }
        GET_AUTHOR_FEED => {
            if let Some(Value::Array(feed)) = body.get_mut("feed")
                && is_own_feed(feed, did)
            {
                if let Some(profile) = &local.profile {
                    update_authors(state, did, profile, feed);
                }
                insert_posts(state, did, local, feed).await;
            }
        }
        GET_ACTOR_LIKES => {
            if let (Some(profile), Some(Value::Array(feed))) =
                (&local.profile, body.get_mut("feed"))
            {
                update_authors(state, did, profile, feed);
            }
        }
        GET_POST_THREAD => {
            if let Some(thread) = body
                .get_mut("thread")
                .filter(|thread| is_thread_view(thread))
            {
                let posts: Vec<&Described> = local.posts.iter().collect();
                add_to_thread(state, did, local, thread, &posts).await;
            }
        }
        _ => {}
    }
}

/// The answer to one of the six methods: the AppView's, with what the
/// account `did` wrote after the AppView's revision.
pub async fn overlay(
    state: &AppState,
    lxm: &str,
    did: &str,
    query: Option<&str>,
    upstream: reqwest::Response,
) -> Result<Response, XrpcError> {
    let header = |name: &str| {
        upstream
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    let Some(rev) = header("atproto-repo-rev") else {
        return proxy::answer(upstream).await;
    };
    if header("content-type").is_some_and(|kind| !kind.contains("json")) {
        return proxy::answer(upstream).await;
    }
    if upstream.status().as_u16() >= 400 {
        let refusal = proxy::refusal(upstream).await;
        if lxm == GET_POST_THREAD
            && refusal.error == "NotFound"
            && let Some(local) = thread_not_found(state, did, &rev, query).await?
        {
            return Ok(local);
        }
        return Err(refusal);
    }
    let local = {
        let (did, rev) = (did.to_owned(), rev.clone());
        state.blocking(move |state| load(state, &did, &rev)).await
    };
    // A store that cannot be read leaves the AppView's answer as it is.
    let local = match local {
        Ok(local) if local.count > 0 => local,
        Ok(_) => return proxy::answer(upstream).await,
        Err(error) => {
            tracing::warn!(%did, error = %error.message, "error in read after write munge");
            return proxy::answer(upstream).await;
        }
    };
    let content_type = header("content-type");
    let mut bytes = Vec::new();
    let mut upstream = upstream;
    while let Some(chunk) = upstream.chunk().await.map_err(XrpcError::internal)? {
        if bytes.len() + chunk.len() > RESPONSE_LIMIT {
            return Err(XrpcError::internal("the upstream answer is too large"));
        }
        bytes.extend_from_slice(&chunk);
    }
    let Ok(mut body) = serde_json::from_slice::<Value>(&bytes) else {
        // An answer that is not JSON goes on as it came.
        let mut response = Response::new(axum::body::Body::from(bytes));
        for (name, value) in [
            ("content-type", content_type),
            ("atproto-repo-rev", Some(rev)),
        ] {
            if let Some(value) = value.and_then(|value| HeaderValue::from_str(&value).ok()) {
                response.headers_mut().insert(name, value);
            }
        }
        return Ok(response);
    };
    munge(state, lxm, did, &local, &mut body).await;
    Ok(munged(&body, local.lag_ms(state.now_ms())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_account_feed_is_known_by_its_first_item() {
        let own = json!([{ "post": { "author": { "did": "did:plc:a" } } }]);
        let other = json!([{ "post": { "author": { "did": "did:plc:b" } } }]);
        let repost = json!([{
            "post": { "author": { "did": "did:plc:b" } },
            "reason": { "$type": "app.bsky.feed.defs#reasonRepost", "by": { "did": "did:plc:a" } },
        }]);
        let feed = |value: &Value| value.as_array().cloned().unwrap_or_default();
        assert!(is_own_feed(&feed(&own), "did:plc:a"));
        assert!(!is_own_feed(&feed(&other), "did:plc:a"));
        assert!(is_own_feed(&feed(&repost), "did:plc:a"));
        assert!(!is_own_feed(&[], "did:plc:a"));
    }

    #[test]
    fn the_lag_is_the_age_of_the_oldest_record() {
        let record = |indexed_at: &str| Described {
            uri: String::new(),
            cid: String::new(),
            indexed_at: indexed_at.to_owned(),
            record: Value::Null,
        };
        let local = Local {
            count: 2,
            profile: Some(record("1970-01-01T00:00:05.000Z")),
            posts: vec![record("1970-01-01T00:00:02.000Z")],
            author: None,
        };
        assert_eq!(local.lag_ms(10_000), Some(8_000));
        assert_eq!(Local::default().lag_ms(10_000), None);
    }

    #[test]
    fn a_blob_is_found_in_both_of_its_forms() {
        let new = json!({ "$type": "blob", "ref": { "$link": "bafy1" }, "mimeType": "image/png" });
        let old = json!({ "cid": "bafy2", "mimeType": "image/png" });
        assert_eq!(blob_cid(&new), Some("bafy1"));
        assert_eq!(blob_cid(&old), Some("bafy2"));
    }
}
