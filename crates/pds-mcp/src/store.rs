//! What the MCP server keeps: clients, authorizations in flight, and
//! sessions. All of it is in memory, and a restart ends every session.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::pds::{PdsPending, PdsSession, hash, random_text};

/// How long an access token is good: one hour.
pub const ACCESS_LIFE: Duration = Duration::from_hours(1);
/// How long a session lasts without a refresh: 30 days.
pub const REFRESH_LIFE: Duration = Duration::from_hours(30 * 24);
/// How long a person has for the sign-in and the consent: 10 minutes.
pub const REQUEST_LIFE: Duration = Duration::from_mins(10);
/// How long a code is good: one minute.
pub const CODE_LIFE: Duration = Duration::from_mins(1);
/// The most entries of each table. One more replaces the oldest.
pub const MOST: usize = 4096;

/// How many rotated-away refresh tokens a session remembers. A second use
/// of one of them ends the session.
const SPENT_KEPT: usize = 64;
/// Calls of a session in a minute, and writes among them.
pub const CALLS_PER_MINUTE: u32 = 120;
/// Writes of a session in a minute.
pub const WRITES_PER_MINUTE: u32 = 20;

/// The scopes the server knows, apart from `repo:write:<collection>` for
/// the collections of the configuration.
pub const SCOPES: &[&str] = &["account:read", "repo:read", "repo:write"];

/// The scopes of a request without a `scope`.
pub const DEFAULT_SCOPE: &str = "account:read repo:read";

pub use pds_oauth::scope::is_nsid;

/// The scopes the server asks of the PDS for the scopes of an AI client
/// (SPEC §22.2). Reads need the identity only.
pub fn pds_scope(scopes: &[String]) -> String {
    let mut out = "atproto".to_owned();
    if scopes.iter().any(|scope| scope == "repo:write") {
        out.push_str(" repo:*");
        return out;
    }
    for collection in scopes
        .iter()
        .filter_map(|scope| scope.strip_prefix("repo:write:"))
    {
        out.push_str(" repo:");
        out.push_str(collection);
    }
    out
}

/// An AI client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpClient {
    /// Its identifier: a registration of this server, or the address of
    /// its metadata document.
    pub id: String,
    /// The name it gives itself.
    pub name: Option<String>,
    /// Where it takes the answer of an authorization.
    pub redirect_uris: Vec<String>,
}

/// An authorization in flight.
#[derive(Debug)]
pub struct Request {
    /// The AI client.
    pub client: McpClient,
    /// The redirect URI of this request.
    pub redirect_uri: String,
    /// The `state` of the AI client.
    pub state: Option<String>,
    /// The PKCE challenge of the AI client.
    pub challenge: String,
    /// The scopes it asks for.
    pub scopes: Vec<String>,
    /// The hash of the cookie of the browser that started the request.
    pub browser: String,
    /// The token of the consent form.
    pub csrf: String,
    /// The authorization at the PDS, until the browser is back.
    pub pending: Option<PdsPending>,
    /// The session at the PDS, from then until the person decides.
    pub pds: Option<PdsSession>,
    started: Instant,
}

impl Request {
    /// A request that the browser `browser` starts.
    pub fn new(
        client: McpClient,
        redirect_uri: String,
        state: Option<String>,
        challenge: String,
        scopes: Vec<String>,
        browser: &str,
    ) -> Self {
        Self {
            client,
            redirect_uri,
            state,
            challenge,
            scopes,
            browser: hash(browser),
            csrf: random_text::<24>(),
            pending: None,
            pds: None,
            started: Instant::now(),
        }
    }
}

/// A code that the token endpoint exchanges once.
#[derive(Debug)]
pub struct Code {
    /// The AI client.
    pub client_id: String,
    /// The redirect URI of the request.
    pub redirect_uri: String,
    /// The PKCE challenge.
    pub challenge: String,
    /// The scopes the person allowed.
    pub scopes: Vec<String>,
    /// The session at the PDS.
    pub pds: PdsSession,
    made: Instant,
}

/// A count of calls in the running minute.
#[derive(Debug)]
struct Window {
    since: Instant,
    calls: u32,
    writes: u32,
}

