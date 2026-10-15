//! The operations console (`docs/sidecars.md` §1, `docs/SPEC.md` §22): a
//! sidecar that shows what the PDS does and takes the actions of an
//! operator.
//!
//! The console holds no database connection, no signing key and no secret
//! of the PDS (SPEC §22.3). An operator signs in on the pages of the PDS;
//! every read and every action is a call to the ops listener with the
//! operator's session, and the PDS writes the audit entry. The pages are
//! rendered on the server with Topcoat; its browser runtime carries the
//! live view of the overview over the browser's WebSocket to the console.

pub mod assets;
pub mod client;
pub mod config;
pub mod live;
pub mod pages;
pub mod session;
#[cfg(test)]
mod tests;
pub mod ui;

use std::{future::Future, sync::Arc};

use pds_sidecar::client::{ClientError, random_text};
use pds_sidecar::request::{cookie, field, fields, same_origin};
use serde_json::{Value, json};
use subtle::ConstantTimeEq as _;
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use topcoat::{
    Result,
    asset::{AssetBundle, RouterBuilderAssetExt as _},
    context::{Cx, app_context},
    router::{
        Body, BodyLimit, HeaderValue, Layer, LayerFuture, Next, Path, Router, StatusCode,
        error::see_other,
        header, page,
        request::{Bytes, headers, uri},
        response::{AsyncIntoResponse as _, Response},
        route,
    },
    runtime::{RouterBuilderRuntimeExt as _, connected_untracked},
    view::{View, ViewExt as _, view},
};

use crate::{
    assets::AssetsError,
    client::{Call, Client},
    config::Config,
    live::Feed,
    pages::Listing,
    session::{Session, Sessions},
    ui::Who,
};

/// The name of the session cookie.
const COOKIE: &str = "console_session";

/// The largest body of a form.
const FORM_LIMIT: usize = 16 * 1024;

