//! The pages, rendered against a stand-in for the PDS: a router that
//! answers the reads of the ops listener with sample values, some of them
//! hostile, and serves the stream as a WebSocket. The console gets a
//! session without the sign-in, which `crates/linkjar-pds/tests/oauth.rs`
//! covers against the real server.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use futures_util::{SinkExt as _, StreamExt as _};
use pds_sidecar::client::{Tokens, new_key};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest as _};
use topcoat::{
    Result,
    context::Cx,
    router::{
        Router,
        content::{
            Json,
            websocket::{Message as ServerMessage, WebSocketUpgrade},
        },
        response::Response,
        route,
    },
};

use crate::{Console, config::Config, session::Session};

/// A user agent that tries to run a script and to leave an attribute, and
/// a host name that tries to leave its attribute.
const HOSTILE_AGENT: &str = "<script>alert(1)</script>\"><b>";
const HOSTILE_HOST: &str = "pds\"><b>";

/// Checks that the hostile values of the stand-in are text in `markup`:
/// no attribute is left, and the script is a script nowhere. Topcoat
/// escapes `<` in text and `"` in an attribute, which is what HTML needs,
/// so the raw script may appear inside a quoted `title` and nowhere else.
fn hostile_values_are_text(markup: &str) {
    assert!(!markup.contains("\"><b>"), "{markup}");
    let raw = markup.matches("<script>alert").count();
    let quoted = markup.matches("title=\"<script>alert").count();
    assert_eq!(raw, quoted, "{markup}");
    assert!(!markup.contains("pds\"><b>"), "{markup}");
}

/// Whether the stand-in shows the console off: plausible values instead
/// of hostile ones, and an endless stream. See [`demo`].
static DEMO: AtomicBool = AtomicBool::new(false);

fn overview(accounts: u64) -> Value {
    if DEMO.load(Ordering::Relaxed) {
        return json!({
            "version": "0.0.1", "profile": "linkjar", "hostname": "pds.linkjar.social",
            "time": "2026-10-16T09:12:04.000Z", "accounts": accounts,
            "sequencer": { "head": 48_211 }, "subscribers": 2, "commitsInFlight": 1,
            "noticeOutboxDepth": 0,
            "audit": { "entries": 312, "unsigned": 4, "checkpointAgeSeconds": 1_830 },
        });
    }
    json!({
        "version": "0.0.1", "profile": "linkjar", "hostname": HOSTILE_HOST,
        "time": "2026-10-15T12:00:00.000Z", "accounts": accounts,
        "sequencer": { "head": 1234 }, "subscribers": 1, "commitsInFlight": 0,
        "noticeOutboxDepth": 2,
        "audit": { "entries": 9, "unsigned": 1, "checkpointAgeSeconds": 42 },
    })
}

fn subscribers() -> Value {
    if DEMO.load(Ordering::Relaxed) {
        let now = chrono::Utc::now();
        let ago = |seconds: i64| {
            (now - chrono::Duration::seconds(seconds))
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        };
        return json!([
            {
                "id": 3, "userAgent": "relay1.us-east.bsky.network", "remote": "34.238.9.71:51322",
                "cursor": 48_209, "lag": 2, "frames": 10_430, "bytes": 51_204_112,
                "lastFrameAt": ago(1), "connectedAt": ago(7_300),
            },
            {
                "id": 9, "userAgent": "indexer/0.4 (linkjar)", "remote": "100.64.0.12:40022",
                "cursor": 46_880, "lag": 1_331, "frames": 2_216, "bytes": 9_903_120,
                "lastFrameAt": ago(45), "connectedAt": ago(640),
            },
        ]);
    }
    json!([{
        "id": 7, "userAgent": HOSTILE_AGENT, "remote": "10.0.0.1:4000",
        "cursor": 1200, "lag": 2000, "frames": 10, "bytes": 2048,
        "lastFrameAt": "2026-10-15T11:59:00.000Z", "connectedAt": "2026-10-15T11:00:00.000Z",
    }])
}

#[route(GET "/xrpc/internal.ops.getOverview")]
async fn get_overview() -> Result<Json<Value>> {
    Ok(Json(overview(3)))
}

