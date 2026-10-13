//! A small data directory in the Reference's layout and schema, written
//! with the repository engine, for tests of `verify` and of the server.

#![allow(dead_code)] // each test file uses a part of this module

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use linkjar_pds::config::Config;
use pds_identity::{Algorithm, Keypair};
use pds_repo::{MemoryBlockStore, Repo, Write};
use pds_types::{Data, Did, Tid};
use rusqlite::{Connection, params};

pub const ALICE: &str = "did:plc:aaaaaaaaaaaaaaaaaaaaaaaa";
pub const DANA: &str = "did:plc:dddddddddddddddddddddddd";
pub const TARA: &str = "did:plc:tttttttttttttttttttttttt";
pub const POST: &str = "app.bsky.feed.post";
pub const JWT_SECRET: &str = "test-jwt-secret";
pub const ADMIN_PASSWORD: &str = "test-admin-password";
pub const BLOB_CID: &str = "bafkreib5njcqaneu6gbwsmfbqeswb654woqznfim3pjbn7klh3alxjsrhm";
pub const BLOB_BYTES: &[u8] = b"a text blob";

static NEXT: AtomicU32 = AtomicU32::new(0);

/// A data directory that is removed when the value is dropped.
pub struct DataDir {
    pub root: PathBuf,
    pub config: Config,
}

impl Drop for DataDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn connection(path: &Path, schema: &str) -> Connection {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(schema).unwrap();
    conn
}

const ACCOUNT_SCHEMA: &str = r#"
CREATE TABLE "kysely_migration" ("name" varchar(255) not null primary key, "timestamp" varchar(255) not null);
CREATE TABLE "actor" ("did" varchar primary key, "handle" varchar, "createdAt" varchar not null, "takedownRef" varchar, "deactivatedAt" varchar, "deleteAfter" varchar);
CREATE TABLE "repo_root" ("did" varchar primary key, "cid" varchar not null, "rev" varchar not null, "indexedAt" varchar not null);
CREATE TABLE "account" ("did" varchar primary key, "email" varchar not null, "passwordScrypt" varchar not null, "emailConfirmedAt" varchar, "invitesDisabled" int2 default 0 not null);
CREATE TABLE "app_password" ("did" varchar not null, "name" varchar not null, "passwordScrypt" varchar not null, "createdAt" varchar not null, "privileged" int2 default 0 not null, constraint "app_password_pkey" primary key ("did", "name"));
CREATE TABLE "invite_code" ("code" varchar primary key, "availableUses" integer not null, "disabled" int2 default 0, "forAccount" varchar not null, "createdBy" varchar not null, "createdAt" varchar not null);
CREATE TABLE "invite_code_use" ("code" varchar not null, "usedBy" varchar not null, "usedAt" varchar not null, constraint "invite_code_use_pkey" primary key ("code", "usedBy"));
CREATE TABLE "email_token" ("purpose" varchar not null, "did" varchar not null, "token" varchar not null, "requestedAt" varchar not null, constraint "email_token_pkey" primary key ("purpose", "did"));
CREATE TABLE "refresh_token" ("id" varchar primary key, "did" varchar not null, "expiresAt" varchar not null, "nextId" varchar, "appPasswordName" varchar);
CREATE TABLE "authorization_request" ("id" varchar primary key, "did" varchar, "deviceId" varchar, "clientId" varchar not null, "clientAuth" varchar not null, "parameters" varchar not null, "expiresAt" varchar not null, "code" varchar);
CREATE TABLE "device" ("id" varchar primary key, "sessionId" varchar not null, "userAgent" varchar, "ipAddress" varchar not null, "lastSeenAt" varchar not null, constraint "device_session_id_idx" unique ("sessionId"));
CREATE TABLE "account_device" ("did" varchar not null, "deviceId" varchar not null, "createdAt" varchar not null, "updatedAt" varchar not null, constraint "account_device_pk" primary key ("deviceId", "did"));
CREATE TABLE "authorized_client" ("did" varchar not null, "clientId" varchar not null, "createdAt" varchar not null, "updatedAt" varchar not null, "data" varchar not null, constraint "authorized_client_pk" primary key ("did", "clientId"));
CREATE TABLE "token" ("id" integer primary key autoincrement, "did" varchar not null, "tokenId" varchar not null, "createdAt" varchar not null, "updatedAt" varchar not null, "expiresAt" varchar not null, "clientId" varchar not null, "clientAuth" varchar not null, "deviceId" varchar, "parameters" varchar not null, "details" varchar, "code" varchar, "currentRefreshToken" varchar, "scope" varchar, constraint "token_current_refresh_token_unique_idx" unique ("currentRefreshToken"), constraint "token_id_unique_idx" unique ("tokenId"));
CREATE TABLE "used_refresh_token" ("refreshToken" varchar primary key, "tokenId" integer not null);
"#;

