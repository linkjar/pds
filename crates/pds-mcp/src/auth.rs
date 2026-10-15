//! The MCP server as an OAuth 2.1 authorization server for AI clients
//! (`docs/sidecars.md` §2.3).
//!
//! A person signs in on the pages of the PDS. This server then asks the
//! person whether the AI client may use the session, because the PDS
//! shows the name of this server and not the name of the AI client.

use std::fmt::Write as _;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use pds_sidecar::client::{hash, random_text};
use pds_sidecar::http::{cookie, field, form, json_response, param, redirect, same_origin};
use serde_json::{Value, json};
use subtle::ConstantTimeEq as _;
use url::Url;

use crate::clients;
use crate::config::PDS_CALLBACK_PATH;
use crate::store::{ACCESS_LIFE, DEFAULT_SCOPE, McpClient, Request, pds_scope};
use crate::{Mcp, escape};

/// The cookie that ties an authorization to the browser that started it.
const COOKIE: &str = "mcp_flow";

/// The largest body of a form or of a registration.
const BODY_LIMIT: usize = 64 * 1024;

/// An error of the token endpoint (RFC 6749 §5.2).
fn oauth_error(status: StatusCode, error: &str, description: &str) -> Response {
    json_response(
        status,
        &json!({ "error": error, "error_description": description }),
    )
}

fn page(status: StatusCode, title: &str, body: &str) -> Response {
    let html = format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} · LinkJar MCP</title>
<link rel="stylesheet" href="/mcp.css">
</head><body><main><h1>{title}</h1>
{body}
</main></body></html>"#,
        title = escape(title),
    );
    (
        status,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; style-src 'self'; frame-ancestors 'none'; base-uri 'none'",
            ),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::REFERRER_POLICY, "no-referrer"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        html,
    )
        .into_response()
}

fn problem(status: StatusCode, message: &str) -> Response {
    page(
        status,
        "The sign-in did not continue",
        &format!(r#"<p role="alert">{}</p>"#, escape(message)),
    )
}

/// The redirect to an AI client with the answer of an authorization.
fn answer(mcp: &Mcp, redirect_uri: &str, state: Option<&str>, pairs: &[(&str, &str)]) -> Response {
    let Ok(mut url) = Url::parse(redirect_uri) else {
        return problem(StatusCode::BAD_REQUEST, "The redirect URI is not a URL.");
    };
    {
        let mut query = url.query_pairs_mut();
        query.extend_pairs(pairs);
        if let Some(state) = state {
            query.append_pair("state", state);
        }
        query.append_pair("iss", &mcp.config().public_url);
    }
    redirect(url.as_str())
}

/// Whether the browser of the request is the one that started `request`.
fn same_browser(headers: &HeaderMap, request: &Request) -> bool {
    cookie(headers, COOKIE).is_some_and(|value| {
        hash(&value)
            .as_bytes()
            .ct_eq(request.browser.as_bytes())
            .into()
    })
}

/// `/.well-known/oauth-protected-resource` (RFC 9728).
pub async fn protected_resource(State(mcp): State<Mcp>) -> Response {
    let config = mcp.config();
    json_response(
        StatusCode::OK,
        &json!({
            "resource": config.resource(),
            "authorization_servers": [config.public_url],
            "scopes_supported": config.scopes(),
            "bearer_methods_supported": ["header"],
            "resource_name": "LinkJar MCP",
        }),
    )
}

/// `/.well-known/oauth-authorization-server` (RFC 8414).
pub async fn authorization_server(State(mcp): State<Mcp>) -> Response {
    let base = &mcp.config().public_url;
    json_response(
        StatusCode::OK,
        &json!({
            "issuer": base,
            "authorization_endpoint": format!("{base}/authorize"),
            "token_endpoint": format!("{base}/token"),
            "registration_endpoint": format!("{base}/register"),
            "revocation_endpoint": format!("{base}/revoke"),
            "response_types_supported": ["code"],
            "grant_types_supported": ["authorization_code", "refresh_token"],
            "code_challenge_methods_supported": ["S256"],
            "token_endpoint_auth_methods_supported": ["none"],
            "scopes_supported": mcp.config().scopes(),
            "authorization_response_iss_parameter_supported": true,
            "client_id_metadata_document_supported": true,
        }),
    )
}

/// The metadata of this server as a client of the PDS. The route is
/// outside the CORS layer of the API, so the answer carries the header
/// itself, as the other OAuth documents do.
pub async fn client_metadata(State(mcp): State<Mcp>) -> Response {
    let mut response = json_response(StatusCode::OK, &mcp.config().client_metadata());
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    response
}

/// `POST /register` (RFC 7591): a public client with its redirect URIs.
pub async fn register(State(mcp): State<Mcp>, body: Body) -> Response {
    let invalid = |description: &str| {
        oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_client_metadata",
            description,
        )
    };
    let Ok(bytes) = axum::body::to_bytes(body, BODY_LIMIT).await else {
        return invalid("the registration is too large");
    };
    let Ok(document) = serde_json::from_slice::<Value>(&bytes) else {
        return invalid("the registration is not JSON");
    };
    let method = document
        .get("token_endpoint_auth_method")
        .and_then(Value::as_str)
        .unwrap_or("none");
    if method != "none" {
        return invalid(
            "the server registers public clients: token_endpoint_auth_method is \"none\"",
        );
    }
    let (name, redirect_uris) = match clients::read_metadata(&document) {
        Ok(read) => read,
        Err(description) => return invalid(&description),
    };
    let client = mcp.store().register(name, redirect_uris);
    json_response(
        StatusCode::CREATED,
        &json!({
            "client_id": client.id,
            "client_name": client.name,
            "redirect_uris": client.redirect_uris,
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
        }),
    )
}

