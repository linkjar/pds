//! The network of the simulation (`docs/SPEC.md` §19.2).
//!
//! Three hosts run in one process on `turmoil`'s network: the server, a
//! relay that follows the firehose over a WebSocket and checks every
//! commit strictly, and a client that writes records over HTTP. A plan
//! drawn from the seed partitions the link between the relay and the
//! server, repairs it, and holds its messages so that the relay falls
//! behind. At the end the link is whole, and the run requires:
//!
//! - the relay received every event of the log once, in order, over all
//!   of its connections;
//! - the relay's copy of each repository is the server's, with `#sync` as
//!   its only reason to fetch a repository again;
//! - each repository holds the records the client was told it wrote.
//!
//! # What this does not model
//!
//! - **Time on the server's blocking pool.** The server does its storage
//!   work on threads that run in real time, while the network's clock is
//!   simulated. The plan and the final state are functions of the seed.
//!   The interleaving of frames and writes is not, so the trace holds the
//!   plan and no count of connections.
//! - **The crawler notification, identity resolution and mail.** They use
//!   an HTTP client that connects to the host's network.
//! - **A restart of the server**, and a fault on the link of the writing
//!   client.

// The hosts of the network run on one thread, and what they share is
// held for a few lines at a time.
#![allow(clippy::future_not_send, clippy::significant_drop_tightening)]

use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use bytes::Bytes;
use cid::Cid;
use futures_util::StreamExt as _;
use http_body_util::{BodyExt as _, Full};
use hyper_util::rt::TokioIo;
use hyper_util::service::TowerToHyperService;
use pds_repo::repo::{Expect, verify_repo_car};
use pds_storage::ActorDb;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use turmoil::net::{TcpListener, TcpStream};

use crate::model::{Event, Next, Relay, error_of};
use crate::rng::Rng;
use crate::world::{self, World};
use crate::{Failure, ONE_RUN};

const SERVER: &str = "pds";
const RELAY: &str = "relay";
const CLIENT: &str = "client";
const COLLECTION: &str = "com.example.sim.note";
const KEYS: usize = 10;
/// The relay opens a new connection when no frame came for this long. The
/// server sends no ping, so a link that drops everything looks the same as
/// a quiet log.
const IDLE: Duration = Duration::from_secs(2);

/// The size of a network run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetOptions {
    /// How many accounts the data directory has.
    pub accounts: usize,
    /// How many operations the plan has.
    pub steps: usize,
    /// The subscriber buffer of the server, in events.
    pub buffer: u32,
}

impl Default for NetOptions {
    fn default() -> Self {
        Self {
            accounts: 3,
            steps: 120,
            buffer: 20,
        }
    }
}

/// What a network run did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NetOutcome {
    /// One line for each operation of the plan, the same for every run of
    /// the seed.
    pub trace: Vec<String>,
    /// Commits the server acknowledged.
    pub commits: u32,
    /// Events in the log at the end. The relay received each of them once.
    pub events: i64,
    /// Partitions of the link between the relay and the server.
    pub partitions: u32,
    /// Times the link held its messages.
    pub holds: u32,
    /// Connections the relay opened. Not a function of the seed.
    pub connections: u32,
    /// Times the server closed the relay's connection as too slow. Not a
    /// function of the seed.
    pub too_slow: u32,
}

#[derive(Debug, Default)]
struct Shared {
    relay: Relay,
    cursor: i64,
    received: i64,
    connections: u32,
    too_slow: u32,
    resyncs: u32,
    /// The plan is done and the link is whole.
    settled: bool,
    /// The relay has the whole log.
    done: bool,
    trace: Vec<String>,
    commits: u32,
    partitions: u32,
    holds: u32,
    /// The records the client was told it wrote, by account and key.
    expected: BTreeMap<String, BTreeMap<String, String>>,
}

type Share = Arc<Mutex<Shared>>;
type Halt = Box<dyn std::error::Error>;