/// The stream of the ops listener, as a client.
pub type Stream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Why the console cannot start.
#[derive(Debug, thiserror::Error)]
pub enum ConsoleError {
    /// The client of the PDS cannot be made.
    #[error(transparent)]
    Client(#[from] ClientError),
    /// The assets of the pages are not there.
    #[error(transparent)]
    Assets(#[from] AssetsError),
}

/// The console: its connection to the PDS, its sessions and its assets.
#[derive(Debug, Clone)]
pub struct Console {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    client: Client,
    sessions: Sessions,
    assets: AssetBundle,
    /// The host name of the PDS, for the sidebar.
    hostname: String,
    /// The content security policy of every page.
    csp: HeaderValue,
}

impl Console {
    /// A console for the configuration. It loads the assets of its pages,
    /// see [`assets`].
    pub fn new(config: Config) -> std::result::Result<Self, ConsoleError> {
        let hostname = url::Url::parse(&config.pds_public_url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
            .unwrap_or_else(|| config.pds_public_url.clone());
        let csp = HeaderValue::from_str(&csp(&config.public_url))
            .unwrap_or_else(|_| HeaderValue::from_static(FALLBACK_CSP));
        Ok(Self {
            inner: Arc::new(Inner {
                client: Client::new(config)?,
                sessions: Sessions::default(),
                assets: assets::load()?,
                hostname,
                csp,
            }),
        })
    }

    /// The configuration.
    pub fn config(&self) -> &Config {
        self.inner.client.config()
    }

    /// The host name of the PDS.
    pub fn hostname(&self) -> &str {
        &self.inner.hostname
    }

    /// Calls the ops listener for a session. When the PDS no longer takes
    /// the access token, the console refreshes the session once.
    pub(crate) async fn ops(
        &self,
        session: &Session,
        call: Call<'_>,
    ) -> std::result::Result<Value, ClientError> {
        let tokens = session.tokens();
        let client = &self.inner.client;
        let first = client.ops(&session.key, &tokens.access, call).await;
        match (first, tokens.refresh) {
            (Err(error), Some(refresh)) if error.is_unauthorized() => {
                let next = client.refresh(&session.key, &refresh).await?;
                let access = next.access.clone();
                session.set_tokens(next);
                client.ops(&session.key, &access, call).await
            }
            (first, _) => first,
        }
    }

    /// Opens the live stream of the ops listener for a session, with the
    /// same refresh as [`Self::ops`].
    pub(crate) async fn stream(
        &self,
        session: &Session,
    ) -> std::result::Result<Stream, ClientError> {
        let tokens = session.tokens();
        let client = &self.inner.client;
        let first = client
            .stream(&session.key, &tokens.access, live::STREAM_PATH)
            .await;
        match (first, tokens.refresh) {
            (Err(error), Some(refresh)) if error.is_unauthorized() => {
                let next = client.refresh(&session.key, &refresh).await?;
                let access = next.access.clone();
                session.set_tokens(next);
                client
                    .stream(&session.key, &access, live::STREAM_PATH)
                    .await
            }
            (first, _) => first,
        }
    }
}

/// The policy when the public address cannot be a header value, which
/// the configuration rules out.
const FALLBACK_CSP: &str = "default-src 'none'; style-src 'self'; script-src 'self' 'unsafe-eval'; connect-src 'self'; img-src 'self'; font-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'";

/// The content security policy of the pages. Topcoat's browser runtime
/// compiles the expressions of a page with `Function`, which needs
/// `'unsafe-eval'`; no inline script runs. The live view is the browser's
/// WebSocket to the console at its own origin.
fn csp(public_url: &str) -> String {
    let socket = public_url.replacen("http", "ws", 1);
    format!(
        "default-src 'none'; style-src 'self'; script-src 'self' 'unsafe-eval'; connect-src 'self' {socket}; img-src 'self'; font-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'"
    )
}

/// The headers of every response: the policy, no sniffing, no referrer,
/// and no cache unless the response set one (the assets do).
#[derive(Debug)]
struct SecurityHeaders {
    csp: HeaderValue,
}

impl Layer for SecurityHeaders {
    fn path(&self) -> Option<&Path> {
        None
    }

    fn handle<'a>(&'a self, cx: &'a Cx, body: Body, next: Next<'a>) -> LayerFuture<'a> {
        Box::pin(async move {
            let mut response = next.run(cx, body).await?;
            let headers = response.headers_mut();
            headers.insert(header::CONTENT_SECURITY_POLICY, self.csp.clone());
            headers.insert(
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            );
            headers.insert(
                header::REFERRER_POLICY,
                HeaderValue::from_static("no-referrer"),
            );
            headers
                .entry(header::CACHE_CONTROL)
                .or_insert(HeaderValue::from_static("no-store"));
            Ok(response)
        })
    }
}

/// The routes of the console.
pub fn router(console: Console) -> Router {
    let csp = console.inner.csp.clone();
    Router::builder()
        .layer(SecurityHeaders { csp })
        .layer(BodyLimit::max(FORM_LIMIT))
        .assets(console.inner.assets.clone())
        .route(theme_css)
        .route(shared::shared_asset)
        .page(overview)
        .page(accounts)
        .page(did::account)
        .page(audit)
        .page(domains)
        .page(roles)
        .page(sign_in)
        .route(login)
        .route(callback)
        .route(sign_out)
        .route(act)
        .route(health)
        .app_context(console)
        .runtime()
        .build()
}

/// Serves the router on a listener until `until` resolves.
pub async fn serve(
    listener: tokio::net::TcpListener,
    router: Router,
    until: impl Future<Output = ()>,
) -> std::io::Result<()> {
    topcoat::serve_until(listener, router, until).await
}

/// The console of the request.
fn console(cx: &Cx) -> &Console {
    app_context::<Console>(cx)
}

/// The session of the request's cookie.
fn session_of(cx: &Cx) -> Option<Arc<Session>> {
    cookie(headers(cx), COOKIE).and_then(|id| console(cx).inner.sessions.get(&id))
}

/// The session of the request, or the way to the sign-in page.
fn signed_in(cx: &Cx) -> Result<Arc<Session>> {
    session_of(cx).ok_or_else(|| see_other("/sign-in").into())
}

/// A parameter of the query.
fn param(cx: &Cx, name: &str) -> Option<String> {
    pds_sidecar::request::param(uri(cx).query(), name)
}

/// The status of a failed call: 403 when the PDS refuses the operator,
/// 502 when the PDS fails, 400 otherwise.
fn status_of(error: &ClientError) -> StatusCode {
    match error {
        ClientError::Transport(_) => StatusCode::BAD_GATEWAY,
        ClientError::Refused {
            status: 401 | 403, ..
        } => StatusCode::FORBIDDEN,
        ClientError::Refused { status, .. } if *status >= 500 => StatusCode::BAD_GATEWAY,
        ClientError::Refused { .. } => StatusCode::BAD_REQUEST,
    }
}

/// A redirect that the browser follows with a GET.
fn redirect(to: &str) -> Response {
    let location = HeaderValue::from_str(to).unwrap_or_else(|_| HeaderValue::from_static("/"));
    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::SEE_OTHER;
    response.headers_mut().insert(header::LOCATION, location);
    response
}

#[page("/")]
async fn overview(cx: &Cx) -> Result<impl View> {
    let session = signed_in(cx)?;
    let who = Who::of(&session);
    let done = param(cx, "done");
    let outcome = live::first(console(cx), &session).await;
    // A render over the browser's connection keeps the stream of the ops
    // listener open for the regions of the page.
    let feed = (outcome.is_ok() && connected_untracked(cx))
        .then(|| Feed::start(console(cx).clone(), Arc::clone(&session)));
    Ok(view! {
        cx =>
        match outcome {
            Ok(first) => pages::overview(who: who, first: first, feed: feed, done: done),
            Err(error) => pages::failure(who: Some(who), status: status_of(&error), message: error.to_string()),
        }
    })
}

/// The most accounts a search by handle or DID reads from the listing.
const SCAN_MOST: usize = 2000;

/// A page of accounts, or the matches of a search: an address goes to
/// `com.atproto.admin.searchAccounts` on the ops listener; a handle or a
/// DID is found in the listing, which has no search of its own.
async fn listing(
    console: &Console,
    session: &Session,
    query: Option<String>,
    cursor: Option<String>,
) -> std::result::Result<Listing, ClientError> {
    let Some(query) = query.map(|q| q.trim().to_owned()).filter(|q| !q.is_empty()) else {
        let mut path = "/xrpc/internal.ops.listAccounts?limit=50".to_owned();
        if let Some(cursor) = cursor {
            path.push_str("&cursor=");
            path.push_str(&pages::encode(&cursor));
        }
        let page = console.ops(session, Call::get(&path)).await?;
        return Ok(Listing {
            accounts: page["accounts"].as_array().cloned().unwrap_or_default(),
            cursor: page["cursor"].as_str().map(str::to_owned),
            query: None,
            note: None,
        });
    };
    if query.contains('@') {
        let path = format!(
            "/xrpc/com.atproto.admin.searchAccounts?limit=50&email={}",
            pages::encode(&query)
        );
        let found = console.ops(session, Call::get(&path)).await?;
        let listed: Vec<Value> = found["accounts"]
            .as_array()
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .map(|view| {
                let status = if view["deactivatedAt"].is_null() {
                    "active"
                } else {
                    "deactivated"
                };
                json!({
                    "did": view["did"],
                    "handle": view["handle"],
                    "status": status,
                    "createdAt": view["indexedAt"],
                    "rev": Value::Null,
                })
            })
            .collect();
        return Ok(Listing {
            accounts: listed,
            cursor: None,
            query: Some(query),
            note: None,
        });
    }
    let needle = query.to_lowercase();
    let mut matches = Vec::new();
    let mut cursor: Option<String> = None;
    let mut read = 0;
    let note = loop {
        let mut path = "/xrpc/internal.ops.listAccounts?limit=500".to_owned();
        if let Some(cursor) = &cursor {
            path.push_str("&cursor=");
            path.push_str(&pages::encode(cursor));
        }
        let page = console.ops(session, Call::get(&path)).await?;
        let listed = page["accounts"].as_array().cloned().unwrap_or_default();
        read += listed.len();
        matches.extend(listed.into_iter().filter(|entry| {
            ["handle", "did"].iter().any(|key| {
                entry[key]
                    .as_str()
                    .is_some_and(|value| value.to_lowercase().contains(&needle))
            })
        }));
        cursor = page["cursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break None;
        }
        if read >= SCAN_MOST {
            break Some(format!(
                "Only the first {read} accounts were searched. Search by email address to reach every account."
            ));
        }
    };
    Ok(Listing {
        accounts: matches,
        cursor: None,
        query: Some(query),
        note,
    })
}

#[page("/accounts")]
async fn accounts(cx: &Cx) -> Result<impl View> {
    let session = signed_in(cx)?;
    let who = Who::of(&session);
    let outcome = listing(console(cx), &session, param(cx, "q"), param(cx, "cursor")).await;
    Ok(view! {
        cx =>
        match outcome {
            Ok(listing) => pages::accounts(who: who, listing: listing),
            Err(error) => pages::failure(who: Some(who), status: status_of(&error), message: error.to_string()),
        }
    })
}

mod did {
    use super::{Call, Cx, Result, View, Who, console, pages, param, signed_in, status_of, view};