#[route(GET "/xrpc/internal.ops.listSubscribers")]
async fn list_subscribers() -> Result<Json<Value>> {
    Ok(Json(json!({ "subscribers": subscribers() })))
}

/// The accounts of the demo.
const DEMO_ACCOUNTS: [(&str, &str, &str, &str); 6] = [
    (
        "did:plc:4a6k2jfxw3rqpl4dmn7yt5ve",
        "avery.linkjar.social",
        "2026-10-14T18:22:10.000Z",
        "active",
    ),
    (
        "did:plc:7mzq2r5kd3xwp4vn8ahysc2t",
        "maren.linkjar.social",
        "2026-10-13T09:41:52.000Z",
        "active",
    ),
    (
        "did:plc:kq3x8v2nrw5tdy7mpj4zb6hc",
        "linkjar.social",
        "2026-10-11T12:00:00.000Z",
        "active",
    ),
    (
        "did:plc:b2n7r4xkq9m3wvt6ydz8pl5s",
        "spam-bot-1134.linkjar.social",
        "2026-10-10T03:17:44.000Z",
        "takendown",
    ),
    (
        "did:plc:x9w3mn5qk2rt7vpd4jz6hy8b",
        "theo.linkjar.social",
        "2026-10-09T15:05:31.000Z",
        "active",
    ),
    (
        "did:plc:p5t2kz8vmx4nq7rw3jd9yh6c",
        "ines.linkjar.social",
        "2026-10-08T20:48:03.000Z",
        "deactivated",
    ),
];

#[route(GET "/xrpc/internal.ops.listAccounts")]
async fn list_accounts() -> Result<Json<Value>> {
    if DEMO.load(Ordering::Relaxed) {
        let accounts: Vec<Value> = DEMO_ACCOUNTS
            .iter()
            .enumerate()
            .map(|(index, (did, handle, created, status))| {
                json!({
                    "did": did, "handle": handle, "createdAt": created, "status": status,
                    "head": format!("bafyreigh2akiscaildc{index}"), "rev": format!("3m2x{}", 90 - index),
                })
            })
            .collect();
        return Ok(Json(json!({ "accounts": accounts, "cursor": null })));
    }
    Ok(Json(json!({
        "accounts": [
            { "did": "did:plc:alice", "handle": "alice.test", "createdAt": "2026-10-01T00:00:00.000Z", "status": "active", "head": "bafy1", "rev": "3l" },
            { "did": "did:plc:dana", "handle": HOSTILE_AGENT, "createdAt": "2026-10-02T00:00:00.000Z", "status": "takendown", "head": "bafy2", "rev": "3m" },
        ],
        "cursor": "2026-10-02::did:plc:dana",
    })))
}

#[route(GET "/xrpc/internal.ops.getAccount")]
async fn get_account(cx: &Cx) -> Result<Json<Value>> {
    let did = crate::param(cx, "did").unwrap_or_default();
    let down = did.ends_with("down");
    if DEMO.load(Ordering::Relaxed) {
        let (_, handle, created, status) = DEMO_ACCOUNTS
            .iter()
            .find(|(known, ..)| *known == did)
            .copied()
            .unwrap_or((
                "",
                "someone.linkjar.social",
                "2026-10-01T00:00:00.000Z",
                "active",
            ));
        let down = status == "takendown";
        return Ok(Json(json!({
            "did": did, "handle": handle, "email": "avery@example.com",
            "emailConfirmedAt": "2026-10-14T18:25:40.000Z", "createdAt": created,
            "status": status,
            "takedownRef": if down { Value::from("console") } else { Value::Null },
            "deactivatedAt": null, "role": null, "head": "bafyreigh2akiscaildcqmrxqgv6o3nxjnhnd4qxqaxkqefh7y6w7v3ylwfe", "rev": "3m2x7qk2lrs2b",
            "repo": { "records": 1_482, "blobs": 96, "referencedBlobs": 94, "blocks": 3_907 },
            "legacySessions": [{ "appPassword": "LinkJar for iPhone", "expiresAt": "2026-12-14T18:30:00.000Z" }],
            "oauthSessions": [
                { "clientId": "https://app.linkjar.io/oauth-client-metadata.json", "createdAt": "2026-10-14T18:30:00.000Z", "updatedAt": "2026-10-16T08:51:12.000Z", "expiresAt": "2026-11-13T18:30:00.000Z", "scope": "atproto transition:generic" },
            ],
            "devices": [{ "userAgent": "Safari 26 on macOS", "ipAddress": "84.112.7.40", "lastSeenAt": "2026-10-16T08:51:12.000Z", "signedInAt": "2026-10-14T18:30:00.000Z" }],
        })));
    }
    Ok(Json(json!({
        "did": did, "handle": "someone.test", "email": "someone@example.com",
        "emailConfirmedAt": "2026-10-01T00:00:00.000Z", "createdAt": "2026-09-30T00:00:00.000Z",
        "status": if down { "takendown" } else { "active" },
        "takedownRef": if down { Value::from("console") } else { Value::Null },
        "deactivatedAt": null, "role": null, "head": "bafyhead", "rev": "3n",
        "repo": { "records": 12, "blobs": 3, "referencedBlobs": 3, "blocks": 40 },
        "legacySessions": [{ "appPassword": "phone", "expiresAt": "2026-11-01T00:00:00.000Z" }],
        "oauthSessions": [{ "clientId": HOSTILE_AGENT, "createdAt": "2026-10-01T00:00:00.000Z", "updatedAt": "2026-10-14T00:00:00.000Z", "expiresAt": "2026-10-20T00:00:00.000Z", "scope": "atproto" }],
        "devices": [],
    })))
}

