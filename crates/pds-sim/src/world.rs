//! The world of one simulation: a data directory, a server over it, and the
//! clock, revision clock and random source that the seed drives.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, KeyInit, Mac};
use linkjar_pds::config::Config;
use pds_blob::DiskBlobStore;
use pds_identity::{Algorithm, DidResolver, Keypair, ResolverConfig};
use pds_repo::{MemoryBlockStore, Repo};
use pds_sequencer::{
    Crawlers, Sequencer, account_event, commit_event, identity_event, iso_millis, sync_event,
};
use pds_storage::{ActorDb, DataLayout, schema};
use pds_types::{Did, TidGenerator};
use pds_xrpc::auth::AuthConfig;
use pds_xrpc::{AppState, ServerConfig, WriteSettings, WriteState};
use sha2::Sha256;
use tower::ServiceExt as _;

use crate::Failure;
use crate::rng::Rng;

const JWT_SECRET: &str = "simulation-jwt-secret";
const ADMIN_PASSWORD: &str = "simulation-admin-password";
const SERVICE_DID: &str = "did:web:pds.test";
/// The first instant of every simulation: 2026-01-01T00:00:00Z.
const EPOCH_MS: i64 = 1_767_225_600_000;

/// The part of `account.sqlite` that the write path and `verify` read. The
/// whole schema arrives with the account lifecycle (`docs/plan.md`, unit 5).
const ACCOUNT_SCHEMA: &str = r#"
create table "actor" ("did" varchar primary key, "handle" varchar, "createdAt" varchar not null, "takedownRef" varchar, "deactivatedAt" varchar, "deleteAfter" varchar);
create table "repo_root" ("did" varchar primary key, "cid" varchar not null, "rev" varchar not null, "indexedAt" varchar not null);
"#;

/// The time of a simulation in milliseconds. Only the simulation moves it.
#[derive(Debug, Clone)]
pub struct Clock(Arc<AtomicI64>);

impl Clock {
    fn new() -> Self {
        Self(Arc::new(AtomicI64::new(EPOCH_MS)))
    }

    /// The time now.
    pub fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }

    /// Moves the time forward.
    pub fn advance(&self, ms: i64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
}

/// One answer of the server.
#[derive(Debug, Clone)]
pub struct Answer {
    /// The HTTP status.
    pub status: u16,
    /// The body.
    pub body: Vec<u8>,
}

impl Answer {
    /// The body as JSON, or `null` when it is not JSON.
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or(serde_json::Value::Null)
    }
}

/// A data directory and the server over it.
pub struct World {
    /// The data directory.
    pub root: PathBuf,
    /// The clock.
    pub clock: Clock,
    /// The accounts, in creation order.
    pub dids: Vec<String>,
    /// The configuration `verify` reads the directory with.
    pub config: Config,
    state: AppState,
    router: Router,
    /// The random source of the server: the names of staged uploads.
    server_rng: Arc<Mutex<Rng>>,
    /// The file system below the databases.
    pub vfs: &'static crate::vfs::SimVfs,
    /// The subscriber buffer of the server.
    buffer: u32,
    /// How many times the server was started.
    pub starts: u32,
}

