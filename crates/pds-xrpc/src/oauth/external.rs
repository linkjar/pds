//! Sign-in through an identity provider (`docs/SPEC.md` §12.3; patch 099).
//!
//! A flow has three requests. `start` binds a one-use state, a nonce and
//! a PKCE verifier to the device and to the pushed request, and sends the
//! browser to the provider. `callback` records what the provider sent
//! back. `complete` runs on the device that started: it exchanges the
//! code, finds or creates the account, and returns to the authorization
//! or to the account page. The callback alone signs nobody in.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use pds_ext::{Boxed, Fetch, ProviderAnswer, ProviderInfo, Suggested, VerifiedIdentity};
use pds_oauth::token::mint_ephemeral;
use pds_storage::Availability;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::authorize::{
    Loaded, Page, Pending, authorize, consent_required, device, error_page, load,
};
use super::{accounts, clients, iso, issuer};
use crate::api::account::{External, Request};
use crate::state::AppState;

/// A flow waits 10 minutes for the provider.
const FLOW_LIFETIME_MS: i64 = 10 * 60 * 1000;
/// The server holds 1,000 flows, and 3 of one device.
const MAX_FLOWS: usize = 1000;
const MAX_FLOWS_OF_A_DEVICE: usize = 3;
/// An address starts 20 flows and completes 10 in a minute.
const STARTS_A_MINUTE: u32 = 20;
const COMPLETIONS_A_MINUTE: u32 = 10;
const MAX_LIMITER_KEYS: usize = 10_000;
/// An answer of a provider is at most 128 KiB and takes at most 10 seconds.
const ANSWER_LIMIT: usize = 128 * 1024;
const ANSWER_TIMEOUT: Duration = Duration::from_secs(10);

/// What the page says when a flow fails. The cause goes to the log.
const FAILED: &str = "External sign-in failed. Start again from your app or account page.";

/// What the provider sent back.
#[derive(Debug, Clone)]
enum Answer {
    Code { code: String, user: Option<String> },
    Refused,
}

/// One sign-in at a provider.
#[derive(Debug, Clone)]
struct Flow {
    provider: &'static str,
    device_id: String,
    /// The pushed request, or none for a sign-in on the account page.
    request_uri: Option<String>,
    verifier: String,
    nonce: String,
    expires_ms: i64,
    answer: Option<Answer>,
}

/// The flows in progress and the count of attempts by address. A restart
/// ends the flows in progress.
#[derive(Debug, Default)]
pub struct Flows {
    flows: Mutex<HashMap<String, Flow>>,
    attempts: Mutex<HashMap<String, (i64, u32)>>,
}

/// Why a flow stops. The page shows [`FAILED`] for each.
type Stop = &'static str;

impl Flows {
    /// Counts an attempt of an address against the limit of its stage.
    fn allow(&self, ip: &str, start: bool, now_ms: i64) -> Result<(), Stop> {
        let mut attempts = self.attempts.lock().unwrap_or_else(PoisonError::into_inner);
        attempts.retain(|_, (expires, _)| *expires > now_ms);
        let key = format!("{}:{ip}", if start { "start" } else { "complete" });
        let limit = if start {
            STARTS_A_MINUTE
        } else {
            COMPLETIONS_A_MINUTE
        };
        let known = attempts.contains_key(&key);
        let bucket = attempts.get(&key).copied().unwrap_or((now_ms + 60_000, 0));
        if (!known && attempts.len() >= MAX_LIMITER_KEYS) || bucket.1 >= limit {
            return Err("Too many external sign-in attempts");
        }
        attempts.insert(key, (bucket.0, bucket.1 + 1));
        drop(attempts);
        Ok(())
    }

    fn create(&self, flow: Flow, now_ms: i64) -> Result<String, Stop> {
        let mut flows = self.flows.lock().unwrap_or_else(PoisonError::into_inner);
        flows.retain(|_, flow| flow.expires_ms > now_ms);
        let of_device = flows
            .values()
            .filter(|other| other.device_id == flow.device_id)
            .count();
        if flows.len() >= MAX_FLOWS || of_device >= MAX_FLOWS_OF_A_DEVICE {
            return Err("Too many pending external sign-ins");
        }
        let state = URL_SAFE_NO_PAD.encode(super::random::<32>());
        flows.insert(state.clone(), flow);
        drop(flows);
        Ok(state)
    }