#[route(GET "/xrpc/internal.ops.listAudit")]
async fn list_audit() -> Result<Json<Value>> {
    if DEMO.load(Ordering::Relaxed) {
        let op = "did:plc:kq3x8v2nrw5tdy7mpj4zb6hc";
        let entries = [
            (
                312,
                "2026-10-16T08:58:30.000Z",
                op,
                "internal.ops.disconnectSubscriber",
                "subscriber 9",
                "ok",
                "Indexer stuck behind after the restart",
                false,
            ),
            (
                311,
                "2026-10-16T08:40:02.000Z",
                op,
                "com.atproto.admin.updateSubjectStatus",
                "did:plc:b2n7r4xkq9m3wvt6ydz8pl5s",
                "ok",
                "Spam links across 40 posts; reported twice",
                false,
            ),
            (
                310,
                "2026-10-16T07:12:45.000Z",
                "admin",
                "internal.ops.checkpoint",
                "audit log",
                "ok",
                "Nightly checkpoint",
                true,
            ),
            (
                309,
                "2026-10-15T22:03:11.000Z",
                op,
                "internal.ops.blockEmailDomain",
                "mailinator.example",
                "ok",
                "Disposable addresses in the sign-up wave",
                true,
            ),
            (
                308,
                "2026-10-15T21:59:40.000Z",
                op,
                "internal.ops.revokeSessions",
                "did:plc:x9w3mn5qk2rt7vpd4jz6hy8b",
                "ok",
                "Lost phone, asked by the member",
                true,
            ),
            (
                307,
                "2026-10-15T18:30:12.000Z",
                "did:plc:7mzq2r5kd3xwp4vn8ahysc2t",
                "internal.ops.emitSync",
                "did:plc:4a6k2jfxw3rqpl4dmn7yt5ve",
                "forbidden",
                "Relay shows an old head",
                true,
            ),
            (
                306,
                "2026-10-15T11:04:58.000Z",
                op,
                "internal.ops.purgeExpired",
                "server",
                "ok",
                "Weekly cleanup",
                true,
            ),
        ];
        let entries: Vec<Value> = entries
            .iter()
            .map(|(seq, time, actor, action, subject, outcome, reason, signed)| {
                json!({ "seq": seq, "time": time, "actor": actor, "action": action, "subject": subject, "outcome": outcome, "reason": reason, "signed": signed })
            })
            .collect();
        return Ok(Json(json!({ "entries": entries, "before": 305 })));
    }
    Ok(Json(json!({
        "entries": [{ "seq": 9, "time": "2026-10-15T11:58:00.000Z", "actor": "did:plc:op", "action": "com.atproto.admin.updateSubjectStatus", "subject": "did:plc:dana", "outcome": "ok", "reason": HOSTILE_AGENT, "signed": false }],
        "before": 8,
    })))
}

