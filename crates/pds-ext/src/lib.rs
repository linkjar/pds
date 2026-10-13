//! Extension traits and the stock profile (`docs/SPEC.md` §12).
//!
//! The server core calls a [`Profile`] and holds no branch for an
//! operator (EX-1). [`Stock`] answers as the Stock Reference behaves
//! (EX-2). Built so far: the handle policy of §12.4.

use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;

/// A future that a trait method returns.
pub type Boxed<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// An identity provider as a page shows it (§12.3): never a secret.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderInfo {
    /// The name in a path and in a row, such as `google`.
    pub id: &'static str,
    /// The name on a button, such as `Google`.
    pub label: &'static str,
    /// Whether the provider answers with a form post in place of a
    /// redirect with a query.
    pub form_post: bool,
}

/// Who an identity provider says signed in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedIdentity {
    /// The provider's `id`.
    pub provider: &'static str,
    /// The provider's identifier of the person.
    pub subject: String,
    /// The email address the provider asserted.
    pub email: Option<String>,
    /// Whether the provider says the address is the person's.
    pub email_verified: bool,
    /// A name to suggest a handle from. It proves nothing.
    pub display_name: Option<String>,
}

/// What a browser brought back from a provider, and what the server kept
/// for it.
#[derive(Debug, Clone, Copy)]
pub struct ProviderAnswer<'a> {
    /// The callback of the flow.
    pub redirect_uri: &'a str,
    /// The code.
    pub code: &'a str,
    /// The PKCE verifier of the flow.
    pub verifier: &'a str,
    /// The nonce of the flow.
    pub nonce: &'a str,
    /// The `user` form field of a provider that sends one.
    pub user: Option<&'a str>,
    /// The time, in seconds.
    pub now_s: i64,
}

/// Requests to an identity provider. The server gives an implementation
/// that bounds the time and the size of an answer and follows no
/// redirect.
pub trait Fetch: Send + Sync {
    /// Posts a form and reads a JSON answer.
    fn post_form<'a>(
        &'a self,
        url: &'a str,
        form: &'a [(&'static str, String)],
    ) -> Boxed<'a, Result<serde_json::Value, &'static str>>;

    /// Gets a JSON document, with a bearer token when one is given.
    fn get_json<'a>(
        &'a self,
        url: &'a str,
        bearer: Option<&'a str>,
    ) -> Boxed<'a, Result<serde_json::Value, &'static str>>;
}

/// What the rename rule of a profile says of a change of handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rename {
    /// The rule does not apply to the change.
    Free,
    /// The change spends an allowance: the server records the target
    /// before it writes to the PLC directory.
    Claim,
    /// The change is refused, with this message.
    Refused(&'static str),
}

/// One attempt at the handle of a new account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Suggested {
    /// The label and the hosted domain to try.
    Handle(String),
    /// This attempt gave nothing usable; the next may.
    Skip,
    /// The profile suggests no handle, or no more.
    End,
}

/// The row a profile keeps for the handle of an account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandlePolicyRow {
    /// When the account was created, in milliseconds, when the row says
    /// so in a form the server can read.
    pub created_ms: Option<i64>,
    /// The target the account claimed with its allowance.
    pub claimed: Option<String>,
}

/// One operator's behaviour on top of the protocol.
pub trait Profile: Send + Sync + Debug {
    /// `stock` or `linkjar`.
    fn name(&self) -> &'static str;

    /// Whether the profile reserves `front` under the hosted `domain`, on
    /// top of the upstream list. An admin cannot lift this reservation.
    fn reserves(&self, _domain: &str, _front: &str) -> bool {
        false
    }

    /// Whether the profile keeps a handle policy row for accounts it
    /// creates through an identity provider.
    fn keeps_handle_policy(&self) -> bool {
        false
    }

    /// Decides a change of handle from `previous` to `handle`.
    fn rename(
        &self,
        _policy: Option<&HandlePolicyRow>,
        _previous: Option<&str>,
        _handle: &str,
        _now_ms: i64,
    ) -> Rename {
        Rename::Free
    }

    /// Whether the profile records which session created an account
    /// (§12.8), and serves `io.linkjar.account.getSignupReceipt` and
    /// `acknowledgeSignupReceipt`.
    fn keeps_signup_receipts(&self) -> bool {
        false
    }

    /// Whether changes of sign-in methods produce notices that the
    /// server mails (§12.6).
    fn produces_notices(&self) -> bool {
        false
    }

    /// The identity providers a person can sign in with (§12.3).
    fn providers(&self) -> Vec<ProviderInfo> {
        Vec::new()
    }

    /// The address a browser is sent to for a sign-in with `provider`.
    /// `None` means the profile has no such provider.
    fn provider_start(
        &self,
        _provider: &str,
        _redirect_uri: &str,
        _state: &str,
        _nonce: &str,
        _code_challenge: &str,
    ) -> Option<Result<String, &'static str>> {
        None
    }

    /// Exchanges the code a provider gave and checks who signed in. The
    /// error is for the log, not for the page.
    fn provider_exchange<'a>(
        &'a self,
        _provider: &'a str,
        _fetch: &'a dyn Fetch,
        _answer: ProviderAnswer<'a>,
    ) -> Boxed<'a, Result<VerifiedIdentity, &'static str>> {
        Box::pin(async { Err("The profile has no identity provider") })
    }

    /// The label of a hosted handle for a new account of an identity, for
    /// one attempt. The server asks with rising attempt numbers until it
    /// finds a label that is free.
    fn suggested_label(
        &self,
        _display_name: Option<&str>,
        _email: Option<&str>,
        _attempt: usize,
        _suffix: [u8; 3],
    ) -> Suggested {
        Suggested::End
    }

    /// Whether the server answers the host name of a hosted handle with
    /// the handle's identity and nothing else (§12.4).
    fn has_handle_hosts(&self) -> bool {
        false
    }

    /// Where a handle host sends a browser: the page of the account `did`
    /// that holds `handle`. `None` means the profile has no such page for
    /// the handle.
    fn profile_page(&self, _handle: &str, _did: &str) -> Option<String> {
        None
    }
}

/// The Stock Reference's behaviour: no extension.
#[derive(Debug, Clone, Copy, Default)]
pub struct Stock;

impl Profile for Stock {
    fn name(&self) -> &'static str {
        "stock"
    }
}