impl std::fmt::Debug for World {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("World")
            .field("root", &self.root)
            .field("dids", &self.dids)
            .field("starts", &self.starts)
            .finish_non_exhaustive()
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn storage(error: impl std::fmt::Display) -> Failure {
    Failure::Setup(error.to_string())
}

/// A legacy access token for `did`, as the Reference mints it.
pub fn token(did: &str) -> String {
    let head =
        URL_SAFE_NO_PAD.encode(serde_json::json!({"alg": "HS256", "typ": "at+jwt"}).to_string());
    let claims = serde_json::json!({
        "scope": "com.atproto.access", "sub": did, "aud": SERVICE_DID,
        "iat": 1, "exp": 9_999_999_999_u64,
    });
    let body = URL_SAFE_NO_PAD.encode(claims.to_string());
    let signature = Hmac::<Sha256>::new_from_slice(JWT_SECRET.as_bytes()).map_or_else(
        |_| Vec::new(),
        |mut mac| {
            mac.update(format!("{head}.{body}").as_bytes());
            mac.finalize().into_bytes().to_vec()
        },
    );
    format!("Bearer {head}.{body}.{}", URL_SAFE_NO_PAD.encode(signature))
}

/// The signing key of the `n`th account. It is a function of `n`, so two
/// runs of a seed sign the same bytes with the same key.
pub fn key(n: usize) -> Keypair {
    let byte = u8::try_from(n % 250).unwrap_or(0) + 1;
    Keypair::generate(Algorithm::Secp256k1, |bytes| bytes.fill(byte))
}

/// The DID of the `n`th account.
fn did(n: usize) -> String {
    let letter = char::from(b'a' + u8::try_from(n % 26).unwrap_or(0));
    format!("did:plc:{}", String::from(letter).repeat(24))
}

/// Starts a server over the data directory, as a process start would: new
/// state from the files, then the repair of SPEC §7.4.
async fn start(
    root: &Path,
    clock: &Clock,
    server_rng: &Arc<Mutex<Rng>>,
    starts: u32,
    buffer: u32,
) -> Result<(AppState, pds_xrpc::write::Repaired), Failure> {
    let layout = DataLayout::new(root);
    let http = reqwest::Client::new();
    let now = {
        let clock = clock.clone();
        move || clock.now_ms()
    };
    let resolver = DidResolver::new(
        ResolverConfig {
            // Nothing listens here, and no operation of a plan resolves a DID.
            plc_url: "http://127.0.0.1:9".to_owned(),
            timeout: Duration::from_secs(1),
            stale_ttl: Duration::from_hours(1),
            max_ttl: Duration::from_hours(24),
            dev_mode: true,
        },
        http.clone(),
        None,
        now.clone(),
    );
    let config = ServerConfig {
        hostname: "pds.test".to_owned(),
        version: Some("0.0.0-sim".to_owned()),
        handle_domains: vec![".pds.test".to_owned()],
        invite_required: false,
        blob_upload_limit: 5 * 1024 * 1024,
        privacy_policy_url: None,
        terms_of_service_url: None,
        contact_email: None,
        appview_url: None,
        appview_did: None,
        mod_service: None,
        report_service: None,
        proxy_timeout: Duration::from_secs(1),
        auth: AuthConfig {
            jwt_secret: JWT_SECRET.as_bytes().to_vec(),
            admin_password: ADMIN_PASSWORD.to_owned(),
            service_did: SERVICE_DID.to_owned(),
        },
        identity: None,
        mail: pds_xrpc::mail::MailConfig::default(),
        rate_limits: pds_xrpc::ratelimit::RateLimitConfig::default(),
        oauth: pds_xrpc::oauth::OAuthConfig::default(),
        appview_cdn_url_pattern: None,
    };
    let crawlers = Crawlers::new("pds.test".to_owned(), Vec::new(), http.clone());
    let sequencer =
        Sequencer::open(&layout.sequencer_db, crawlers, now.clone()).map_err(storage)?;
    // The revision clock reads the simulated time. Each start has its own
    // clock id, as each process start has.
    let micros = {
        let clock = clock.clone();
        move || u64::try_from(clock.now_ms()).unwrap_or(0) * 1000
    };
    let random = {
        let rng = Arc::clone(server_rng);
        move || rng.lock().unwrap_or_else(PoisonError::into_inner).next()
    };
    let writes = WriteState::open(
        &layout,
        Arc::new(sequencer),
        TidGenerator::with_clock(u16::try_from(starts % 1024).unwrap_or(0), micros),
        WriteSettings {
            accepting_imports: true,
            max_import_size: None,
            backfill_limit_ms: 86_400_000,
            max_subscription_buffer: buffer,
            actor_store_cache_size: 2,
        },
    )
    .map_err(storage)?
    .with_random(random);
    let blobs = DiskBlobStore::new(root.join("blocks"));
    let state = AppState::new(
        config,
        layout,
        Some(blobs.into()),
        resolver,
        http,
        Some(writes),
        now,
    );
    let repairing = state.clone();
    let repaired = tokio::task::spawn_blocking(move || pds_xrpc::write::repair(&repairing))
        .await
        .map_err(storage)?
        .map_err(|error| Failure::Setup(error.message))?;
    Ok((state, repaired))
}

/// Writes one account as its creation leaves it: the key, a repository
/// with an empty first commit, the rows of the account database, and the
/// four events of SPEC §10.4. Returns its DID.
fn create_account(
    layout: &DataLayout,
    account_db: &rusqlite::Connection,
    sequencer: &Sequencer,
    clock: &Clock,
    n: usize,
) -> Result<String, Failure> {
    let now = iso_millis(clock.now_ms());

    let did = did(n);
    let handle = format!("user{n}.pds.test");
    let parsed: Did = did.parse().map_err(storage)?;
    let keypair = key(n);
    std::fs::create_dir_all(layout.actor_store_dir(&did)).map_err(storage)?;
    std::fs::write(layout.actor_key(&did), keypair.secret_bytes().as_slice()).map_err(storage)?;
    let rev = pds_types::Tid::from_parts(
        u64::try_from(clock.now_ms()).unwrap_or(0) * 1000 + n as u64,
        0,
    );
    let commit = Repo::format_init_commit(MemoryBlockStore::new(), &parsed, rev, &[], |bytes| {
        Ok(keypair.sign(bytes).to_vec())
    })
    .map_err(storage)?;
    drop(schema::create(&layout.actor_db(&did), schema::ACTOR_STORE).map_err(storage)?);
    let mut store = ActorDb::open_writable(&layout.actor_db(&did)).map_err(storage)?;
    let txn = store.begin().map_err(storage)?;
    txn.update_root(&did, &commit.cid, commit.rev.as_str(), &now, true)
        .map_err(storage)?;
    txn.put_blocks(&commit.new_blocks, commit.rev.as_str())
        .map_err(storage)?;
    txn.commit().map_err(storage)?;
    account_db
        .execute(
            r#"insert into "actor" ("did", "handle", "createdAt") values (?1, ?2, ?3)"#,
            rusqlite::params![did, handle, now],
        )
        .map_err(storage)?;
    account_db
        .execute(
            r#"insert into "repo_root" values (?1, ?2, ?3, ?4)"#,
            rusqlite::params![did, commit.cid.to_string(), commit.rev.as_str(), now],
        )
        .map_err(storage)?;
    let commit_block = commit
        .new_blocks
        .get(&commit.cid)
        .ok_or_else(|| Failure::Setup("the first commit has no block".to_owned()))?;
    sequencer
        .sequence(&[
            identity_event(&did, Some(&handle), &now),
            account_event(&did, None, &now),
            commit_event(&did, &commit, &[], &now),
            sync_event(&did, commit.rev.as_str(), &commit.cid, commit_block, &now),
        ])
        .map_err(storage)?;
    Ok(did)
}

impl World {
    /// A data directory with `accounts` accounts, each with an empty
    /// repository and the four events of an account creation
    /// (SPEC §10.4), and a server over it.
    pub async fn create(seed: u64, accounts: usize) -> Result<Self, Failure> {
        Self::create_sized(seed, accounts, 500).await
    }

