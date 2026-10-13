//! The operations console (`docs/sidecars.md` §1, `docs/SPEC.md` §22): a
//! sidecar that shows what the PDS does and takes the actions of an
//! operator.
//!
//! The console holds no database connection, no signing key and no secret
//! of the PDS (SPEC §22.3). An operator signs in on the pages of the PDS;
//! every read and every action is a call to the ops listener with the
//! operator's session, and the PDS writes the audit entry.

pub mod client;
pub mod config;
pub mod pages;
pub mod session;

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde_json::{Value, json};
use subtle::ConstantTimeEq as _;

use crate::client::{Call, Client, ClientError, random_text};
use crate::config::{CALLBACK_PATH, Config};
use crate::session::{Session, Sessions};

/// The name of the session cookie.
const COOKIE: &str = "console_session";

/// The largest body of a form.
const FORM_LIMIT: usize = 16 * 1024;

const CSP: &str = "default-src 'none'; style-src 'self'; script-src 'self'; connect-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'";

/// The console: its connection to the PDS and its sessions.
#[derive(Debug, Clone)]
pub struct Console {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    client: Client,
    sessions: Sessions,
}

impl Console {
    /// A console for the configuration.
    pub fn new(config: Config) -> Result<Self, ClientError> {
        Ok(Self {
            inner: Arc::new(Inner {
                client: Client::new(config)?,
                sessions: Sessions::default(),
            }),
        })
    }

    fn config(&self) -> &Config {
        self.inner.client.config()
    }

    /// Calls the ops listener for a session. When the PDS no longer takes
    /// the access token, the console refreshes the session once.
    async fn ops(&self, session: &Session, call: Call<'_>) -> Result<Value, ClientError> {
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
}

/// The routes of the console.
pub fn router(console: Console) -> axum::Router {
    axum::Router::new()
        .route("/", get(overview))
        .route("/live.json", get(live))
        .route("/accounts", get(accounts))
        .route("/accounts/{did}", get(account))
        .route("/audit", get(audit))
        .route("/domains", get(domains))
        .route("/roles", get(roles))
        .route("/act", post(act))
        .route("/sign-in", get(sign_in))
        .route("/login", get(login))
        .route(CALLBACK_PATH, get(callback))
        .route("/sign-out", post(sign_out))
        .route("/health", get(|| async { "ok" }))
        .route(
            "/console.css",
            get(|| async { asset("text/css; charset=utf-8", include_str!("console.css")) }),
        )
        .route(
            "/console.js",
            get(|| async { asset("text/javascript; charset=utf-8", include_str!("console.js")) }),
        )
        .with_state(console)
}

fn asset(kind: &'static str, text: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, kind),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        text,
    )
        .into_response()
}

fn html(status: StatusCode, page: String) -> Response {
    (
        status,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CONTENT_SECURITY_POLICY, CSP),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::REFERRER_POLICY, "no-referrer"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        page,
    )
        .into_response()
}

fn redirect(to: &str) -> Response {
    let location = HeaderValue::from_str(to).unwrap_or_else(|_| HeaderValue::from_static("/"));
    (
        StatusCode::SEE_OTHER,
        [
            (header::LOCATION, location),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
    )
        .into_response()
}

/// The session of the request's cookie.
fn session_of(console: &Console, headers: &HeaderMap) -> Option<Arc<Session>> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == COOKIE)
        .and_then(|(_, id)| console.inner.sessions.get(id))
}

/// A parameter of the query.
fn param(query: Option<&str>, name: &str) -> Option<String> {
    url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
        .find(|(key, value)| key == name && !value.is_empty())
        .map(|(_, value)| value.into_owned())
}

/// The page of a failed call: 403 when the PDS refuses the operator, 502
/// when the PDS fails, 400 otherwise.
fn failed(session: Option<&Session>, error: &ClientError) -> Response {
    let status = match error {
        ClientError::Transport(_) => StatusCode::BAD_GATEWAY,
        ClientError::Refused {
            status: 401 | 403, ..
        } => StatusCode::FORBIDDEN,
        ClientError::Refused { status, .. } if *status >= 500 => StatusCode::BAD_GATEWAY,
        ClientError::Refused { .. } => StatusCode::BAD_REQUEST,
    };
    html(status, pages::failure(session, &error.to_string()))
}

/// Runs a page for the session of the request, or sends the browser to
/// the sign-in page.
macro_rules! signed_in {
    ($console:expr, $headers:expr) => {
        match session_of(&$console, &$headers) {
            Some(session) => session,
            None => return redirect("/sign-in"),
        }
    };
}