#[route(GET "/xrpc/internal.ops.listRoles")]
async fn list_roles() -> Result<Json<Value>> {
    if DEMO.load(Ordering::Relaxed) {
        return Ok(Json(json!({ "roles": [
            { "did": "did:plc:kq3x8v2nrw5tdy7mpj4zb6hc", "role": "operator", "grantedBy": "admin", "grantedAt": "2026-10-11T12:05:00.000Z" },
            { "did": "did:plc:7mzq2r5kd3xwp4vn8ahysc2t", "role": "moderator", "grantedBy": "admin", "grantedAt": "2026-10-13T10:00:00.000Z" },
        ] })));
    }
    Ok(Json(
        json!({ "roles": [{ "did": "did:plc:op", "role": "operator", "grantedBy": "admin", "grantedAt": "2026-10-01T00:00:00.000Z" }] }),
    ))
}

#[route(GET "/xrpc/internal.ops.listBlockedEmailDomains")]
async fn list_domains() -> Result<Json<Value>> {
    if DEMO.load(Ordering::Relaxed) {
        return Ok(Json(json!({ "domains": [
            { "domain": "mailinator.example", "reason": "Disposable addresses in the sign-up wave", "createdBy": "did:plc:kq3x8v2nrw5tdy7mpj4zb6hc", "createdAt": "2026-10-15T22:03:11.000Z" },
            { "domain": "spam.example", "reason": "Every account from here was taken down", "createdBy": "admin", "createdAt": "2026-10-02T09:12:00.000Z" },
        ] })));
    }
    Ok(Json(
        json!({ "domains": [{ "domain": "spam.example", "reason": HOSTILE_AGENT, "createdBy": "did:plc:op", "createdAt": "2026-10-10T00:00:00.000Z" }] }),
    ))
}

#[route(POST "/xrpc/{*method}")]
async fn act() -> Result<Json<Value>> {
    Ok(Json(json!({})))
}