    /// [`World::create`] with a subscriber buffer of `buffer` events
    /// (`PDS_MAX_SUBSCRIPTION_BUFFER`).
    pub async fn create_sized(seed: u64, accounts: usize, buffer: u32) -> Result<Self, Failure> {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "pds-sim-{}-{}-{seed}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("blocks")).map_err(storage)?;
        // Every database of the run is behind the simulation's file system.
        let vfs = crate::vfs::SimVfs::install().map_err(storage)?;
        vfs.reset();
        let layout = DataLayout::new(&root);
        let clock = Clock::new();
        let account_db = schema::create(&layout.account_db, ACCOUNT_SCHEMA).map_err(storage)?;
        drop(schema::create(&layout.did_cache_db, schema::DID_CACHE).map_err(storage)?);
        drop(schema::create(&layout.sequencer_db, schema::SEQUENCER).map_err(storage)?);
        let sequencer = Sequencer::open(
            &layout.sequencer_db,
            Crawlers::new("pds.test".to_owned(), Vec::new(), reqwest::Client::new()),
            {
                let clock = clock.clone();
                move || clock.now_ms()
            },
        )
        .map_err(storage)?;

        let mut dids = Vec::with_capacity(accounts);
        for n in 0..accounts {
            dids.push(create_account(&layout, &account_db, &sequencer, &clock, n)?);
        }
        drop(sequencer);
        drop(account_db);

        let vars: BTreeMap<String, String> = [
            ("PDS_HOSTNAME", "pds.test".to_owned()),
            ("PDS_DATA_DIRECTORY", root.to_string_lossy().into_owned()),
            (
                "PDS_BLOBSTORE_DISK_LOCATION",
                root.join("blocks").to_string_lossy().into_owned(),
            ),
        ]
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect();
        let config = Config::from_vars(&vars).map_err(storage)?;
        // A simulated minute passes before the first operation, so that
        // every revision of the plan follows the first commits.
        clock.advance(60_000);
        let server_rng = Arc::new(Mutex::new(Rng::new(seed ^ 0x5eed_5eed_5eed_5eed)));
        let (state, _) = start(&root, &clock, &server_rng, 1, buffer).await?;
        Ok(Self {
            router: pds_xrpc::router(state.clone()),
            state,
            root,
            clock,
            dids,
            config,
            server_rng,
            vfs,
            buffer,
            starts: 1,
        })
    }