/// The client of an identifier: a registration, or a metadata document.
async fn client_of(mcp: &Mcp, client_id: &str) -> Result<McpClient, String> {
    if let Some(client) = mcp.store().client(client_id) {
        return Ok(client);
    }
    match clients::is_document_url(client_id) {
        Some(url) => clients::fetch(mcp.http(), client_id, &url).await,
        None => Err("The server does not know this client.".to_owned()),
    }
}

/// `GET /authorize`: checks the request of the AI client and sends the
/// browser to the PDS.
pub async fn authorize(State(mcp): State<Mcp>, request: axum::extract::Request) -> Response {
    let query = request.uri().query();
    let get = |name: &str| param(query, name);
    let bad = |message: &str| problem(StatusCode::BAD_REQUEST, message);
    let Some(client_id) = get("client_id") else {
        return bad("The request names no client.");
    };
    let client = match client_of(&mcp, &client_id).await {
        Ok(client) => client,
        Err(message) => return bad(&message),
    };
    let Some(redirect_uri) = get("redirect_uri").filter(|uri| clients::has_redirect(&client, uri))
    else {
        return bad("The redirect URI is not one of the client.");
    };
    // From here an error goes to the client.
    let state = get("state");
    let fail = |error: &str, description: &str| {
        let pairs = [("error", error), ("error_description", description)];
        answer(&mcp, &redirect_uri, state.as_deref(), &pairs)
    };
    if get("response_type").as_deref() != Some("code") {
        return fail(
            "unsupported_response_type",
            "response_type must be \"code\"",
        );
    }
    let challenge = get("code_challenge").unwrap_or_default();
    if get("code_challenge_method").as_deref() != Some("S256") || challenge.len() != 43 {
        return fail("invalid_request", "PKCE with S256 is required");
    }
    if get("resource").is_some_and(|resource| resource != mcp.config().resource()) {
        return fail("invalid_target", "the resource is not this server");
    }
    let scopes: Vec<String> = get("scope")
        .unwrap_or_else(|| DEFAULT_SCOPE.to_owned())
        .split(' ')
        .filter(|scope| !scope.is_empty())
        .map(str::to_owned)
        .collect();
    let known = mcp.config().scopes();
    if scopes.is_empty() || !scopes.iter().all(|scope| known.contains(scope)) {
        return fail(
            "invalid_scope",
            "the server does not know a scope of the request",
        );
    }
    let browser = random_text::<24>();
    let mut opened = Request::new(
        client,
        redirect_uri.clone(),
        state.clone(),
        challenge,
        scopes,
        &browser,
    );
    let id = random_text::<24>();
    match mcp.pds().start(&id, &pds_scope(&opened.scopes)).await {
        Ok((address, pending)) => {
            opened.pending = Some(pending);
            mcp.store().put_back(&id, opened);
            let mut response = redirect(&address);
            let secure = if mcp.config().public_url.starts_with("https://") {
                "; Secure"
            } else {
                ""
            };
            let value =
                format!("{COOKIE}={browser}; Path=/; HttpOnly; SameSite=Lax; Max-Age=600{secure}");
            if let Ok(value) = HeaderValue::from_str(&value) {
                response.headers_mut().insert(header::SET_COOKIE, value);
            }
            response
        }
        Err(error) => {
            tracing::warn!(%error, "the PDS did not take the authorization request");
            fail("server_error", "the PDS did not take the request")
        }
    }
}