async fn overview(
    State(console): State<Console>,
    headers: HeaderMap,
    request: axum::extract::Request,
) -> Response {
    let session = signed_in!(console, headers);
    let state = console
        .ops(&session, Call::get("/xrpc/internal.ops.getOverview"))
        .await;
    let listed = console
        .ops(&session, Call::get("/xrpc/internal.ops.listSubscribers"))
        .await;
    match (state, listed) {
        (Ok(state), Ok(listed)) => {
            let subscribers = listed["subscribers"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let done = param(request.uri().query(), "done");
            let page = pages::overview(&session, &state, &subscribers, done.as_deref());
            html(StatusCode::OK, page)
        }
        (Err(error), _) | (_, Err(error)) => failed(Some(&session), &error),
    }
}

/// The state for the live numbers of the overview.
async fn live(State(console): State<Console>, headers: HeaderMap) -> Response {
    let Some(session) = session_of(&console, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let state = console
        .ops(&session, Call::get("/xrpc/internal.ops.getOverview"))
        .await;
    let listed = console
        .ops(&session, Call::get("/xrpc/internal.ops.listSubscribers"))
        .await;
    let (Ok(state), Ok(listed)) = (state, listed) else {
        return StatusCode::BAD_GATEWAY.into_response();
    };
    let body = json!({ "overview": state, "subscribers": listed["subscribers"] });
    (
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body.to_string(),
    )
        .into_response()
}

async fn accounts(
    State(console): State<Console>,
    headers: HeaderMap,
    request: axum::extract::Request,
) -> Response {
    let session = signed_in!(console, headers);
    let mut path = "/xrpc/internal.ops.listAccounts?limit=50".to_owned();
    if let Some(cursor) = param(request.uri().query(), "cursor") {
        path.push_str("&cursor=");
        path.push_str(&pages::encode(&cursor));
    }
    match console.ops(&session, Call::get(&path)).await {
        Ok(page) => html(StatusCode::OK, pages::accounts(&session, &page)),
        Err(error) => failed(Some(&session), &error),
    }
}

async fn account(
    State(console): State<Console>,
    Path(did): Path<String>,
    headers: HeaderMap,
    request: axum::extract::Request,
) -> Response {
    let session = signed_in!(console, headers);
    let path = format!("/xrpc/internal.ops.getAccount?did={}", pages::encode(&did));
    match console.ops(&session, Call::get(&path)).await {
        Ok(card) => {
            let done = param(request.uri().query(), "done");
            html(
                StatusCode::OK,
                pages::account(&session, &card, done.as_deref()),
            )
        }
        Err(error) => failed(Some(&session), &error),
    }
}

async fn audit(
    State(console): State<Console>,
    headers: HeaderMap,
    request: axum::extract::Request,
) -> Response {
    let session = signed_in!(console, headers);
    let before = param(request.uri().query(), "before").and_then(|value| value.parse::<u64>().ok());
    let path = before.map_or_else(
        || "/xrpc/internal.ops.listAudit?limit=50".to_owned(),
        |before| format!("/xrpc/internal.ops.listAudit?limit=50&before={before}"),
    );
    match console.ops(&session, Call::get(&path)).await {
        Ok(page) => html(StatusCode::OK, pages::audit(&session, &page)),
        Err(error) => failed(Some(&session), &error),
    }
}

async fn domains(
    State(console): State<Console>,
    headers: HeaderMap,
    request: axum::extract::Request,
) -> Response {
    let session = signed_in!(console, headers);
    let call = Call::get("/xrpc/internal.ops.listBlockedEmailDomains");
    match console.ops(&session, call).await {
        Ok(list) => {
            let done = param(request.uri().query(), "done");
            html(
                StatusCode::OK,
                pages::domains(&session, &list, done.as_deref()),
            )
        }
        Err(error) => failed(Some(&session), &error),
    }
}

async fn roles(State(console): State<Console>, headers: HeaderMap) -> Response {
    let session = signed_in!(console, headers);
    match console
        .ops(&session, Call::get("/xrpc/internal.ops.listRoles"))
        .await
    {
        Ok(list) => html(StatusCode::OK, pages::roles(&session, &list)),
        Err(error) => failed(Some(&session), &error),
    }
}

/// The fields of a form.
async fn form(body: Body) -> Option<Vec<(String, String)>> {
    let bytes = axum::body::to_bytes(body, FORM_LIMIT).await.ok()?;
    Some(
        url::form_urlencoded::parse(&bytes)
            .map(|(key, value)| (key.into_owned(), value.trim().to_owned()))
            .collect(),
    )
}

fn field<'a>(fields: &'a [(String, String)], name: &str) -> &'a str {
    fields
        .iter()
        .find(|(key, _)| key == name)
        .map_or("", |(_, value)| value.as_str())
}