    /// Records the provider's answer. An answer comes once.
    fn receive(
        &self,
        state: &str,
        provider: &str,
        answer: Answer,
        now_ms: i64,
    ) -> Result<(), Stop> {
        let mut flows = self.flows.lock().unwrap_or_else(PoisonError::into_inner);
        let flow = flows
            .get_mut(state)
            .filter(|flow| flow.provider == provider && flow.expires_ms > now_ms)
            .ok_or("Unknown or expired external sign-in")?;
        if flow.answer.is_some() {
            return Err("External response already received");
        }
        flow.answer = Some(answer);
        drop(flows);
        Ok(())
    }

    /// Takes the flow of a state for the device that started it. A flow
    /// is taken once, also when it then fails.
    fn consume(
        &self,
        state: &str,
        provider: &str,
        device_id: &str,
        now_ms: i64,
    ) -> Result<Flow, Stop> {
        let mut flows = self.flows.lock().unwrap_or_else(PoisonError::into_inner);
        let known = flows
            .get(state)
            .is_some_and(|flow| flow.provider == provider && flow.expires_ms > now_ms);
        if !known {
            return Err("Unknown or expired external sign-in");
        }
        if flows
            .get(state)
            .is_some_and(|flow| flow.device_id != device_id)
        {
            return Err("External sign-in belongs to another device");
        }
        flows
            .remove(state)
            .ok_or("Unknown or expired external sign-in")
    }
}

/// Requests to a provider: 10 seconds, 128 KiB, no redirect.
struct Http<'a>(&'a reqwest::Client);

async fn read_json(request: reqwest::RequestBuilder) -> Result<Value, &'static str> {
    let mut response = request
        .header(header::ACCEPT, "application/json")
        .timeout(ANSWER_TIMEOUT)
        .send()
        .await
        .map_err(|_| "External provider request failed")?;
    if !response.status().is_success() {
        return Err("External provider request failed");
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "External provider request failed")?
    {
        if bytes.len() + chunk.len() > ANSWER_LIMIT {
            return Err("External provider response is too large");
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "Invalid external provider response")
}

impl Fetch for Http<'_> {
    fn post_form<'a>(
        &'a self,
        url: &'a str,
        form: &'a [(&'static str, String)],
    ) -> Boxed<'a, Result<Value, &'static str>> {
        Box::pin(async move {
            let body = url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(form.iter().map(|(name, value)| (*name, value.as_str())))
                .finish();
            read_json(
                self.0
                    .post(url)
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(body),
            )
            .await
        })
    }

    fn get_json<'a>(
        &'a self,
        url: &'a str,
        bearer: Option<&'a str>,
    ) -> Boxed<'a, Result<Value, &'static str>> {
        Box::pin(async move {
            let mut request = self.0.get(url);
            if let Some(token) = bearer {
                request = request
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header("x-github-api-version", "2022-11-28")
                    .header(header::USER_AGENT, "atproto-external-auth");
            }
            read_json(request).await
        })
    }
}

fn see_other(location: &str, cookies: &[HeaderValue]) -> Response {
    let mut response = StatusCode::SEE_OTHER.into_response();
    let headers = response.headers_mut();
    if let Ok(location) = HeaderValue::from_str(location) {
        headers.insert(header::LOCATION, location);
    }
    for cookie in cookies {
        headers.append(header::SET_COOKIE, cookie.clone());
    }
    response
}

fn query_of(query: Option<&str>) -> HashMap<String, String> {
    url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
        .into_owned()
        .collect()
}

/// The page that a start request comes from: the authorization page or
/// the account page, on this origin.
fn referrer_page(state: &AppState, headers: &HeaderMap) -> Result<bool, Stop> {
    let site = headers
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok());
    if site.is_some_and(|site| site != "same-origin") {
        return Err("The request came from another site");
    }
    let referrer = headers
        .get(header::REFERER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| url::Url::parse(value).ok())
        .ok_or("Invalid external sign-in page")?;
    if referrer.origin().ascii_serialization() != issuer(state) {
        return Err("Invalid external sign-in page");
    }
    let path = referrer.path();
    if path == "/account" || path.starts_with("/account/") {
        Ok(true)
    } else if path == "/oauth/authorize" || path.starts_with("/oauth/authorize/") {
        Ok(false)
    } else {
        Err("Invalid external sign-in page")
    }
}

