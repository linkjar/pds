//! The sessions of the console. They live in memory: the console has no
//! database, and a restart signs every operator out.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use pds_identity::Keypair;
use pds_sidecar::client::{Pending, Tokens, random_text};

/// How long an operator stays signed in: 12 hours.
pub const SESSION_LIFE: Duration = Duration::from_hours(12);

/// How long the browser has to come back from the PDS: 10 minutes.
pub const PENDING_LIFE: Duration = Duration::from_mins(10);

/// The most sessions, and the most authorizations in flight. One more
/// replaces the oldest.
pub const MOST: usize = 64;

/// An operator who is signed in.
#[derive(Debug)]
pub struct Session {
    /// The value of the cookie.
    pub id: String,
    /// The DPoP key of the session.
    pub key: Keypair,
    /// The account.
    pub did: String,
    /// The handle of the account, when the console could read it.
    pub handle: Option<String>,
    /// `operator` or `moderator`.
    pub role: String,
    /// The token of the forms.
    pub csrf: String,
    tokens: Mutex<Tokens>,
    started: Instant,
}

impl Session {
    /// The tokens in use.
    pub fn tokens(&self) -> Tokens {
        self.tokens
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Replaces the tokens after a refresh.
    pub fn set_tokens(&self, tokens: Tokens) {
        *self.tokens.lock().unwrap_or_else(PoisonError::into_inner) = tokens;
    }

    /// Whether the role may take the actions of an operator.
    pub fn is_operator(&self) -> bool {
        self.role == "operator"
    }
}

/// Every session and every authorization in flight.
#[derive(Debug, Default)]
pub struct Sessions {
    live: Mutex<HashMap<String, Arc<Session>>>,
    pending: Mutex<HashMap<String, (Pending, Instant)>>,
}

/// Drops what is older than `life`, and the oldest entry when the map is
/// full.
fn make_room<T>(map: &mut HashMap<String, T>, life: Duration, started: impl Fn(&T) -> Instant) {
    map.retain(|_, entry| started(entry).elapsed() < life);
    if map.len() >= MOST {
        let oldest = map
            .iter()
            .min_by_key(|(_, entry)| started(entry))
            .map(|(key, _)| key.clone());
        if let Some(oldest) = oldest {
            map.remove(&oldest);
        }
    }
}

impl Sessions {
    /// Keeps an authorization until the browser is back. Returns the
    /// `state` of the request.
    pub fn expect(&self, state: String, pending: Pending) {
        let mut map = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        make_room(&mut map, PENDING_LIFE, |(_, started)| *started);
        map.insert(state, (pending, Instant::now()));
    }

    /// Takes the authorization of a `state`. It is good once.
    pub fn take(&self, state: &str) -> Option<Pending> {
        let found = self
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(state);
        found
            .filter(|(_, started)| started.elapsed() < PENDING_LIFE)
            .map(|(pending, _)| pending)
    }

    /// Starts a session.
    pub fn start(
        &self,
        key: Keypair,
        tokens: Tokens,
        role: String,
        handle: Option<String>,
    ) -> Arc<Session> {
        let session = Arc::new(Session {
            id: random_text::<32>(),
            key,
            did: tokens.did.clone(),
            handle,
            role,
            csrf: random_text::<32>(),
            tokens: Mutex::new(tokens),
            started: Instant::now(),
        });
        let mut map = self.live.lock().unwrap_or_else(PoisonError::into_inner);
        make_room(&mut map, SESSION_LIFE, |session| session.started);
        map.insert(session.id.clone(), Arc::clone(&session));
        session
    }

    /// The session of a cookie value.
    pub fn get(&self, id: &str) -> Option<Arc<Session>> {
        let found = self
            .live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .cloned();
        found.filter(|session| session.started.elapsed() < SESSION_LIFE)
    }

    /// Ends a session.
    pub fn end(&self, id: &str) -> Option<Arc<Session>> {
        self.live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(id)
    }
}

#[cfg(test)]
mod tests {
    use pds_sidecar::client::new_key;

    use super::*;

    fn tokens() -> Tokens {
        Tokens {
            access: "access".to_owned(),
            refresh: Some("refresh".to_owned()),
            did: "did:plc:a".to_owned(),
        }
    }

    #[test]
    fn an_authorization_is_taken_once() {
        let sessions = Sessions::default();
        let pending = Pending {
            key: new_key(),
            verifier: "v".to_owned(),
        };
        sessions.expect("state-1".to_owned(), pending);
        assert!(sessions.take("state-2").is_none());
        assert_eq!(sessions.take("state-1").unwrap().verifier, "v");
        assert!(sessions.take("state-1").is_none());
    }

    #[test]
    fn a_session_is_found_until_it_ends_and_the_oldest_makes_room() {
        let sessions = Sessions::default();
        let first = sessions.start(new_key(), tokens(), "moderator".to_owned(), None);
        assert!(!first.is_operator());
        assert!(first.handle.is_none());
        assert_eq!(sessions.get(&first.id).unwrap().did, "did:plc:a");
        first.set_tokens(Tokens {
            access: "next".to_owned(),
            ..tokens()
        });
        assert_eq!(first.tokens().access, "next");
        for _ in 0..MOST {
            sessions.start(new_key(), tokens(), "operator".to_owned(), None);
        }
        assert!(sessions.get(&first.id).is_none());
        let last = sessions.start(
            new_key(),
            tokens(),
            "operator".to_owned(),
            Some("op.test".to_owned()),
        );
        assert_eq!(last.handle.as_deref(), Some("op.test"));
        assert!(sessions.end(&last.id).is_some());
        assert!(sessions.get(&last.id).is_none());
    }
}