    path_param!(did);

    #[page("/accounts/{did}")]
    pub async fn account(cx: &Cx) -> Result<impl View> {
        let session = signed_in(cx)?;
        let who = Who::of(&session);
        let did: &str = path_param::<Did>(cx);
        let path = format!("/xrpc/internal.ops.getAccount?did={}", pages::encode(did));
        let outcome = console(cx).ops(&session, Call::get(&path)).await;
        let done = param(cx, "done");
        Ok(view! {
            cx =>
            match outcome {
                Ok(card) => pages::account(who: who, card: card, done: done),
                Err(error) => pages::failure(who: Some(who), status: status_of(&error), message: error.to_string()),
            }
        })
    }

    pub(super) use topcoat::router::{page, path_param};
}

#[page("/audit")]
async fn audit(cx: &Cx) -> Result<impl View> {
    let session = signed_in(cx)?;
    let who = Who::of(&session);
    let before = param(cx, "before").and_then(|value| value.parse::<u64>().ok());
    let path = before.map_or_else(
        || "/xrpc/internal.ops.listAudit?limit=50".to_owned(),
        |before| format!("/xrpc/internal.ops.listAudit?limit=50&before={before}"),
    );
    let outcome = console(cx).ops(&session, Call::get(&path)).await;
    Ok(view! {
        cx =>
        match outcome {
            Ok(page) => pages::audit(who: who, page: page),
            Err(error) => pages::failure(who: Some(who), status: status_of(&error), message: error.to_string()),
        }
    })
}

#[page("/domains")]
async fn domains(cx: &Cx) -> Result<impl View> {
    let session = signed_in(cx)?;
    let who = Who::of(&session);
    let call = Call::get("/xrpc/internal.ops.listBlockedEmailDomains");
    let outcome = console(cx).ops(&session, call).await;
    let done = param(cx, "done");
    Ok(view! {
        cx =>
        match outcome {
            Ok(list) => pages::domains(who: who, blocked: list, done: done),
            Err(error) => pages::failure(who: Some(who), status: status_of(&error), message: error.to_string()),
        }
    })
}

#[page("/roles")]
async fn roles(cx: &Cx) -> Result<impl View> {
    let session = signed_in(cx)?;
    let who = Who::of(&session);
    let outcome = console(cx)
        .ops(&session, Call::get("/xrpc/internal.ops.listRoles"))
        .await;
    Ok(view! {
        cx =>
        match outcome {
            Ok(list) => pages::roles(who: who, list: list),
            Err(error) => pages::failure(who: Some(who), status: status_of(&error), message: error.to_string()),
        }
    })
}

#[page("/sign-in")]
async fn sign_in(cx: &Cx) -> Result<impl View> {
    Ok(view! { cx => pages::sign_in() })
}

#[route(GET "/health")]
async fn health() -> Result<&'static str> {
    Ok("ok")
}