async fn start(
    state: &AppState,
    provider: ProviderInfo,
    query: Option<&str>,
    headers: &HeaderMap,
    peer: Option<IpAddr>,
) -> Result<Response, Stop> {
    let account_page = referrer_page(state, headers)?;
    let query = query_of(query);
    let request_uri = query.get("request_uri").cloned();
    if account_page == request_uri.is_some() {
        return Err("Authorization request mismatch");
    }
    let device = device(state, headers, peer, true)
        .await
        .map_err(|_| "The device could not be loaded")?
        .ok_or("The device could not be loaded")?;
    let now_ms = state.now_ms();
    let flows = &state.oauth().flows;
    flows.allow(&crate::ratelimit::client_ip(headers, peer), true, now_ms)?;
    if let Some(request_uri) = &request_uri {
        match load(state, request_uri, None, &device.id).await {
            Ok(Loaded::Ready(_)) => {}
            _ => return Err("The authorization request is not open"),
        }
    }
    let verifier = URL_SAFE_NO_PAD.encode(super::random::<32>());
    let nonce = URL_SAFE_NO_PAD.encode(super::random::<32>());
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let flow_state = flows.create(
        Flow {
            provider: provider.id,
            device_id: device.id.clone(),
            request_uri,
            verifier,
            nonce: nonce.clone(),
            expires_ms: now_ms + FLOW_LIFETIME_MS,
            answer: None,
        },
        now_ms,
    )?;
    let callback = format!("{}/oauth/external/{}/callback", issuer(state), provider.id);
    let target = state
        .config()
        .profile
        .provider_start(provider.id, &callback, &flow_state, &nonce, &challenge)
        .ok_or("The provider is not configured")??;
    Ok(see_other(&target, &device.cookies))
}