fn consent_page(id: &str, request: &Request, did: &str) -> Response {
    let name = request.client.name.as_deref().unwrap_or("An application");
    let host = Url::parse(&request.redirect_uri)
        .ok()
        .map(|url| match url.host_str() {
            Some(host) => host.to_owned(),
            None => format!("{}:", url.scheme()),
        })
        .unwrap_or_default();
    let mut scopes = String::new();
    for scope in &request.scopes {
        let text = match scope.as_str() {
            "account:read" => "See which account you are".to_owned(),
            "repo:read" => "Read the records of your repository".to_owned(),
            "repo:write" => "Create, change and delete records of your repository".to_owned(),
            other => format!(
                "Create, change and delete records of {}",
                other.trim_start_matches("repo:write:")
            ),
        };
        let _ = write!(scopes, "<li>{}</li>", escape(&text));
    }
    let body = format!(
        r#"<p><strong>{name}</strong> asks for access to the account <code>{did}</code>.</p>
<p>After your answer the browser goes to <code>{host}</code>. Allow this only when you started the sign-in from that application.</p>
<ul>{scopes}</ul>
<form method="post" action="/authorize/decision">
<input type="hidden" name="request" value="{id}">
<input type="hidden" name="csrf" value="{csrf}">
<button name="decision" value="deny">Deny</button>
<button name="decision" value="allow" class="primary">Allow</button>
</form>"#,
        name = escape(name),
        did = escape(did),
        host = escape(&host),
        id = escape(id),
        csrf = escape(&request.csrf),
    );
    page(StatusCode::OK, "Allow access", &body)
}

/// The browser is back from the PDS: the server takes the session and
/// asks the person about the AI client.
pub async fn pds_callback(
    State(mcp): State<Mcp>,
    headers: HeaderMap,
    request: axum::extract::Request,
) -> Response {
    debug_assert_eq!(request.uri().path(), PDS_CALLBACK_PATH);
    let query = request.uri().query();
    let unknown = || {
        problem(
            StatusCode::BAD_REQUEST,
            "The server did not start this sign-in, or it took too long.",
        )
    };
    let Some(id) = param(query, "state") else {
        return unknown();
    };
    let Some(mut opened) = mcp.store().take(&id) else {
        return unknown();
    };
    if !same_browser(&headers, &opened) {
        // The request stays for the browser that started it.
        mcp.store().put_back(&id, opened);
        return unknown();
    }
    let fail = |opened: &Request, error: &str, description: &str| {
        let pairs = [("error", error), ("error_description", description)];
        answer(&mcp, &opened.redirect_uri, opened.state.as_deref(), &pairs)
    };
    if param(query, "error").is_some() {
        return fail(
            &opened,
            "access_denied",
            "the sign-in at the PDS was refused",
        );
    }
    if param(query, "iss").as_deref() != Some(mcp.config().pds_public_url.as_str()) {
        return unknown();
    }
    let (Some(pending), Some(code)) = (opened.pending.take(), param(query, "code")) else {
        return unknown();
    };
    match mcp.pds().finish(pending, &code).await {
        Ok(session) => {
            let did = session.did.clone();
            opened.pds = Some(session);
            let response = consent_page(&id, &opened, &did);
            mcp.store().put_back(&id, opened);
            response
        }
        Err(error) => {
            tracing::warn!(%error, "the PDS gave no session");
            fail(&opened, "server_error", "the PDS gave no session")
        }
    }
}