/// A response of static text or bytes with its media type and its cache
/// rule.
fn file(kind: &'static str, cache: &'static str, body: Body) -> Response {
    let mut response = Response::new(body);
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(kind));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    response
}

/// The design tokens of the LinkJar journey and the typeface, the block
/// the pages of the PDS use (`pds_oauth::theme`), so that a colour is
/// defined once for both.
#[route(GET "/theme.css")]
async fn theme_css() -> Result<Response> {
    let css = format!(
        "{}\n{}\n",
        pds_oauth::theme::TOKENS,
        pds_oauth::theme::FONT_FACE
    );
    Ok(file("text/css; charset=utf-8", "no-cache", Body::from(css)))
}

mod shared {
    use topcoat::router::{Body, error::not_found, path_param, route};

    use super::{Cx, Response, Result, file};

    path_param!(name);

    /// The typeface and the marks of `pds_oauth::theme`, under the paths
    /// the PDS serves them at, kept for a year as there.
    #[route(GET "/oauth/assets/{name}")]
    pub async fn shared_asset(cx: &Cx) -> Result<Response> {
        let name: &str = path_param::<Name>(cx);
        let (kind, bytes) = pds_oauth::theme::asset(name).ok_or_else(not_found)?;
        Ok(file(
            kind,
            "public, max-age=31536000, immutable",
            Body::from(bytes),
        ))
    }
}