/// Records what the provider sent. The request may come without the
/// device's cookies: a provider that answers with a form post sends it
/// from its own site.
async fn callback(
    state: &AppState,
    provider: ProviderInfo,
    method: &Method,
    query: Option<&str>,
    headers: &HeaderMap,
    body: Body,
) -> Result<Response, Stop> {
    let input: HashMap<String, String> = if provider.form_post {
        if method != Method::POST {
            return Err("The callback takes a form post");
        }
        super::read_fields(headers, body)
            .await
            .map_err(|_| "Invalid callback")?
            .into_iter()
            .collect()
    } else {
        query_of(query)
    };
    let flow_state = input
        .get("state")
        .filter(|state| {
            state.len() == 43
                && state
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
        .ok_or("Invalid callback")?;
    let answer = match (input.get("code"), input.get("error")) {
        (Some(code), None) if !code.is_empty() && code.len() <= 4096 => Answer::Code {
            code: code.clone(),
            user: input.get("user").filter(|user| user.len() <= 8192).cloned(),
        },
        (None, Some(error)) if !error.is_empty() && error.len() <= 128 => Answer::Refused,
        _ => return Err("Invalid callback"),
    };
    state
        .oauth()
        .flows
        .receive(flow_state, provider.id, answer, state.now_ms())?;
    Ok(see_other(
        &format!(
            "/oauth/external/{}/complete?state={flow_state}",
            provider.id
        ),
        &[],
    ))
}

/// The account of an identity that has none yet: a hosted handle that
/// the profile suggests, the provider's verified address, no password.
async fn create_account(
    state: &AppState,
    identity: &VerifiedIdentity,
    signup: Option<&(String, String, String)>,
) -> Result<String, Stop> {
    let email = identity
        .email
        .clone()
        .filter(|_| identity.email_verified)
        .ok_or("A verified email is required to create an account")?;
    let profile = &state.config().profile;
    for _ in 0..3 {
        let mut handle = None;
        for attempt in 0.. {
            let suffix: [u8; 3] = super::random();
            let label = profile.suggested_label(
                identity.display_name.as_deref(),
                Some(&email),
                attempt,
                suffix,
            );
            let candidate = match label {
                Suggested::Handle(candidate) => candidate,
                Suggested::Skip => continue,
                Suggested::End => break,
            };
            let (lookup, address) = (candidate.clone(), email.to_lowercase());
            let (taken, email_taken) = accounts(state, move |db| {
                Ok((
                    db.actor(&lookup, Availability::ANY)?.is_some(),
                    db.account(&address, true, Availability::ANY)?.is_some(),
                ))
            })
            .await
            .map_err(|_| "The account database failed")?;
            // An address that an account holds is refused. It is never linked.
            if email_taken {
                return Err("The email address belongs to an existing account");
            }
            if !taken {
                handle = Some(candidate);
                break;
            }
        }
        let handle = handle.ok_or("Could not allocate a handle")?;
        let request = Request {
            handle,
            email: email.clone(),
            password: None,
            invite_code: None,
            recovery_key: None,
            external: Some(External {
                provider: identity.provider.to_owned(),
                subject: identity.subject.clone(),
                display_name: identity.display_name.clone(),
            }),
            legacy_session: false,
            signup: signup.cloned(),
        };
        match crate::api::account::create(state, request).await {
            Ok(created) => return Ok(created.did),
            // Another sign-up took the handle between the lookup and the write.
            Err(error) if error.message == "User already exists" => {}
            Err(error) => {
                tracing::warn!(error = %error.message, "an external account was not created");
                return Err("The account could not be created");
            }
        }
    }
    Err("Unable to allocate a handle")
}

async fn complete(
    state: &AppState,
    provider: ProviderInfo,
    query: Option<&str>,
    headers: &HeaderMap,
    peer: Option<IpAddr>,
) -> Result<Response, Stop> {
    let device = device(state, headers, peer, false)
        .await
        .map_err(|_| "The device could not be loaded")?
        .ok_or("Original browser session is required")?;
    let now_ms = state.now_ms();
    let flows = &state.oauth().flows;
    flows.allow(&crate::ratelimit::client_ip(headers, peer), false, now_ms)?;
    let flow_state = query_of(query).remove("state").unwrap_or_default();
    let flow = flows.consume(&flow_state, provider.id, &device.id, now_ms)?;
    let Some(Answer::Code { code, user }) = flow.answer.clone() else {
        return Err("External sign-in canceled");
    };
    let pending: Option<Box<Pending>> = match &flow.request_uri {
        Some(request_uri) => match load(state, request_uri, None, &device.id).await {
            Ok(Loaded::Ready(pending)) => Some(pending),
            _ => return Err("The authorization request is not open"),
        },
        None => None,
    };
    let callback = format!("{}/oauth/external/{}/callback", issuer(state), provider.id);
    let identity = state
        .config()
        .profile
        .provider_exchange(
            provider.id,
            &Http(state.http()),
            ProviderAnswer {
                redirect_uri: &callback,
                code: &code,
                verifier: &flow.verifier,
                nonce: &flow.nonce,
                user: user.as_deref(),
                now_s: now_ms / 1000,
            },
        )
        .await?;

    finish(state, &device, pending, &identity).await
}

/// The end of a flow, once the provider said who signed in: the account
/// of the identity, and the way back to the authorization or to the
/// account page.
async fn finish(
    state: &AppState,
    device: &super::authorize::Device,
    pending: Option<Box<Pending>>,
    identity: &VerifiedIdentity,
) -> Result<Response, Stop> {
    let now_ms = state.now_ms();
    let known = {
        let (provider, subject) = (identity.provider, identity.subject.clone());
        accounts(state, move |db| {
            db.external_identity(provider, &subject, &iso(now_ms))
        })
        .await
        .map_err(|_| "The account database failed")?
    };
    let created = known.is_none();
    let hint = pending
        .as_ref()
        .and_then(|pending| pending.text("login_hint"));
    if created && hint.is_some() {
        return Err("No linked account matches the requested account");
    }
    if created && state.config().invite_required {
        return Err("External account creation is disabled while invites are required");
    }
    let did = if let Some(did) = known {
        did
    } else {
        // An account that is created in an authorization leaves its
        // receipt there. A sign-in on the account page leaves none.
        let signup = pending.as_ref().map(|pending| {
            (
                pending.row.id.clone(),
                pending.row.client_id.clone(),
                device.id.clone(),
            )
        });
        create_account(state, identity, signup.as_ref()).await?
    };
    let actor = {
        let did = did.clone();
        accounts(state, move |db| {
            db.actor(
                &did,
                Availability {
                    include_taken_down: false,
                    include_deactivated: true,
                },
            )
        })
        .await
        .map_err(|_| "The account database failed")?
        .ok_or("The account is not available")?
    };
    let handle = actor.handle.clone().unwrap_or_else(|| did.clone());
    if hint.is_some_and(|hint| hint != did && hint != handle) {
        return Err("External account does not match the requested account");
    }

    let Some(pending) = pending else {
        // A sign-in on the account page: the device remembers the account.
        let (device_id, did) = (device.id.clone(), did.clone());
        accounts(state, move |db| {
            db.upsert_device_account(&device_id, &did, &iso(now_ms))
        })
        .await
        .map_err(|_| "The account database failed")?;
        return Ok(see_other("/account", &device.cookies));
    };
    let client = clients::get(state, &pending.row.client_id)
        .await
        .map_err(|_| "The client could not be loaded")?;
    // A new account that came through a trusted client has authorized it
    // by signing up there. A known identity gets no such grant.
    let authorized = {
        let (did, client_id) = (did.clone(), client.client.id.clone());
        let grant = (created && client.trusted)
            .then(|| json!({ "authorizedScopes": pending.scopes() }).to_string());
        accounts(state, move |db| {
            if let Some(grant) = &grant {
                db.set_authorized_client(&did, &client_id, grant, &iso(now_ms))?;
            }
            db.authorized_client(&did, &client_id)
        })
        .await
        .map_err(|_| "The account database failed")?
    };
    if actor.deactivated_at.is_none() && !consent_required(&pending, authorized.as_deref()) {
        return authorize(state, &pending, &did, &device.cookies)
            .await
            .map_err(|_| "The authorization failed");
    }
    // The device does not remember the account: the consent form carries
    // a proof of this sign-in.
    let proof = mint_ephemeral(
        &state.config().auth.jwt_secret,
        &did,
        &device.id,
        &pending.row.id,
        now_ms / 1000,
    );
    let page = Page {
        state,
        client: &client,
        pending: &pending,
        device,
    };
    Ok(page.consent(&did, &handle, Some(&proof)))
}

/// Serves `/oauth/external/<provider>/start`, `/callback` and
/// `/complete`. `None` means the path names no provider of the profile.
pub async fn serve(
    state: &AppState,
    method: &Method,
    path: &str,
    query: Option<&str>,
    headers: &HeaderMap,
    peer: Option<IpAddr>,
    body: &mut Option<Body>,
) -> Option<Response> {
    let (name, step) = path.strip_prefix("/oauth/external/")?.split_once('/')?;
    let provider = state
        .config()
        .profile
        .providers()
        .into_iter()
        .find(|provider| provider.id == name)?;
    let reads = method == Method::GET || method == Method::HEAD;
    let outcome = match step {
        "start" if reads => start(state, provider, query, headers, peer).await,
        "callback" if reads || method == Method::POST => {
            let body = body.take().unwrap_or_else(Body::empty);
            callback(state, provider, method, query, headers, body).await
        }
        "complete" if reads => complete(state, provider, query, headers, peer).await,
        _ => return None,
    };
    let mut response = outcome.unwrap_or_else(|cause| {
        tracing::info!(provider = provider.id, cause, "External sign-in failed");
        error_page(StatusCode::BAD_REQUEST, &[], FAILED)
    });
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers
        .entry(header::REFERRER_POLICY)
        .or_insert(HeaderValue::from_static("no-referrer"));
    Some(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flow(device: &str) -> Flow {
        Flow {
            provider: "google",
            device_id: device.to_owned(),
            request_uri: None,
            verifier: "v".to_owned(),
            nonce: "n".to_owned(),
            expires_ms: 1000,
            answer: None,
        }
    }

    #[test]
    fn a_flow_is_for_one_provider_one_device_and_one_use() {
        let flows = Flows::default();
        let state = flows.create(flow("dev-1"), 0).unwrap();
        assert_eq!(state.len(), 43);
        assert!(flows.receive(&state, "github", Answer::Refused, 1).is_err());
        let code = Answer::Code {
            code: "c".to_owned(),
            user: None,
        };
        flows.receive(&state, "google", code.clone(), 1).unwrap();
        assert!(
            flows.receive(&state, "google", code, 1).is_err(),
            "an answer comes once"
        );
        assert!(flows.consume(&state, "google", "dev-2", 1).is_err());
        assert!(flows.consume(&state, "google", "dev-1", 1).is_ok());
        assert!(
            flows.consume(&state, "google", "dev-1", 1).is_err(),
            "a flow is taken once"
        );

        // A flow ends after its time, and a device holds three.
        let late = flows.create(flow("dev-1"), 0).unwrap();
        assert!(flows.consume(&late, "google", "dev-1", 1000).is_err());
        let flows = Flows::default();
        for _ in 0..3 {
            flows.create(flow("dev-1"), 0).unwrap();
        }
        assert!(flows.create(flow("dev-1"), 0).is_err());
        assert!(flows.create(flow("dev-2"), 0).is_ok());
        assert!(
            flows.create(flow("dev-1"), 1000).is_ok(),
            "ended flows make room"
        );
    }

    #[test]
    fn an_address_starts_twenty_flows_and_completes_ten_in_a_minute() {
        let flows = Flows::default();
        for _ in 0..20 {
            flows.allow("192.0.2.1", true, 0).unwrap();
        }
        assert!(flows.allow("192.0.2.1", true, 0).is_err());
        assert!(flows.allow("192.0.2.2", true, 0).is_ok());
        for _ in 0..10 {
            flows.allow("192.0.2.1", false, 0).unwrap();
        }
        assert!(flows.allow("192.0.2.1", false, 59_999).is_err());
        assert!(flows.allow("192.0.2.1", false, 60_000).is_ok());
    }
}