    /// The power is lost: the files lose writes that were not synced, and
    /// the server starts again over them. `choose(n)` answers a number
    /// below `n`.
    pub async fn power_loss(
        &mut self,
        choose: impl FnMut(u64) -> u64,
    ) -> Result<(crate::vfs::Lost, pds_xrpc::write::Repaired), Failure> {
        let lost = self.vfs.power_on(choose).map_err(storage)?;
        let repaired = self.restart().await?;
        Ok((lost, repaired))
    }

    /// Stops the server and starts it again over the same files. Returns
    /// what the repair of SPEC §7.4 did.
    pub async fn restart(&mut self) -> Result<pds_xrpc::write::Repaired, Failure> {
        self.starts += 1;
        let (state, repaired) = start(
            &self.root,
            &self.clock,
            &self.server_rng,
            self.starts,
            self.buffer,
        )
        .await?;
        self.router = pds_xrpc::router(state.clone());
        self.state = state;
        Ok(repaired)
    }

    /// The router of the running server.
    pub fn router(&self) -> Router {
        self.router.clone()
    }

    /// The event log of the running server.
    pub fn sequencer(&self) -> Result<&Arc<Sequencer>, Failure> {
        self.state
            .writes()
            .map(WriteState::sequencer)
            .ok_or_else(|| Failure::Setup("the server is read-only".to_owned()))
    }

    async fn send(&self, request: Request<Body>) -> Result<Answer, Failure> {
        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .map_err(|error| Failure::Setup(error.to_string()))?;
        let status = response.status().as_u16();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .map_err(storage)?
            .to_vec();
        Ok(Answer { status, body })
    }

    /// `POST /xrpc/<nsid>` with a JSON body, as the account `did`.
    pub async fn procedure(
        &self,
        nsid: &str,
        did: &str,
        input: &serde_json::Value,
    ) -> Result<Answer, Failure> {
        let body = input.to_string();
        let request = Request::post(format!("/xrpc/{nsid}"))
            .header(header::AUTHORIZATION, token(did))
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::CONTENT_LENGTH, body.len())
            .body(Body::from(body))
            .map_err(storage)?;
        self.send(request).await
    }

    /// `POST /xrpc/com.atproto.repo.uploadBlob`.
    pub async fn upload(&self, did: &str, bytes: Vec<u8>) -> Result<Answer, Failure> {
        let request = Request::post("/xrpc/com.atproto.repo.uploadBlob")
            .header(header::AUTHORIZATION, token(did))
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .header(header::CONTENT_LENGTH, bytes.len())
            .body(Body::from(bytes))
            .map_err(storage)?;
        self.send(request).await
    }

    /// `GET <path>` without a credential.
    pub async fn get(&self, path: &str) -> Result<Answer, Failure> {
        let request = Request::get(path).body(Body::empty()).map_err(storage)?;
        self.send(request).await
    }

    /// Whether a status is a success.
    pub fn ok(answer: &Answer) -> bool {
        answer.status == StatusCode::OK.as_u16()
    }
}