/// Whether a form comes from a page of this console: the token of the
/// session, and the origin of the request when the browser names it.
fn form_is_ours(cx: &Cx, session: &Session, csrf: &str) -> bool {
    let token_ok: bool = csrf.as_bytes().ct_eq(session.csrf.as_bytes()).into();
    token_ok && same_origin(headers(cx), &console(cx).config().public_url)
}

/// What an action of a form is on the ops listener: the method, its body,
/// and the page that follows it.
pub fn plan(
    action: &str,
    fields: &[(String, String)],
) -> std::result::Result<(&'static str, Value, String), String> {
    let did = field(fields, "did");
    let account_page = pages::account_path(did);
    let repo = |applied: bool| {
        json!({
            "subject": { "$type": "com.atproto.admin.defs#repoRef", "did": did },
            "takedown": { "applied": applied, "ref": "console" },
        })
    };
    let status = "com.atproto.admin.updateSubjectStatus";
    Ok(match action {
        "takedown" if field(fields, "confirm") != did => {
            return Err("Type the DID of the account to confirm the takedown.".to_owned());
        }
        "takedown" => (status, repo(true), account_page),
        "reinstate" => (status, repo(false), account_page),
        "revoke-sessions" => (
            "internal.ops.revokeSessions",
            json!({ "did": did }),
            account_page,
        ),
        "sync" => ("internal.ops.emitSync", json!({ "did": did }), account_page),
        "disconnect" => {
            let id: u64 = field(fields, "id")
                .parse()
                .map_err(|_| "The subscriber has no such id.".to_owned())?;
            (
                "internal.ops.disconnectSubscriber",
                json!({ "id": id }),
                "/".to_owned(),
            )
        }
        "block-domain" => (
            "internal.ops.blockEmailDomain",
            json!({ "domain": field(fields, "domain") }),
            "/domains".to_owned(),
        ),
        "unblock-domain" => (
            "internal.ops.unblockEmailDomain",
            json!({ "domain": field(fields, "domain") }),
            "/domains".to_owned(),
        ),
        "checkpoint" => ("internal.ops.checkpoint", json!({}), "/".to_owned()),
        "purge" => ("internal.ops.purgeExpired", json!({}), "/".to_owned()),
        _ => return Err("The console has no such action.".to_owned()),
    })
}