/// A session of an AI client.
#[derive(Debug)]
pub struct Session {
    /// The AI client.
    pub client_id: String,
    /// The scopes.
    pub scopes: Vec<String>,
    /// The session at the PDS.
    pub pds: PdsSession,
    window: Mutex<Window>,
}

impl Session {
    /// Whether the session holds `scope`.
    pub fn has(&self, scope: &str) -> bool {
        self.scopes.iter().any(|held| held == scope)
    }

    /// Whether the session may write records of `collection`.
    pub fn may_write(&self, collection: &str) -> bool {
        self.has("repo:write") || self.has(&format!("repo:write:{collection}"))
    }

    /// Counts a call. `false` when the session made too many in this
    /// minute (`docs/sidecars.md` §2.6).
    pub fn spend(&self, write: bool) -> bool {
        let mut window = self.window.lock().unwrap_or_else(PoisonError::into_inner);
        if window.since.elapsed() >= Duration::from_mins(1) {
            *window = Window {
                since: Instant::now(),
                calls: 0,
                writes: 0,
            };
        }
        if window.calls >= CALLS_PER_MINUTE || (write && window.writes >= WRITES_PER_MINUTE) {
            return false;
        }
        window.calls += 1;
        window.writes += u32::from(write);
        true
    }
}

/// The tokens of a session, as the token endpoint gives them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issued {
    /// The bearer access token.
    pub access: String,
    /// The refresh token.
    pub refresh: String,
    /// The scopes, separated by spaces.
    pub scope: String,
}

#[derive(Debug)]
struct Live {
    session: Arc<Session>,
    access: String,
    access_until: Instant,
    refresh: String,
    refresh_until: Instant,
    /// The hashes of the refresh tokens rotated away, oldest first.
    spent: Vec<String>,
}

#[derive(Debug, Default)]
struct Sessions {
    /// By the hash of the refresh token in use.
    live: HashMap<String, Live>,
    /// The hash of each access token in use, to the refresh hash.
    by_access: HashMap<String, String>,
    /// The hash of each refresh token that was rotated away, to the hash
    /// of the one in use. A second use ends the session.
    spent: HashMap<String, String>,
}

/// Every table of the server.
#[derive(Debug, Default)]
pub struct Store {
    clients: Mutex<HashMap<String, (McpClient, Instant)>>,
    requests: Mutex<HashMap<String, Request>>,
    codes: Mutex<HashMap<String, Code>>,
    sessions: Mutex<Sessions>,
}

/// Drops the ended rows of a full table, and its oldest row when every
/// row is live. `true` when a row went.
fn make_room<T>(
    map: &mut HashMap<String, T>,
    life: Duration,
    started: impl Fn(&T) -> Instant,
) -> bool {
    if map.len() < MOST {
        return false;
    }
    let before = map.len();
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
    map.len() < before
}