/// The stream: two frames, then the end. In the demo, a frame every two
/// seconds, like the server's.
#[route(GET "/xrpc/internal.ops.subscribeState")]
async fn stream(cx: &Cx, upgrade: WebSocketUpgrade) -> Result<Response> {
    // The console proves the handshake like any call of the listener.
    assert!(
        topcoat::router::request::headers(cx).contains_key("dpop"),
        "the handshake carries no proof"
    );
    upgrade.on_upgrade(|mut socket| async move {
        if DEMO.load(Ordering::Relaxed) {
            let mut head = 48_211_u64;
            loop {
                head += 3;
                let mut state = overview(1_284);
                state["sequencer"]["head"] = json!(head);
                state["time"] =
                    json!(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
                let frame = json!({ "overview": state, "subscribers": subscribers() });
                if socket
                    .send(ServerMessage::text(frame.to_string()))
                    .await
                    .is_err()
                {
                    return;
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
        for accounts in [4, 5] {
            let frame = json!({ "overview": overview(accounts), "subscribers": subscribers() });
            if socket
                .send(ServerMessage::text(frame.to_string()))
                .await
                .is_err()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let _ = socket.close().await;
    })
}

fn stand_in() -> Router {
    Router::builder()
        .route(get_overview)
        .route(list_subscribers)
        .route(list_accounts)
        .route(get_account)
        .route(list_audit)
        .route(list_roles)
        .route(list_domains)
        .route(act)
        .route(stream)
        .build()
}

/// The console in front of the stand-in, with a session of each role.
struct Harness {
    origin: String,
    http: reqwest::Client,
    operator: Arc<Session>,
    moderator: Arc<Session>,
}

impl Harness {
    async fn start() -> Self {
        let pds = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let pds_origin = format!("http://{}", pds.local_addr().unwrap());
        tokio::spawn(topcoat::serve_until(
            pds,
            stand_in(),
            std::future::pending(),
        ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let vars = [
            ("CONSOLE_PUBLIC_URL", origin.as_str()),
            ("CONSOLE_PDS_PUBLIC_URL", "https://pds.test"),
            ("CONSOLE_PDS_INTERNAL_URL", pds_origin.as_str()),
            ("CONSOLE_OPS_URL", pds_origin.as_str()),
        ]
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
        let console = Console::new(Config::from_vars(&vars).unwrap()).unwrap();
        let session = |did: &str, role: &str| {
            let tokens = Tokens {
                access: "access".to_owned(),
                refresh: None,
                did: did.to_owned(),
            };
            console.inner.sessions.start(
                new_key(),
                tokens,
                role.to_owned(),
                Some("op.test".to_owned()),
            )
        };
        let operator = session("did:plc:op", "operator");
        let moderator = session("did:plc:mod", "moderator");
        tokio::spawn(crate::serve(
            listener,
            crate::router(console),
            std::future::pending(),
        ));
        Self {
            origin,
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            operator,
            moderator,
        }
    }

    async fn get(
        &self,
        session: &Session,
        path: &str,
    ) -> (u16, reqwest::header::HeaderMap, String) {
        let response = self
            .http
            .get(format!("{}{path}", self.origin))
            .header("cookie", format!("{}={}", crate::COOKIE, session.id))
            .send()
            .await
            .unwrap();
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        (status, headers, response.text().await.unwrap())
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // one operator and one moderator over every page
async fn every_value_of_the_server_is_escaped_and_the_pages_have_their_parts() {
    let console = Harness::start().await;

    let (status, headers, home) = console.get(&console.operator, "/?done=disconnect").await;
    assert_eq!(status, 200, "{home}");
    let policy = headers["content-security-policy"].to_str().unwrap();
    assert!(policy.starts_with("default-src 'none'; style-src 'self'; script-src 'self' 'unsafe-eval'; connect-src 'self' ws://127.0.0.1:"), "{policy}");
    assert!(policy.ends_with("; img-src 'self'; font-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'"));
    assert_eq!(headers["x-content-type-options"], "nosniff");
    assert_eq!(headers["referrer-policy"], "no-referrer");
    assert_eq!(headers["cache-control"], "no-store");
    // The hostile values are text, not markup.
    hostile_values_are_text(&home);
    assert!(home.contains("&lt;script&gt;alert(1)&lt;/script&gt;\"&gt;&lt;b&gt;"));
    assert!(home.contains("pds\"&gt;&lt;b&gt;"), "{home}");
    assert!(home.contains("title=\"<script>alert(1)</script>&quot;><b>\""));
    // The parts: the sidebar with the current section, the tiles, the
    // diagram with the tone of the lag, the row with its action, the toast.
    assert!(
        home.contains(r#"href="/" aria-current="page" data-slot="side-item""#),
        "{home}"
    );
    assert!(home.contains(
        r#"<span data-slot="tile-label">Accounts</span><strong data-slot="tile-value">3"#
    ));
    assert!(home.contains(r#"<span data-slot="tile-label">Checkpoint age</span><strong data-slot="tile-value">42<small data-slot="tile-unit">s</small>"#));
    assert!(home.contains(r#"data-lag="behind""#));
    // The diagram draws with the shared tokens and nothing of its own.
    assert!(home.contains(r#"fill="var(--lj-fill-2)""#), "{home}");
    assert!(!home.contains("var(--color-"), "{home}");
    assert!(home.contains(r#"data-subscriber="7""#));
    assert!(home.contains(r#"commandfor="disconnect-7""#));
    assert!(home.contains(r#"<dialog id="disconnect-7""#));
    assert!(home.contains("The subscriber is disconnected."));
    assert!(home.contains(r#"data-slot="live" data-state="snapshot""#));
    assert!(home.contains("::topcoat::connect"), "{home}");
    assert!(home.contains("<small>did:plc:op</small>"));
    assert!(home.contains(r#"value="checkpoint""#) && home.contains(r#"value="purge""#));
    // The tokens and the typeface are the PDS's, served by the console.
    assert!(
        home.contains(r#"<link rel="stylesheet" href="/theme.css">"#),
        "{home}"
    );
    assert!(home.contains(r#"<img src="/oauth/assets/linkjar-mark.png""#));
    let (status, headers, theme) = console.get(&console.operator, "/theme.css").await;
    assert_eq!(status, 200);
    assert_eq!(headers["content-type"], "text/css; charset=utf-8");
    assert!(theme.starts_with(pds_oauth::theme::TOKENS), "{theme}");
    assert!(theme.contains("@font-face{font-family:Geist;"));
    let (status, headers, _) = console
        .get(&console.operator, pds_oauth::theme::FONT_PATH)
        .await;
    assert_eq!(status, 200);
    assert_eq!(headers["content-type"], "font/woff2");
    assert_eq!(
        headers["cache-control"],
        "public, max-age=31536000, immutable"
    );
    assert_eq!(
        console
            .get(&console.operator, "/oauth/assets/missing.woff2")
            .await
            .0,
        404
    );
    // No inline script and no style attribute: the policy allows neither.
    // The one script element loads the runtime; the other `<script` is the
    // text of the hostile user agent inside its quoted attribute.
    assert_eq!(home.matches("<script").count(), 2, "{home}");
    assert_eq!(
        home.matches("<script type=\"module\" src=\"/_topcoat/assets/topcoat-")
            .count(),
        1
    );
    assert!(!home.contains(" style=\""), "{home}");
    // A toast is one of the known texts.
    let (_, _, forged) = console.get(&console.operator, "/?done=%3Cb%3E").await;
    assert!(!forged.contains(r#"data-slot="toast""#));

    // A moderator sees no housekeeping and no disconnect.
    let (_, _, home) = console.get(&console.moderator, "/").await;
    assert!(!home.contains(r#"value="checkpoint""#));
    assert!(!home.contains(r#"commandfor="disconnect-7""#));
    assert!(home.contains(r#"data-tone="accent">moderator</span>"#));

    // The accounts, with a search in the listing.
    let (_, _, list) = console.get(&console.operator, "/accounts").await;
    assert!(
        list.contains(r#"href="/accounts/did%3Aplc%3Aalice""#),
        "{list}"
    );
    assert!(list.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    assert!(list.contains(r#"href="/accounts?cursor=2026-10-02%3A%3Adid%3Aplc%3Adana""#));
    let (_, _, found) = console.get(&console.operator, "/accounts?q=ALICE").await;
    assert!(
        found.contains("did:plc:alice") && !found.contains("did:plc:dana"),
        "{found}"
    );
    assert!(found.contains(r#"value="ALICE""#));
    assert!(!found.contains("Next page"));
    let (status, _, refused) = console.get(&console.operator, "/accounts?q=a@b").await;
    // The stand-in has no search by email: the refusal of the listener is
    // shown as a failure, with the status of a bad request.
    assert_eq!(status, 400, "{refused}");
    assert!(refused.contains(r#"role="alert""#));

    // One account: the moderator's actions, the operator's, and the
    // takedown with the DID to type.
    let (_, _, card) = console.get(&console.moderator, "/accounts/did:plc:x").await;
    assert!(card.contains(r#"value="revoke-sessions""#));
    assert!(card.contains(r#"value="takedown""#));
    assert!(card.contains(r#"pattern="did:plc:x""#));
    assert!(card.contains(r"<code>did:plc:x</code> to confirm"));
    assert!(!card.contains(r#"value="sync""#));
    assert!(card.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    let (_, _, card) = console.get(&console.operator, "/accounts/did:plc:x").await;
    assert!(card.contains(r#"value="sync""#));
    let (_, _, down) = console
        .get(&console.operator, "/accounts/did:plc:down")
        .await;
    assert!(down.contains(r#"value="reinstate""#));
    assert!(!down.contains(r#"value="takedown""#));
    assert!(down.contains(r#"data-tone="warn">takendown</span>"#));

    // The audit log, the domains and the roles.
    let (_, _, log) = console.get(&console.operator, "/audit").await;
    assert!(log.contains("com.atproto.admin.updateSubjectStatus"));
    assert!(log.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    assert!(log.contains(r#"href="/audit?before=8""#));
    assert!(log.contains(
        r#"<time datetime="2026-10-15T11:58:00.000Z" title="2026-10-15T11:58:00.000Z">"#
    ));
    let (_, _, domains) = console
        .get(&console.operator, "/domains?done=block-domain")
        .await;
    assert!(domains.contains("<code>spam.example</code>"));
    assert!(domains.contains(r#"commandfor="unblock-0""#));
    assert!(domains.contains(r#"value="block-domain""#));
    assert!(domains.contains("The email domain is blocked."));
    let (_, _, domains) = console.get(&console.moderator, "/domains").await;
    assert!(!domains.contains(r#"value="block-domain""#));
    let (_, _, roles) = console.get(&console.operator, "/roles").await;
    assert!(roles.contains(r#"href="/accounts/did%3Aplc%3Aop""#));
    assert!(roles.contains(r#"data-tone="accent">operator</span>"#));

    // Without a session: the sign-in page, and the way to it.
    let response = console
        .http
        .get(format!("{}/", console.origin))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 303);
    assert_eq!(response.headers()["location"], "/sign-in");
    let sign_in = console
        .http
        .get(format!("{}/sign-in", console.origin))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(sign_in.contains(r#"href="/login""#));
    assert!(sign_in.contains("<code>https://pds.test</code>"));
    assert!(!sign_in.contains("data-slot=\"sidebar\""));
}

/// The frames of a run on the runtime socket of the console: the
/// snapshot of the page, then the swaps of its live regions.
async fn frames(origin: &str, session: &Session, most: usize) -> Vec<Value> {
    let address = origin.replacen("http", "ws", 1);
    let mut request = format!("{address}/").into_client_request().unwrap();
    let headers = request.headers_mut();
    headers.insert("sec-websocket-protocol", "topcoat-runtime".parse().unwrap());
    headers.insert(
        "cookie",
        format!("{}={}", crate::COOKIE, session.id).parse().unwrap(),
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    let run = json!({ "run": 1, "method": "GET", "path": "/", "headers": {}, "body": "" });
    socket
        .send(Message::Text(run.to_string().into()))
        .await
        .unwrap();
    let mut frames = Vec::new();
    let read = async {
        while let Some(Ok(message)) = socket.next().await {
            if let Message::Text(text) = message {
                let envelope: Value = serde_json::from_str(&text).unwrap();
                frames.push(envelope["frame"].clone());
                if frames.len() >= most {
                    break;
                }
            }
        }
    };
    let _ = tokio::time::timeout(Duration::from_secs(10), read).await;
    frames
}

#[tokio::test]
async fn the_live_view_follows_the_stream_of_the_server() {
    let console = Harness::start().await;
    let frames = frames(&console.origin, &console.operator, 8).await;
    let snapshot = &frames[0];
    assert_eq!(snapshot["t"], "snapshot", "{frames:?}");
    let html = snapshot["html"].as_str().unwrap();
    assert!(html.contains(r#"data-state="snapshot""#), "{html}");
    let swaps: Vec<&str> = frames[1..]
        .iter()
        .filter(|frame| frame["t"] == "swap")
        .filter_map(|frame| frame["html"].as_str())
        .collect();
    assert!(!swaps.is_empty(), "{frames:?}");
    // The numbers follow the frames of the stream, with their change, and
    // the end of the stream is shown.
    assert!(
        swaps.iter().any(|html| html.contains(r#"data-state="streaming""#)
            && html.contains(r#"<strong data-slot="tile-value">4<em data-slot="tile-delta" data-sign="up">+1</em>"#)),
        "{swaps:?}"
    );
    assert!(
        swaps
            .iter()
            .any(|html| html.contains(r#"data-slot="tile-value">5<em"#)),
        "{swaps:?}"
    );
    assert!(
        swaps
            .iter()
            .any(|html| html.contains(r#"data-state="ended""#)
                && html.contains("The stream of the server ended.")),
        "{swaps:?}"
    );
    // The hostile values stay text in the swaps too.
    for html in &swaps {
        hostile_values_are_text(html);
    }
}

/// The console in front of the stand-in with plausible values, for a look
/// in a browser: `cargo test -p pds-console --lib tests::demo -- --ignored
/// --nocapture` prints the address and the cookie of an operator, then
/// serves for twenty minutes.
#[tokio::test]
#[ignore = "serves the console for a person to look at"]
#[allow(clippy::print_stdout)]
async fn demo() {
    DEMO.store(true, Ordering::Relaxed);
    let console = Harness::start().await;
    println!(
        "DEMO {} {}={}",
        console.origin,
        crate::COOKIE,
        console.operator.id
    );
    tokio::time::sleep(Duration::from_mins(20)).await;
}