/// `POST /authorize/decision`: the answer of the person.
pub async fn decision(State(mcp): State<Mcp>, headers: HeaderMap, body: Body) -> Response {
    let fields = form(body, BODY_LIMIT).await.unwrap_or_default();
    let id = field(&fields, "request");
    let refuse = || {
        problem(
            StatusCode::FORBIDDEN,
            "The form did not come from this server.",
        )
    };
    let Some(mut opened) = mcp.store().take(id) else {
        return problem(
            StatusCode::BAD_REQUEST,
            "The server did not start this sign-in, or it took too long.",
        );
    };
    let csrf_ok: bool = field(&fields, "csrf")
        .as_bytes()
        .ct_eq(opened.csrf.as_bytes())
        .into();
    let origin_ok = same_origin(&headers, &mcp.config().public_url);
    if !csrf_ok || !origin_ok || !same_browser(&headers, &opened) {
        mcp.store().put_back(id, opened);
        return refuse();
    }
    let Some(session) = opened.pds.take() else {
        return refuse();
    };
    let (redirect_uri, state) = (opened.redirect_uri.clone(), opened.state.clone());
    if field(&fields, "decision") != "allow" {
        mcp.pds().revoke(&session).await;
        let pairs = [("error", "access_denied")];
        return answer(&mcp, &redirect_uri, state.as_deref(), &pairs);
    }
    let code = mcp.store().grant(opened, session);
    answer(&mcp, &redirect_uri, state.as_deref(), &[("code", &code)])
}

fn issued(tokens: &crate::store::Issued) -> Response {
    json_response(
        StatusCode::OK,
        &json!({
            "access_token": tokens.access,
            "token_type": "Bearer",
            "expires_in": ACCESS_LIFE.as_secs(),
            "refresh_token": tokens.refresh,
            "scope": tokens.scope,
        }),
    )
}

/// `POST /token`: a code or a refresh token for new tokens.
pub async fn token(State(mcp): State<Mcp>, body: Body) -> Response {
    let fields = form(body, BODY_LIMIT).await.unwrap_or_default();
    let get = |name: &str| field(&fields, name);
    let grant =
        |description: &str| oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", description);
    match get("grant_type") {
        "authorization_code" => {
            let Some(code) = mcp.store().redeem(get("code")) else {
                return grant("the code is not one of this server, or it ended");
            };
            let fits = code.client_id == get("client_id")
                && code.redirect_uri == get("redirect_uri")
                && pds_oauth::pkce::verify_s256(get("code_verifier"), &code.challenge);
            if !fits {
                mcp.pds().revoke(&code.pds).await;
                return grant("the client, the redirect URI or the verifier does not fit the code");
            }
            issued(&mcp.store().start(code))
        }
        "refresh_token" => match mcp.store().rotate(get("refresh_token"), get("client_id")) {
            Ok(Some(tokens)) => issued(&tokens),
            Ok(None) => grant("the refresh token is not one of this client, or it ended"),
            Err(session) => {
                // A refresh token came a second time: both sides end.
                mcp.pds().revoke(&session.pds).await;
                grant("the refresh token was used before; the session is ended")
            }
        },
        _ => oauth_error(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            "grant_type must be \"authorization_code\" or \"refresh_token\"",
        ),
    }
}

/// `POST /revoke` (RFC 7009): ends the session of a token on both sides.
pub async fn revoke(State(mcp): State<Mcp>, body: Body) -> Response {
    let fields = form(body, BODY_LIMIT).await.unwrap_or_default();
    if let Some(session) = mcp.store().end(field(&fields, "token")) {
        mcp.pds().revoke(&session.pds).await;
    }
    json_response(StatusCode::OK, &json!({}))
}