fn locked(share: &Share) -> std::sync::MutexGuard<'_, Shared> {
    share.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One HTTP request to the server, on a connection of its own.
async fn request(
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<String>,
) -> Result<(u16, Bytes), Halt> {
    let stream = TcpStream::connect((SERVER, 80)).await?;
    let (mut sender, connection) =
        hyper::client::conn::http1::handshake(TokioIo::new(stream)).await?;
    tokio::spawn(connection);
    let mut builder = hyper::Request::builder()
        .method(method)
        .uri(path)
        .header("host", SERVER)
        .header("connection", "close");
    if let Some(token) = token {
        builder = builder.header("authorization", token);
    }
    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    let request = builder.body(Full::new(Bytes::from(body.unwrap_or_default())))?;
    let response = sender.send_request(request).await?;
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await?.to_bytes();
    Ok((status, bytes))
}

/// The server: every connection is served by the router of production.
async fn serve(router: axum::Router) -> Result<(), Halt> {
    let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 80)).await?;
    loop {
        let (stream, _) = listener.accept().await?;
        let service = TowerToHyperService::new(router.clone());
        tokio::spawn(async move {
            // A connection that the network broke ends here.
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .with_upgrades()
                .await;
        });
    }
}

/// Fetches a repository as a relay does after `#sync`.
async fn fetch(did: &str) -> Result<(String, Cid, BTreeMap<String, Cid>), Halt> {
    let (status, car) = request(
        "GET",
        &format!("/xrpc/com.atproto.sync.getRepo?did={did}"),
        None,
        None,
    )
    .await?;
    if status != 200 {
        return Err(format!("getRepo of {did} answered {status}").into());
    }
    let expect = Expect {
        did: None,
        verify: None,
    };
    let repo = verify_repo_car(&car, &expect, false)
        .map_err(|error| format!("getRepo of {did}: {error}"))?;
    Ok((
        repo.commit.rev.as_str().to_owned(),
        repo.commit.data,
        repo.records.into_iter().collect(),
    ))
}

/// One connection of the relay, from its cursor until the connection ends.
async fn follow(share: &Share, dids: &[String]) -> Result<(), Halt> {
    let cursor = locked(share).cursor;
    let stream = tokio::time::timeout(IDLE, TcpStream::connect((SERVER, 80))).await??;
    let url = format!("ws://{SERVER}/xrpc/com.atproto.sync.subscribeRepos?cursor={cursor}");
    let (mut socket, _) =
        tokio::time::timeout(IDLE, tokio_tungstenite::client_async(url, stream)).await??;
    locked(share).connections += 1;
    loop {
        let Ok(Some(message)) = tokio::time::timeout(IDLE, socket.next()).await else {
            return Ok(());
        };
        let Message::Binary(frame) = message? else {
            continue;
        };
        if let Some(error) = error_of(&frame) {
            if error == "ConsumerTooSlow" {
                locked(share).too_slow += 1;
                return Ok(());
            }
            return Err(format!("the relay was refused: {error}").into());
        }
        let event = Event::decode(&frame)?;
        let seq = event.seq()?;
        let key_of = |did: &str| {
            dids.iter()
                .position(|known| known == did)
                .map(|n| world::key(n).public_key())
        };
        let next = {
            let mut shared = locked(share);
            if seq <= shared.cursor {
                return Err(format!("event {seq} after {}", shared.cursor).into());
            }
            let next = shared.relay.apply(&event, key_of)?;
            // A `#sync` counts as received when the repository is fetched:
            // a fetch that the network breaks leaves the cursor before it.
            if next == Next::Continue {
                shared.cursor = seq;
                shared.received += 1;
            }
            next
        };
        if let Next::Resync(did) = next {
            let (rev, data, records) = tokio::time::timeout(IDLE, fetch(&did)).await??;
            let mut shared = locked(share);
            // A `#sync` repairs when the relay had dropped a commit. The
            // one that ends an account's creation repairs nothing.
            if shared
                .relay
                .repos
                .get(&did)
                .is_some_and(|repo| repo.desynced)
            {
                shared.resyncs += 1;
            }
            shared.relay.resync(&did, rev, data, records);
            shared.cursor = seq;
            shared.received += 1;
        }
    }
}

