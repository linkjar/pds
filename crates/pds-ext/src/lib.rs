//! Extension traits and the stock profile (`docs/SPEC.md` §12).
//!
//! The server core calls a [`Profile`] and holds no branch for an
//! operator (EX-1). [`Stock`] answers as the Stock Reference behaves
//! (EX-2). Built so far: the handle policy of §12.4.

use std::fmt::Debug;

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