/// A page that refuses a form, as a response.
async fn refused(cx: &Cx, who: Option<Who>, status: StatusCode, message: &str) -> Result<Response> {
    let message = message.to_owned();
    view! { cx => pages::failure(who: who, status: status, message: message) }
        .boxed()
        .async_into_response(cx)
        .await
}

/// Takes the action of a form.
#[route(POST "/act")]
async fn act(cx: &Cx, body: Bytes) -> Result<Response> {
    let session = signed_in(cx)?;
    let who = Who::of(&session);
    let fields = fields(&body);
    if !form_is_ours(cx, &session, field(&fields, "csrf")) {
        return refused(
            cx,
            Some(who),
            StatusCode::FORBIDDEN,
            "The form did not come from this console.",
        )
        .await;
    }
    let reason = field(&fields, "reason");
    if reason.is_empty() {
        return refused(
            cx,
            Some(who),
            StatusCode::BAD_REQUEST,
            "An action needs a reason.",
        )
        .await;
    }
    let action = field(&fields, "action");
    let (method, input, back) = match plan(action, &fields) {
        Ok(plan) => plan,
        Err(message) => return refused(cx, Some(who), StatusCode::BAD_REQUEST, &message).await,
    };
    let path = format!("/xrpc/{method}");
    match console(cx)
        .ops(&session, Call::post(&path, reason, &input))
        .await
    {
        Ok(_) => Ok(redirect(&format!("{back}?done={action}"))),
        Err(error) => {
            let message = error.to_string();
            refused(cx, Some(who), status_of(&error), &message).await
        }
    }
}

/// Starts an authorization at the PDS and sends the browser there.
#[route(GET "/login")]
async fn login(cx: &Cx) -> Result<Response> {
    let console = console(cx);
    let state = random_text::<24>();
    match console.inner.client.start(&state).await {
        Ok((address, pending)) => {
            console.inner.sessions.expect(state, pending);
            Ok(redirect(&address))
        }
        Err(error) => {
            let message = error.to_string();
            refused(cx, None, status_of(&error), &message).await
        }
    }
}

/// The browser is back from the PDS: the console takes the session, asks
/// the ops listener who the operator is, and sets the cookie.
#[route(GET "/oauth/callback")]
async fn callback(cx: &Cx) -> Result<Response> {
    let console = console(cx);
    if let Some(error) = param(cx, "error") {
        let message = format!("The server did not authorize the console: {error}.");
        return refused(cx, None, StatusCode::FORBIDDEN, &message).await;
    }
    if param(cx, "iss").as_deref() != Some(console.config().pds_public_url.as_str()) {
        return refused(
            cx,
            None,
            StatusCode::BAD_REQUEST,
            "The answer does not come from the server of this console.",
        )
        .await;
    }
    let pending = param(cx, "state").and_then(|state| console.inner.sessions.take(&state));
    let (Some(pending), Some(code)) = (pending, param(cx, "code")) else {
        return refused(
            cx,
            None,
            StatusCode::BAD_REQUEST,
            "The console did not start this sign-in, or it took too long.",
        )
        .await;
    };
    let client = &console.inner.client;
    let tokens = match client.finish(&pending, &code).await {
        Ok(tokens) => tokens,
        Err(error) => {
            let message = error.to_string();
            return refused(cx, None, status_of(&error), &message).await;
        }
    };
    let who = Call::get("/xrpc/internal.ops.getOperator");
    let role = match client.ops(&pending.key, &tokens.access, who).await {
        Ok(operator) => operator["role"].as_str().unwrap_or_default().to_owned(),
        Err(error) => {
            // An account without a role keeps no session of the console.
            client.revoke(&pending.key, &tokens.access).await;
            let message = error.to_string();
            return refused(cx, None, status_of(&error), &message).await;
        }
    };
    // The handle is for the sidebar; a card the listener does not give
    // leaves the DID there.
    let card = format!(
        "/xrpc/internal.ops.getAccount?did={}",
        pages::encode(&tokens.did)
    );
    let handle = client
        .ops(&pending.key, &tokens.access, Call::get(&card))
        .await
        .ok()
        .and_then(|card| card["handle"].as_str().map(str::to_owned));
    let session = console
        .inner
        .sessions
        .start(pending.key, tokens, role, handle);
    let cookie = format!(
        "{COOKIE}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
        session.id,
        session::SESSION_LIFE.as_secs()
    );
    let mut response = redirect("/");
    if let Ok(cookie) = HeaderValue::from_str(&cookie) {
        response.headers_mut().insert(header::SET_COOKIE, cookie);
    }
    Ok(response)
}

