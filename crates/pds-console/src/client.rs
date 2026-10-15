//! The console as a client of the PDS (`docs/sidecars.md` §1.4).
//!
//! It is an OAuth client, and a client of the ops listener. The console
//! holds no password: an operator signs in on the pages of the PDS, and
//! the console keeps the session with the DPoP key it made for it.

use pds_identity::Keypair;
use pds_sidecar::client::{ClientError, DpopClient, Peer, Pending, Tokens};
use serde_json::Value;

use crate::config::{Config, SCOPE};

/// A request to the ops listener.
#[derive(Debug, Clone, Copy)]
pub struct Call<'a> {
    /// The path with its query, such as `/xrpc/internal.ops.getOverview`.
    pub path: &'a str,
    /// The reason of an action.
    pub reason: Option<&'a str>,
    /// The body of an action. `None` makes a GET.
    pub body: Option<&'a Value>,
}

impl<'a> Call<'a> {
    /// A read.
    pub const fn get(path: &'a str) -> Self {
        Self {
            path,
            reason: None,
            body: None,
        }
    }

    /// An action with its reason.
    pub const fn post(path: &'a str, reason: &'a str, body: &'a Value) -> Self {
        Self {
            path,
            reason: Some(reason),
            body: Some(body),
        }
    }
}

/// The connection of the console to the PDS.
#[derive(Debug)]
pub struct Client {
    config: Config,
    dpop: DpopClient,
}

impl Client {
    /// A client for the configuration. Requests end after 15 seconds and
    /// follow no redirect.
    pub fn new(config: Config) -> Result<Self, ClientError> {
        let dpop = DpopClient::new(Peer {
            issuer: config.pds_public_url.clone(),
            internal_url: config.pds_internal_url.clone(),
            client_id: config.client_id(),
            redirect_uri: config.redirect_uri(),
        })?;
        Ok(Self { config, dpop })
    }

    /// The configuration.
    pub const fn config(&self) -> &Config {
        &self.config
    }

    /// Pushes an authorization request. Gives the address to send the
    /// browser to, and what the console keeps until the browser is back.
    pub async fn start(&self, state: &str) -> Result<(String, Pending), ClientError> {
        self.dpop.start(state, SCOPE, None).await
    }

    /// Exchanges the code of the redirect for a session.
    pub async fn finish(&self, pending: &Pending, code: &str) -> Result<Tokens, ClientError> {
        self.dpop.finish(pending, code, None).await
    }

    /// Takes new tokens for a session.
    pub async fn refresh(&self, key: &Keypair, refresh: &str) -> Result<Tokens, ClientError> {
        self.dpop.refresh(key, refresh, None).await
    }

    /// Ends a session at the PDS. A failure is not reported: the console
    /// forgets the session in every case.
    pub async fn revoke(&self, key: &Keypair, token: &str) {
        self.dpop.revoke(key, token, None).await;
    }

    /// Calls the ops listener with the session of an operator.
    pub async fn ops(
        &self,
        key: &Keypair,
        token: &str,
        call: Call<'_>,
    ) -> Result<Value, ClientError> {
        let method = if call.body.is_some() {
            reqwest::Method::POST
        } else {
            reqwest::Method::GET
        };
        let (status, answer) = self
            .dpop
            .send(
                key,
                &method,
                &self.config.ops_url,
                call.path,
                Some(token),
                |mut request| {
                    if let Some(reason) = call.reason {
                        request = request.header("x-ops-reason", reason);
                    }
                    if let Some(body) = call.body {
                        request = request
                            .header("content-type", "application/json")
                            .body(body.to_string());
                    }
                    request
                },
            )
            .await?;
        if (200..300).contains(&status) {
            Ok(answer)
        } else {
            Err(DpopClient::refused(status, &answer))
        }
    }
}