/// The relay: it follows the log, and connects again with its cursor when
/// a connection ends.
async fn relay(share: Share, dids: Vec<String>, head: impl Fn() -> i64) -> Result<(), Halt> {
    loop {
        // A connection that cannot be opened is a partition.
        let result = follow(&share, &dids).await;
        if let Err(error) = result {
            let text = error.to_string();
            let network = error.is::<std::io::Error>()
                || error.is::<tokio::time::error::Elapsed>()
                || error.is::<tokio_tungstenite::tungstenite::Error>();
            if !network {
                return Err(text.into());
            }
        }
        {
            let mut shared = locked(&share);
            if shared.settled && shared.cursor >= head() {
                shared.done = true;
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The client: it runs the plan of the seed.
async fn client(share: Share, dids: Vec<String>, seed: u64, steps: usize) -> Result<(), Halt> {
    let mut rng = Rng::new(seed);
    let (mut parted, mut held) = (false, false);
    for step in 0..steps {
        let line = match rng.index(100) {
            0..=7 => {
                parted = !parted;
                if parted {
                    turmoil::partition(RELAY, SERVER);
                    locked(&share).partitions += 1;
                    "partition the relay from the server".to_owned()
                } else {
                    turmoil::repair(RELAY, SERVER);
                    "repair the link of the relay".to_owned()
                }
            }
            8..=15 => {
                held = !held;
                if held {
                    turmoil::hold(RELAY, SERVER);
                    locked(&share).holds += 1;
                    "hold the messages of the relay's link".to_owned()
                } else {
                    turmoil::release(RELAY, SERVER);
                    "release the messages of the relay's link".to_owned()
                }
            }
            16..=25 => {
                let pause = 100 * (1 + rng.index(30) as u64);
                tokio::time::sleep(Duration::from_millis(pause)).await;
                format!("wait {pause} ms")
            }
            draw => {
                let did = rng.pick(&dids).clone();
                let rkey = format!("k{}", rng.index(KEYS));
                let delete = draw >= 90;
                let token = world::token(&did);
                let (nsid, body) = if delete {
                    (
                        "com.atproto.repo.deleteRecord",
                        json!({"repo": did, "collection": COLLECTION, "rkey": rkey}),
                    )
                } else {
                    (
                        "com.atproto.repo.putRecord",
                        json!({"repo": did, "collection": COLLECTION, "rkey": rkey,
                               "record": {"$type": COLLECTION, "n": step}}),
                    )
                };
                let (status, answer) = request(
                    "POST",
                    &format!("/xrpc/{nsid}"),
                    Some(&token),
                    Some(body.to_string()),
                )
                .await?;
                if status != 200 {
                    return Err(format!(
                        "{nsid} answered {status}: {}",
                        String::from_utf8_lossy(&answer)
                    )
                    .into());
                }
                let answer: Value = serde_json::from_slice(&answer)?;
                let mut shared = locked(&share);
                if answer.get("commit").is_some() {
                    shared.commits += 1;
                }
                let records = shared.expected.entry(did.clone()).or_default();
                let path = format!("{COLLECTION}/{rkey}");
                if delete {
                    records.remove(&path);
                    format!("delete {did} {rkey}")
                } else {
                    let cid = answer["cid"].as_str().unwrap_or_default().to_owned();
                    records.insert(path, cid.clone());
                    format!("put {did} {rkey} -> {cid}")
                }
            }
        };
        locked(&share).trace.push(format!("{step}: {line}"));
    }
    turmoil::repair(RELAY, SERVER);
    turmoil::release(RELAY, SERVER);
    locked(&share).settled = true;
    // The relay now has a whole link and reads to the head.
    for _ in 0..3000 {
        if locked(&share).done {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let shared = locked(&share);
    Err(format!(
        "the relay did not reach the head of the log on a whole link: cursor {}, {} connections, {} closed as too slow",
        shared.cursor, shared.connections, shared.too_slow
    )
    .into())
}

fn stored(world: &World, did: &str) -> Result<BTreeMap<String, String>, Failure> {
    let prefix = format!("at://{did}/");
    let read = || -> Result<_, pds_storage::StorageError> {
        let store = ActorDb::open(&world.config.layout.actor_db(did))?;
        Ok(store
            .all_records()?
            .into_iter()
            .map(|(uri, cid)| (uri.strip_prefix(&prefix).unwrap_or(&uri).to_owned(), cid))
            .collect())
    };
    read().map_err(|error| Failure::Setup(format!("{did}: {error}")))
}

/// Runs the network plan of `seed` and checks the oracles.
pub fn run_network_seed(seed: u64, options: NetOptions) -> Result<NetOutcome, Failure> {
    let _one_run = ONE_RUN.blocking_lock();
    pds_storage::fail::disarm_all();
    let setup = |error: std::io::Error| Failure::Setup(error.to_string());
    let world = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(setup)?
        .block_on(World::create_sized(seed, options.accounts, options.buffer))?;
    let share = Share::default();
    let fail = |what: String| Failure::Oracle { step: 0, what };

    let mut sim = turmoil::Builder::new()
        .rng_seed(seed)
        .simulation_duration(Duration::from_secs(3600))
        .tick_duration(Duration::from_millis(5))
        .min_message_latency(Duration::from_millis(1))
        .max_message_latency(Duration::from_millis(40))
        .tcp_capacity(16)
        .build();
    let router = world.router();
    sim.host(SERVER, move || serve(router.clone()));
    let sequencer = Arc::clone(world.sequencer()?);
    {
        let (share, dids) = (Arc::clone(&share), world.dids.clone());
        let sequencer = Arc::clone(&sequencer);
        sim.client(RELAY, relay(share, dids, move || sequencer.head()));
    }
    sim.client(
        CLIENT,
        client(Arc::clone(&share), world.dids.clone(), seed, options.steps),
    );
    sim.run().map_err(|error| fail(error.to_string()))?;

    let shared = std::mem::take(&mut *locked(&share));
    let events = sequencer.head();
    if shared.received != events {
        return Err(fail(format!(
            "the log has {events} events and the relay received {}",
            shared.received
        )));
    }
    // No server stopped, so every commit follows the one before, and the
    // relay had no reason to drop one.
    if shared.relay.dropped != 0 || shared.resyncs != 0 {
        return Err(fail(format!(
            "the relay dropped {} commits and needed {} repairs with #sync",
            shared.relay.dropped, shared.resyncs
        )));
    }
    for did in &world.dids {
        let stored = stored(&world, did)?;
        let expected = shared.expected.get(did).cloned().unwrap_or_default();
        if stored != expected {
            return Err(fail(format!(
                "{did} holds other records than the client was told"
            )));
        }
        let relay = shared.relay.repos.get(did).cloned().unwrap_or_default();
        let copy: BTreeMap<String, String> = relay
            .records
            .iter()
            .map(|(path, cid)| (path.clone(), cid.to_string()))
            .collect();
        if relay.desynced || copy != stored {
            return Err(fail(format!(
                "the relay's copy of {did} is not the server's"
            )));
        }
    }
    Ok(NetOutcome {
        trace: shared.trace,
        commits: shared.commits,
        events,
        partitions: shared.partitions,
        holds: shared.holds,
        connections: shared.connections,
        too_slow: shared.too_slow,
    })
}
