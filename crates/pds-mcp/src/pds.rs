//! The MCP server as a confidential OAuth client of the PDS
//! (`docs/SPEC.md` §22.2).
//!
//! It authenticates with a `private_key_jwt` assertion and holds a DPoP
//! key for each session. A token of an AI client never reaches the PDS,
//! and a token of the PDS never reaches an AI client.

use std::sync::{Mutex, PoisonError};

use pds_identity::Keypair;
use pds_sidecar::client::{ClientError, ClientKey, DpopClient, Peer, Pending, Tokens};
use serde_json::Value;

use crate::config::Config;

/// The header that names the AI client of a write to the PDS, for its
/// audit log.
pub const MCP_CLIENT_HEADER: &str = "x-mcp-client-id";

/// A session at the PDS.
#[derive(Debug)]
pub struct PdsSession {
    /// The DPoP key of the session.
    pub key: Keypair,
    /// The account.
    pub did: String,
    tokens: Mutex<(String, Option<String>)>,
}

impl PdsSession {
    fn tokens(&self) -> (String, Option<String>) {
        self.tokens
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// A call of an XRPC method.
#[derive(Debug, Clone, Copy)]
pub struct Xrpc<'a> {
    /// The NSID.
    pub nsid: &'a str,
    /// The parameters of a query.
    pub params: &'a [(&'a str, &'a str)],
    /// The input of a procedure. `None` makes a GET.
    pub input: Option<&'a Value>,
    /// The AI client, for the audit log of the PDS.
    pub ai_client: &'a str,
}

/// The connection of the server to the PDS.
#[derive(Debug)]
pub struct Pds {
    config: Config,
    /// The name of the client key in the key set of the server.
    kid: String,
    dpop: DpopClient,
}

impl Pds {
    /// A connection for the configuration. Requests end after 15 seconds
    /// and follow no redirect.
    pub fn new(config: Config) -> Result<Self, ClientError> {
        let dpop = DpopClient::new(Peer {
            issuer: config.pds_public_url.clone(),
            internal_url: config.pds_internal_url.clone(),
            client_id: config.client_id(),
            redirect_uri: config.redirect_uri(),
        })?;
        Ok(Self {
            kid: config.key_id(),
            config,
            dpop,
        })
    }

    /// The configuration.
    pub const fn config(&self) -> &Config {
        &self.config
    }

    /// The key that signs the assertion of the server at the authorization
    /// server.
    fn assertion(&self) -> ClientKey<'_> {
        ClientKey {
            key: &self.config.client_key,
            kid: &self.kid,
        }
    }

    /// Pushes an authorization request for `scope`. Gives the address to
    /// send the browser to, and what the server keeps until it is back.
    pub async fn start(&self, state: &str, scope: &str) -> Result<(String, Pending), ClientError> {
        self.dpop.start(state, scope, Some(self.assertion())).await
    }

    /// Exchanges the code of the redirect for a session.
    pub async fn finish(&self, pending: Pending, code: &str) -> Result<PdsSession, ClientError> {
        let Tokens {
            access,
            refresh,
            did,
        } = self
            .dpop
            .finish(&pending, code, Some(self.assertion()))
            .await?;
        Ok(PdsSession {
            key: pending.key,
            did,
            tokens: Mutex::new((access, refresh)),
        })
    }

    /// Ends a session at the PDS. A failure is not reported: the server
    /// forgets the session in every case.
    pub async fn revoke(&self, session: &PdsSession) {
        let (access, _) = session.tokens();
        self.dpop
            .revoke(&session.key, &access, Some(self.assertion()))
            .await;
    }

    async fn refresh(&self, session: &PdsSession, refresh: &str) -> Result<String, ClientError> {
        let tokens = self
            .dpop
            .refresh(&session.key, refresh, Some(self.assertion()))
            .await?;
        *session
            .tokens
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = (tokens.access.clone(), tokens.refresh);
        Ok(tokens.access)
    }

    async fn once(
        &self,
        session: &PdsSession,
        access: &str,
        call: Xrpc<'_>,
    ) -> Result<(u16, Value), ClientError> {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(call.params)
            .finish();
        let mut path = format!("/xrpc/{}", call.nsid);
        if !query.is_empty() {
            path.push('?');
            path.push_str(&query);
        }
        let method = if call.input.is_some() {
            reqwest::Method::POST
        } else {
            reqwest::Method::GET
        };
        self.dpop
            .send(
                &session.key,
                &method,
                &self.config.pds_internal_url,
                &path,
                Some(access),
                |mut request| {
                    request = request.header(MCP_CLIENT_HEADER, call.ai_client);
                    if let Some(input) = call.input {
                        request = request
                            .header("content-type", "application/json")
                            .body(input.to_string());
                    }
                    request
                },
            )
            .await
    }

    /// Calls a method of the PDS with a session. When the PDS no longer
    /// takes the access token, the server refreshes the session once.
    pub async fn xrpc(&self, session: &PdsSession, call: Xrpc<'_>) -> Result<Value, ClientError> {
        let (access, refresh) = session.tokens();
        let (mut status, mut answer) = self.once(session, &access, call).await?;
        if status == 401
            && let Some(refresh) = refresh
        {
            let access = self.refresh(session, &refresh).await?;
            (status, answer) = self.once(session, &access, call).await?;
        }
        if (200..300).contains(&status) {
            Ok(answer)
        } else {
            Err(DpopClient::refused(status, &answer))
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use pds_identity::Algorithm;

    use super::*;

    /// A session of `did` with tokens that no PDS issued.
    pub fn session(did: &str) -> PdsSession {
        PdsSession {
            key: Keypair::generate(Algorithm::P256, |bytes| bytes.fill(9)),
            did: did.to_owned(),
            tokens: Mutex::new(("access".to_owned(), None)),
        }
    }
}