fn locked<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Store {
    /// Registers an AI client and gives it an identifier.
    pub fn register(&self, name: Option<String>, redirect_uris: Vec<String>) -> McpClient {
        let client = McpClient {
            id: format!("mcp-client-{}", random_text::<16>()),
            name,
            redirect_uris,
        };
        let mut clients = locked(&self.clients);
        make_room(&mut clients, Duration::MAX, |(_, made)| *made);
        clients.insert(client.id.clone(), (client.clone(), Instant::now()));
        client
    }

    /// A registered client.
    pub fn client(&self, id: &str) -> Option<McpClient> {
        locked(&self.clients)
            .get(id)
            .map(|(client, _)| client.clone())
    }

    /// Takes a request out. It goes back with [`Store::put_back`] when
    /// the flow goes on.
    pub fn take(&self, id: &str) -> Option<Request> {
        locked(&self.requests)
            .remove(id)
            .filter(|request| request.started.elapsed() < REQUEST_LIFE)
    }

    /// Keeps a request under its identifier.
    pub fn put_back(&self, id: &str, request: Request) {
        let mut requests = locked(&self.requests);
        make_room(&mut requests, REQUEST_LIFE, |request| request.started);
        requests.insert(id.to_owned(), request);
    }

    /// Makes the code of an allowed request.
    pub fn grant(&self, request: Request, pds: PdsSession) -> String {
        let code = random_text::<32>();
        let mut codes = locked(&self.codes);
        make_room(&mut codes, CODE_LIFE, |code| code.made);
        codes.insert(
            hash(&code),
            Code {
                client_id: request.client.id,
                redirect_uri: request.redirect_uri,
                challenge: request.challenge,
                scopes: request.scopes,
                pds,
                made: Instant::now(),
            },
        );
        code
    }

    /// Takes a code. It is good once and for one minute.
    pub fn redeem(&self, code: &str) -> Option<Code> {
        locked(&self.codes)
            .remove(&hash(code))
            .filter(|code| code.made.elapsed() < CODE_LIFE)
    }

    /// Starts a session and gives its tokens.
    pub fn start(&self, code: Code) -> Issued {
        let issued = Issued {
            access: random_text::<32>(),
            refresh: random_text::<32>(),
            scope: code.scopes.join(" "),
        };
        let session = Arc::new(Session {
            client_id: code.client_id,
            scopes: code.scopes,
            pds: code.pds,
            window: Mutex::new(Window {
                since: Instant::now(),
                calls: 0,
                writes: 0,
            }),
        });
        let now = Instant::now();
        let mut sessions = locked(&self.sessions);
        let Sessions {
            live,
            by_access,
            spent,
        } = &mut *sessions;
        if make_room(live, REFRESH_LIFE, |row| {
            row.refresh_until
                .checked_sub(REFRESH_LIFE)
                .unwrap_or(row.refresh_until)
        }) {
            // The rows of the sessions that went.
            by_access.retain(|_, refresh| live.contains_key(refresh));
            spent.retain(|_, refresh| live.contains_key(refresh));
        }
        let (access, refresh) = (hash(&issued.access), hash(&issued.refresh));
        by_access.insert(access.clone(), refresh.clone());
        live.insert(
            refresh.clone(),
            Live {
                session,
                access,
                access_until: now + ACCESS_LIFE,
                refresh,
                refresh_until: now + REFRESH_LIFE,
                spent: Vec::new(),
            },
        );
        drop(sessions);
        issued
    }

    /// The session of an access token.
    pub fn session(&self, access: &str) -> Option<Arc<Session>> {
        let sessions = locked(&self.sessions);
        let refresh = sessions.by_access.get(&hash(access))?;
        sessions
            .live
            .get(refresh)
            .filter(|live| Instant::now() < live.access_until)
            .map(|live| Arc::clone(&live.session))
    }

    /// Rotates the tokens of a session of `client_id`. `Err` gives the
    /// session to end at the PDS when a refresh token came a second time.
    pub fn rotate(&self, refresh: &str, client_id: &str) -> Result<Option<Issued>, Arc<Session>> {
        let given = hash(refresh);
        let mut sessions = locked(&self.sessions);
        if let Some(current) = sessions.spent.remove(&given) {
            // A refresh token that was rotated away: someone copied it.
            return match sessions.live.remove(&current) {
                Some(live) => {
                    sessions.by_access.remove(&live.access);
                    for old in &live.spent {
                        sessions.spent.remove(old);
                    }
                    Err(live.session)
                }
                None => Ok(None),
            };
        }
        let Some(mut live) = sessions.live.remove(&given) else {
            return Ok(None);
        };
        if live.session.client_id != client_id || Instant::now() >= live.refresh_until {
            sessions.live.insert(given, live);
            return Ok(None);
        }
        let issued = Issued {
            access: random_text::<32>(),
            refresh: random_text::<32>(),
            scope: live.session.scopes.join(" "),
        };
        let now = Instant::now();
        sessions.by_access.remove(&live.access);
        live.access = hash(&issued.access);
        live.access_until = now + ACCESS_LIFE;
        live.refresh = hash(&issued.refresh);
        live.refresh_until = now + REFRESH_LIFE;
        // The tokens that were rotated away point at the one in use; a
        // session remembers the last `SPENT_KEPT` of them.
        live.spent.push(given);
        if live.spent.len() > SPENT_KEPT {
            sessions.spent.remove(&live.spent.remove(0));
        }
        for old in &live.spent {
            sessions.spent.insert(old.clone(), live.refresh.clone());
        }
        sessions
            .by_access
            .insert(live.access.clone(), live.refresh.clone());
        sessions.live.insert(live.refresh.clone(), live);
        drop(sessions);
        Ok(Some(issued))
    }

    /// Ends the session of an access token or of a refresh token.
    pub fn end(&self, token: &str) -> Option<Arc<Session>> {
        let given = hash(token);
        let mut sessions = locked(&self.sessions);
        let refresh = sessions.by_access.get(&given).cloned().unwrap_or(given);
        let live = sessions.live.remove(&refresh)?;
        sessions.by_access.remove(&live.access);
        for old in &live.spent {
            sessions.spent.remove(old);
        }
        drop(sessions);
        Some(live.session)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A session at the PDS that no test sends anywhere.
    fn pds_session() -> PdsSession {
        crate::pds::tests::session("did:plc:a")
    }

    fn code(scopes: &[&str]) -> Code {
        Code {
            client_id: "client-1".to_owned(),
            redirect_uri: "https://app.example/cb".to_owned(),
            challenge: "c".to_owned(),
            scopes: scopes.iter().map(|scope| (*scope).to_owned()).collect(),
            pds: pds_session(),
            made: Instant::now(),
        }
    }

    #[test]
    fn scopes_map_to_the_scopes_of_the_pds() {
        let scopes = |list: &[&str]| -> Vec<String> {
            list.iter().map(|scope| (*scope).to_owned()).collect()
        };
        assert_eq!(
            pds_scope(&scopes(&["account:read", "repo:read"])),
            "atproto"
        );
        assert_eq!(pds_scope(&scopes(&["repo:write"])), "atproto repo:*");
        assert_eq!(
            pds_scope(&scopes(&["repo:read", "repo:write:com.example.note"])),
            "atproto repo:com.example.note"
        );
        assert!(is_nsid("com.example.note"));
        for bad in ["", "note", "a..b", "com.example", "com.exa mple.note"] {
            assert!(!is_nsid(bad), "{bad}");
        }
    }

    #[test]
    fn a_session_is_found_by_its_access_token_and_rotated_by_its_refresh_token() {
        let store = Store::default();
        let first = store.start(code(&["repo:read", "repo:write:com.example.note"]));
        let session = store.session(&first.access).unwrap();
        assert!(session.may_write("com.example.note"));
        assert!(!session.may_write("com.example.other"));
        assert!(store.session(&first.refresh).is_none());

        // Another client cannot use the refresh token.
        assert_eq!(store.rotate(&first.refresh, "client-2").unwrap(), None);
        let second = store.rotate(&first.refresh, "client-1").unwrap().unwrap();
        assert!(store.session(&first.access).is_none());
        assert!(store.session(&second.access).is_some());
        let third = store.rotate(&second.refresh, "client-1").unwrap().unwrap();

        // A refresh token that was rotated away ends the session.
        assert!(store.rotate(&first.refresh, "client-1").is_err());
        assert!(store.session(&third.access).is_none());
        assert_eq!(store.rotate(&third.refresh, "client-1").unwrap(), None);
    }

    #[test]
    fn a_session_ends_by_either_token_and_a_code_is_good_once() {
        let store = Store::default();
        let issued = store.start(code(&["repo:read"]));
        assert!(store.end(&issued.refresh).is_some());
        assert!(store.session(&issued.access).is_none());
        let issued = store.start(code(&["repo:read"]));
        assert!(store.end(&issued.access).is_some());
        assert!(store.end(&issued.access).is_none());

        let client = store.register(None, vec!["https://app.example/cb".to_owned()]);
        assert_eq!(store.client(&client.id).unwrap(), client);
        let request = Request::new(
            client,
            "https://app.example/cb".to_owned(),
            None,
            "c".to_owned(),
            vec![],
            "b",
        );
        let made = store.grant(request, pds_session());
        assert!(store.redeem(&made).is_some());
        assert!(store.redeem(&made).is_none());
    }

    #[test]
    fn a_session_has_a_budget_of_calls_and_of_writes() {
        let store = Store::default();
        let issued = store.start(code(&["repo:write"]));
        let session = store.session(&issued.access).unwrap();
        for _ in 0..WRITES_PER_MINUTE {
            assert!(session.spend(true));
        }
        assert!(!session.spend(true));
        for _ in WRITES_PER_MINUTE..CALLS_PER_MINUTE {
            assert!(session.spend(false));
        }
        assert!(!session.spend(false));
    }
}