/// The password of every account of the directory.
pub const PASSWORD: &str = "alice-password";
/// Its stored hash, as Node's `crypto.scrypt` gives it for this salt.
const PASSWORD_HASH: &str = "00112233445566778899aabbccddeeff:21cec7557f6df22e962d06b05c9df10145e9373dd082daf06fd42aa777197af56a06d85c25dd645395d58fa7b3ac1019fbde9f94771b3caebe7be25c47fa2ba5";
/// An app password of [`ALICE`], named `phone`.
pub const APP_PASSWORD: &str = "abcd-efgh-ijkl-mnop";
const APP_PASSWORD_HASH: &str = "693c5d2d2f93d8419c4274771bd9ec78:85e4cd03fdd2ee08d68a16d6808fee33682cf4bd9cb8d317da8f36fd36e214dee69ad32d417c59d658f2f47d69784739a46444038cf8feb551da2288f0f8d890";
const ACTOR_SCHEMA: &str = r#"
CREATE TABLE "repo_root" ("did" varchar primary key, "cid" varchar not null, "rev" varchar not null, "indexedAt" varchar not null);
CREATE TABLE "repo_block" ("cid" varchar primary key, "repoRev" varchar not null, "size" integer not null, "content" blob not null);
CREATE INDEX "repo_block_repo_rev_idx" on "repo_block" ("repoRev", "cid");
CREATE TABLE "record" ("uri" varchar primary key, "cid" varchar not null, "collection" varchar not null, "rkey" varchar not null, "repoRev" varchar not null, "indexedAt" varchar not null, "takedownRef" varchar);
CREATE TABLE "blob" ("cid" varchar primary key, "mimeType" varchar not null, "size" integer not null, "tempKey" varchar, "width" integer, "height" integer, "createdAt" varchar not null, "takedownRef" varchar);
CREATE TABLE "record_blob" ("blobCid" varchar not null, "recordUri" varchar not null, constraint "record_blob_pkey" primary key ("blobCid", "recordUri"));
CREATE TABLE "backlink" ("uri" varchar not null, "path" varchar not null, "linkTo" varchar not null, constraint "backlinks_pkey" primary key ("uri", "path"));
CREATE INDEX "backlink_link_to_idx" on "backlink" ("path", "linkTo");
CREATE TABLE "account_pref" ("id" integer primary key autoincrement, "name" varchar not null, "valueJson" text not null);
"#;
const SEQUENCER_SCHEMA: &str = r#"
CREATE TABLE "repo_seq" ("seq" integer primary key autoincrement, "did" varchar not null, "eventType" varchar not null, "event" blob not null, "invalidated" int2 default 0 not null, "sequencedAt" varchar not null);
"#;
const DID_CACHE_SCHEMA: &str = r#"
CREATE TABLE "did_doc" ("did" varchar primary key, "doc" text not null, "updatedAt" bigint not null);
"#;

const TIME: &str = "2026-01-01T00:00:00.000Z";

fn post(text: &str) -> Vec<u8> {
    let mut fields = BTreeMap::new();
    fields.insert("$type".to_owned(), Data::String(POST.to_owned()));
    fields.insert("text".to_owned(), Data::String(text.to_owned()));
    fields.insert("createdAt".to_owned(), Data::String(TIME.to_owned()));
    Data::Object(fields.into_iter().collect()).to_dag_cbor()
}

/// The signing key of the `n`th actor.
pub fn key(n: u8) -> Keypair {
    Keypair::generate(Algorithm::Secp256k1, |bytes| bytes.fill(n + 1))
}