/// Whether a form comes from a page of this console: the token of the
/// session, and the origin of the request when the browser names it.
fn form_is_ours(console: &Console, session: &Session, headers: &HeaderMap, csrf: &str) -> bool {
    let token_ok: bool = csrf.as_bytes().ct_eq(session.csrf.as_bytes()).into();
    let origin_ok = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .is_none_or(|origin| origin == console.config().public_url);
    token_ok && origin_ok
}

/// What an action of a form is on the ops listener: the method, its body,
/// and the page that follows it.
fn plan(
    action: &str,
    fields: &[(String, String)],
) -> Result<(&'static str, Value, String), String> {
    let did = field(fields, "did");
    let account_page = format!("/accounts/{}", pages::encode(did));
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

/// Takes the action of a form.
async fn act(State(console): State<Console>, headers: HeaderMap, body: Body) -> Response {
    let session = signed_in!(console, headers);
    let bad = |message: &str| {
        html(
            StatusCode::BAD_REQUEST,
            pages::failure(Some(&session), message),
        )
    };
    let Some(fields) = form(body).await else {
        return bad("The form is too large.");
    };
    if !form_is_ours(&console, &session, &headers, field(&fields, "csrf")) {
        return html(
            StatusCode::FORBIDDEN,
            pages::failure(Some(&session), "The form did not come from this console."),
        );
    }
    let reason = field(&fields, "reason");
    if reason.is_empty() {
        return bad("An action needs a reason.");
    }
    let action = field(&fields, "action");
    let (method, input, back) = match plan(action, &fields) {
        Ok(plan) => plan,
        Err(message) => return bad(&message),
    };
    let path = format!("/xrpc/{method}");
    match console
        .ops(&session, Call::post(&path, reason, &input))
        .await
    {
        Ok(_) => redirect(&format!("{back}?done={action}")),
        Err(error) => failed(Some(&session), &error),
    }
}

async fn sign_in(State(console): State<Console>) -> Response {
    html(
        StatusCode::OK,
        pages::sign_in(&console.config().pds_public_url),
    )
}

/// Starts an authorization at the PDS and sends the browser there.
async fn login(State(console): State<Console>) -> Response {
    let state = random_text::<24>();
    match console.inner.client.start(&state).await {
        Ok((address, pending)) => {
            console.inner.sessions.expect(state, pending);
            redirect(&address)
        }
        Err(error) => failed(None, &error),
    }
}

/// The browser is back from the PDS: the console takes the session, asks
/// the ops listener who the operator is, and sets the cookie.
async fn callback(State(console): State<Console>, request: axum::extract::Request) -> Response {
    let query = request.uri().query();
    let refuse = |status, message: &str| html(status, pages::failure(None, message));
    if let Some(error) = param(query, "error") {
        let message = format!("The server did not authorize the console: {error}.");
        return refuse(StatusCode::FORBIDDEN, &message);
    }
    if param(query, "iss").as_deref() != Some(console.config().pds_public_url.as_str()) {
        return refuse(
            StatusCode::BAD_REQUEST,
            "The answer does not come from the server of this console.",
        );
    }
    let pending = param(query, "state").and_then(|state| console.inner.sessions.take(&state));
    let (Some(pending), Some(code)) = (pending, param(query, "code")) else {
        return refuse(
            StatusCode::BAD_REQUEST,
            "The console did not start this sign-in, or it took too long.",
        );
    };
    let client = &console.inner.client;
    let tokens = match client.finish(&pending, &code).await {
        Ok(tokens) => tokens,
        Err(error) => return failed(None, &error),
    };
    let who = Call::get("/xrpc/internal.ops.getOperator");
    let role = match client.ops(&pending.key, &tokens.access, who).await {
        Ok(operator) => operator["role"].as_str().unwrap_or_default().to_owned(),
        Err(error) => {
            // An account without a role keeps no session of the console.
            client.revoke(&pending.key, &tokens.access).await;
            return failed(None, &error);
        }
    };
    let session = console.inner.sessions.start(pending.key, tokens, role);
    let cookie = format!(
        "{COOKIE}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
        session.id,
        session::SESSION_LIFE.as_secs()
    );
    let mut response = redirect("/");
    if let Ok(cookie) = HeaderValue::from_str(&cookie) {
        response.headers_mut().insert(header::SET_COOKIE, cookie);
    }
    response
}

/// Ends the session at the console and at the PDS.
async fn sign_out(State(console): State<Console>, headers: HeaderMap, body: Body) -> Response {
    let session = signed_in!(console, headers);
    let fields = form(body).await.unwrap_or_default();
    if !form_is_ours(&console, &session, &headers, field(&fields, "csrf")) {
        return html(
            StatusCode::FORBIDDEN,
            pages::failure(Some(&session), "The form did not come from this console."),
        );
    }
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
    response
}

#[cfg(test)]
mod tests {
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
        // Every action with a banner has a plan.
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
}