/// Ends the session at the console and at the PDS.
#[route(POST "/sign-out")]
async fn sign_out(cx: &Cx, body: Bytes) -> Result<Response> {
    let session = signed_in(cx)?;
    let fields = fields(&body);
    if !form_is_ours(cx, &session, field(&fields, "csrf")) {
        return refused(
            cx,
            Some(Who::of(&session)),
            StatusCode::FORBIDDEN,
            "The form did not come from this console.",
        )
        .await;
    }
    let console = console(cx);
    console.inner.sessions.end(&session.id);
    let tokens = session.tokens();
    console
        .inner
        .client
        .revoke(&session.key, &tokens.access)
        .await;
    let mut response = redirect("/sign-in");
    let gone = format!("{COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0");
    if let Ok(gone) = HeaderValue::from_str(&gone) {
        response.headers_mut().insert(header::SET_COOKIE, gone);
    }
    Ok(response)
}

#[cfg(test)]
mod plan_tests {
    use super::*;

    fn fields(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn an_action_of_a_form_is_a_method_of_the_ops_listener() {
        let did = "did:plc:x";
        let (method, input, back) = plan("revoke-sessions", &fields(&[("did", did)])).unwrap();
        assert_eq!(method, "internal.ops.revokeSessions");
        assert_eq!(input, json!({"did": did}));
        assert_eq!(back, "/accounts/did%3Aplc%3Ax");

        // A takedown needs the DID typed again.
        assert!(plan("takedown", &fields(&[("did", did)])).is_err());
        assert!(
            plan(
                "takedown",
                &fields(&[("did", did), ("confirm", "did:plc:y")])
            )
            .is_err()
        );
        let (method, input, _) =
            plan("takedown", &fields(&[("did", did), ("confirm", did)])).unwrap();
        assert_eq!(method, "com.atproto.admin.updateSubjectStatus");
        assert_eq!(input["takedown"]["applied"], true);
        assert_eq!(input["subject"]["did"], did);

        let (_, input, back) = plan("disconnect", &fields(&[("id", "7")])).unwrap();
        assert_eq!((input, back.as_str()), (json!({"id": 7}), "/"));
        assert!(plan("disconnect", &fields(&[("id", "seven")])).is_err());
        assert!(plan("drop-database", &[]).is_err());
        // Every action with a toast has a plan.
        for (name, _) in pages::ACTIONS {
            let all = fields(&[
                ("did", did),
                ("confirm", did),
                ("id", "1"),
                ("domain", "a.b"),
            ]);
            assert!(plan(name, &all).is_ok(), "{name}");
        }
    }

    #[test]
    fn the_callback_route_is_the_redirect_uri_of_the_client() {
        // The route attribute takes a literal; the client builds the
        // redirect URI from the constant.
        assert_eq!(crate::config::CALLBACK_PATH, "/oauth/callback");
    }

    #[test]
    fn the_policy_allows_the_runtime_and_the_socket_of_the_console_only() {
        let policy = csp("http://127.0.0.1:2585");
        assert!(policy.starts_with("default-src 'none'; "));
        assert!(policy.contains("script-src 'self' 'unsafe-eval'; "));
        assert!(policy.contains("connect-src 'self' ws://127.0.0.1:2585; "));
        assert!(policy.ends_with("frame-ancestors 'none'; base-uri 'none'"));
        assert!(!policy.contains("unsafe-inline"));
    }
}