impl DataDir {
    /// Three accounts: alice with `posts` posts and one blob, dana
    /// deactivated, tara taken down.
    pub fn new(posts: usize) -> Self {
        Self::new_with(posts, &[])
    }

    /// [`DataDir::new`] with more variables of the configuration.
    pub fn new_with(posts: usize, extra: &[(&str, &str)]) -> Self {
        let root = std::env::temp_dir().join(format!(
            "linkjar-pds-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("blocks")).unwrap();
        let vars: BTreeMap<String, String> = [
            ("PDS_HOSTNAME", "pds.test"),
            ("PDS_DATA_DIRECTORY", root.to_str().unwrap()),
            (
                "PDS_BLOBSTORE_DISK_LOCATION",
                root.join("blocks").to_str().unwrap(),
            ),
            ("PDS_JWT_SECRET", JWT_SECRET),
            ("PDS_ADMIN_PASSWORD", ADMIN_PASSWORD),
            ("PDS_VERSION", "0.0.0-test"),
            ("PDS_INVITE_REQUIRED", "false"),
            // Nothing listens here: a test that reaches the network fails fast.
            ("PDS_DID_PLC_URL", "http://127.0.0.1:9"),
        ]
        .into_iter()
        .chain(extra.iter().copied())
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        let config = Config::from_vars(&vars).unwrap();

        let accounts = connection(&config.layout.account_db, ACCOUNT_SCHEMA);
        let sequencer = connection(&config.layout.sequencer_db, SEQUENCER_SCHEMA);
        let did_cache = connection(&config.layout.did_cache_db, DID_CACHE_SCHEMA);
        let dir = Self { root, config };
        for (n, (did, handle, takedown, deactivated)) in [
            (ALICE, "alice.pds.test", None, None),
            (DANA, "dana.pds.test", None, Some(TIME)),
            (TARA, "tara.pds.test", Some("mod-1"), None),
        ]
        .into_iter()
        .enumerate()
        {
            let n = u8::try_from(n).unwrap();
            let count = if did == ALICE { posts } else { 1 };
            let (cid, rev) = dir.write_actor(did, n, count);
            accounts
                .execute(
                    r#"insert into "actor" values (?1, ?2, ?3, ?4, ?5, null)"#,
                    params![
                        did,
                        handle,
                        format!("2026-01-0{}T00:00:00.000Z", n + 1),
                        takedown,
                        deactivated
                    ],
                )
                .unwrap();
            let email = format!("{}@example.com", handle.split('.').next().unwrap());
            let confirmed = (did == ALICE).then_some(TIME);
            accounts
                .execute(
                    r#"insert into "account" values (?1, ?2, ?3, ?4, 0)"#,
                    params![did, email, PASSWORD_HASH, confirmed],
                )
                .unwrap();
            if did == ALICE {
                accounts
                    .execute(
                        r#"insert into "app_password" values (?1, 'phone', ?2, ?3, 0)"#,
                        params![did, APP_PASSWORD_HASH, TIME],
                    )
                    .unwrap();
            }
            accounts
                .execute(
                    r#"insert into "repo_root" values (?1, ?2, ?3, ?4)"#,
                    params![did, cid, rev, TIME],
                )
                .unwrap();
            sequencer
                .execute(
                    r#"insert into "repo_seq" ("did", "eventType", "event", "sequencedAt") values (?1, 'append', x'00', ?2)"#,
                    params![did, TIME],
                )
                .unwrap();
            let multikey = key(n).public_key().to_multikey();
            let doc = format!(
                r#"{{"@context":["https://www.w3.org/ns/did/v1"],"id":"{did}","alsoKnownAs":["at://{handle}"],"verificationMethod":[{{"id":"{did}#atproto","type":"Multikey","controller":"{did}","publicKeyMultibase":"{multikey}"}}]}}"#
            );
            did_cache
                .execute(
                    r#"insert into "did_doc" values (?1, ?2, ?3)"#,
                    params![did, doc, i64::MAX / 2],
                )
                .unwrap();
        }
        dir
    }

    /// Writes one actor store with `count` posts, and for alice a blob.
    /// Returns the commit CID and the revision.
    fn write_actor(&self, did: &str, n: u8, count: usize) -> (String, String) {
        let layout = &self.config.layout;
        std::fs::create_dir_all(layout.actor_store_dir(did)).unwrap();
        let keypair = key(n);
        std::fs::write(layout.actor_key(did), keypair.secret_bytes().as_slice()).unwrap();
        let store = connection(&layout.actor_db(did), ACTOR_SCHEMA);

        // A commit takes at most 200 writes, so a large repository is
        // written in several revisions.
        let records: Vec<(String, Vec<u8>)> = (0..count)
            .map(|i| (format!("{POST}/r{i:04}"), post(&format!("post {i}"))))
            .collect();
        let parsed: Did = did.parse().unwrap();
        let mut blocks = MemoryBlockStore::new();
        let mut head: Option<(pds_repo::CommitData, Tid)> = None;
        for (n, chunk) in records.chunks(200).enumerate() {
            let writes: Vec<Write<'_>> = chunk
                .iter()
                .map(|(key, record)| Write::Create { key, record })
                .collect();
            let rev: Tid = format!("3aaaaaaaaaaa{}", n + 2).parse().unwrap();
            let sign = |bytes: &[u8]| Ok(keypair.sign(bytes).to_vec());
            let commit = match &head {
                None => {
                    Repo::format_init_commit(&blocks, &parsed, rev.clone(), &writes, sign).unwrap()
                }
                Some((previous, _)) => Repo::load(&blocks, previous.cid)
                    .unwrap()
                    .format_commit(rev.clone(), &writes, sign)
                    .unwrap(),
            };
            for cid in &commit.removed_cids {
                blocks.remove(cid);
                store
                    .execute(
                        r#"delete from "repo_block" where "cid" = ?1"#,
                        params![cid.to_string()],
                    )
                    .unwrap();
            }
            blocks.put_all(&commit.new_blocks);
            for (cid, bytes) in commit.new_blocks.iter() {
                store
                    .execute(
                        r#"insert into "repo_block" values (?1, ?2, ?3, ?4)"#,
                        params![
                            cid.to_string(),
                            rev.to_string(),
                            i64::try_from(bytes.len()).unwrap(),
                            bytes
                        ],
                    )
                    .unwrap();
            }
            for change in &commit.changes {
                let (collection, rkey) = change.key.split_once('/').unwrap();
                store
                    .execute(
                        r#"insert into "record" values (?1, ?2, ?3, ?4, ?5, ?6, null)"#,
                        params![
                            format!("at://{did}/{}", change.key),
                            change.cid.unwrap().to_string(),
                            collection,
                            rkey,
                            rev.to_string(),
                            TIME
                        ],
                    )
                    .unwrap();
            }
            head = Some((commit, rev));
        }
        let (commit, rev) = head.unwrap();
        store
            .execute(
                r#"insert into "repo_root" values (?1, ?2, ?3, ?4)"#,
                params![did, commit.cid.to_string(), rev.to_string(), TIME],
            )
            .unwrap();
        if did == ALICE && count > 0 {
            store
                .execute(
                    r#"insert into "blob" values (?1, 'text/plain', ?2, null, null, null, ?3, null)"#,
                    params![BLOB_CID, i64::try_from(BLOB_BYTES.len()).unwrap(), TIME],
                )
                .unwrap();
            store
                .execute(
                    r#"insert into "record_blob" values (?1, ?2)"#,
                    params![BLOB_CID, format!("at://{did}/{POST}/r0000")],
                )
                .unwrap();
            let blobs = self.root.join("blocks").join(did);
            std::fs::create_dir_all(&blobs).unwrap();
            std::fs::write(blobs.join(BLOB_CID), BLOB_BYTES).unwrap();
        }
        (commit.cid.to_string(), rev.to_string())
    }

    /// A connection to one actor's store, for a test that damages it.
    pub fn actor(&self, did: &str) -> Connection {
        Connection::open(self.config.layout.actor_db(did)).unwrap()
    }
}

/// A connection to one of the shared databases, for a test that damages it.
pub fn open(path: &Path) -> Connection {
    Connection::open(path).unwrap()
}
