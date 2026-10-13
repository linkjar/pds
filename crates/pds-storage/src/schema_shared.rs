//! The shared databases as the Reference creates them at the pin.
//!
//! Every table and index of `account.sqlite`, `sequencer.sqlite` and
//! `did_cache.sqlite` is here, in the order the Reference creates them,
//! with the names of the migrations it records. The statements were read
//! from `sqlite_master` and `kysely_migration` of the stock build of the
//! parity stack. A database created from them is one the Reference opens
//! without migrating.

/// One shared database.
#[derive(Debug, Clone, Copy)]
pub struct Shared {
    /// The statements that create its tables and indexes.
    pub statements: &'static [&'static str],
    /// The migrations the Reference records for it.
    pub migrations: &'static [&'static str],
}

/// `account.sqlite`.
pub const ACCOUNT: Shared = Shared {
    statements: &[
        r#"CREATE TABLE "kysely_migration" ("name" varchar(255) not null primary key, "timestamp" varchar(255) not null)"#,
        r#"CREATE TABLE "kysely_migration_lock" ("id" varchar(255) not null primary key, "is_locked" integer default 0 not null)"#,
        r#"CREATE TABLE "app_password" ("did" varchar not null, "name" varchar not null, "passwordScrypt" varchar not null, "createdAt" varchar not null, "privileged" integer default 0 not null, constraint "app_password_pkey" primary key ("did", "name"))"#,
        r#"CREATE TABLE "invite_code" ("code" varchar primary key, "availableUses" integer not null, "disabled" int2 default 0, "forAccount" varchar not null, "createdBy" varchar not null, "createdAt" varchar not null)"#,
        r#"CREATE INDEX "invite_code_for_account_idx" on "invite_code" ("forAccount")"#,
        r#"CREATE TABLE "invite_code_use" ("code" varchar not null, "usedBy" varchar not null, "usedAt" varchar not null, constraint "invite_code_use_pkey" primary key ("code", "usedBy"))"#,
        r#"CREATE TABLE "refresh_token" ("id" varchar primary key, "did" varchar not null, "expiresAt" varchar not null, "nextId" varchar, "appPasswordName" varchar)"#,
        r#"CREATE INDEX "refresh_token_did_idx" on "refresh_token" ("did")"#,
        r#"CREATE TABLE "repo_root" ("did" varchar primary key, "cid" varchar not null, "rev" varchar not null, "indexedAt" varchar not null)"#,
        r#"CREATE TABLE "actor" ("did" varchar primary key, "handle" varchar, "createdAt" varchar not null, "takedownRef" varchar, "deactivatedAt" varchar, "deleteAfter" varchar)"#,
        r#"CREATE UNIQUE INDEX "actor_handle_lower_idx" on "actor" (lower("handle"))"#,
        r#"CREATE INDEX "actor_cursor_idx" on "actor" ("createdAt", "did")"#,
        r#"CREATE TABLE "account" ("did" varchar primary key, "email" varchar not null, "passwordScrypt" varchar not null, "emailConfirmedAt" varchar, "invitesDisabled" int2 default 0 not null)"#,
        r#"CREATE UNIQUE INDEX "account_email_lower_idx" on "account" (lower("email"))"#,
        r#"CREATE TABLE "email_token" ("purpose" varchar not null, "did" varchar not null, "token" varchar not null, "requestedAt" varchar not null, constraint "email_token_pkey" primary key ("purpose", "did"), constraint "email_token_purpose_token_unique" unique ("purpose", "token"))"#,
        r#"CREATE TABLE "authorization_request" ("id" varchar primary key, "did" varchar, "deviceId" varchar, "clientId" varchar not null, "clientAuth" varchar not null, "parameters" varchar not null, "expiresAt" varchar not null, "code" varchar)"#,
        r#"CREATE UNIQUE INDEX "authorization_request_code_idx" on "authorization_request" (code DESC) WHERE (code IS NOT NULL)"#,
        r#"CREATE INDEX "authorization_request_expires_at_idx" on "authorization_request" ("expiresAt")"#,
        r#"CREATE TABLE "device" ("id" varchar primary key, "sessionId" varchar not null, "userAgent" varchar, "ipAddress" varchar not null, "lastSeenAt" varchar not null, constraint "device_session_id_idx" unique ("sessionId"))"#,
        r#"CREATE TABLE "device_account" ("did" varchar not null, "deviceId" varchar not null, "authenticatedAt" varchar not null, "remember" boolean not null, "authorizedClients" varchar not null, constraint "device_account_pk" primary key ("deviceId", "did"), constraint "device_account_device_id_fk" foreign key ("deviceId") references "device" ("id") on delete cascade on update cascade)"#,
        r#"CREATE TABLE "token" ("id" integer primary key autoincrement, "did" varchar not null, "tokenId" varchar not null, "createdAt" varchar not null, "updatedAt" varchar not null, "expiresAt" varchar not null, "clientId" varchar not null, "clientAuth" varchar not null, "deviceId" varchar, "parameters" varchar not null, "details" varchar, "code" varchar, "currentRefreshToken" varchar, "scope" varchar, constraint "token_current_refresh_token_unique_idx" unique ("currentRefreshToken"), constraint "token_id_unique_idx" unique ("tokenId"))"#,
        r#"CREATE INDEX "token_did_idx" on "token" ("did")"#,
        r#"CREATE UNIQUE INDEX "token_code_idx" on "token" (code DESC) WHERE (code IS NOT NULL)"#,
        r#"CREATE TABLE "used_refresh_token" ("refreshToken" varchar primary key, "tokenId" integer not null, constraint "used_refresh_token_fk" foreign key ("tokenId") references "token" ("id") on delete cascade on update cascade)"#,
        r#"CREATE INDEX "used_refresh_token_id_idx" on "used_refresh_token" ("tokenId")"#,
        r#"CREATE TABLE "account_device" ("did" varchar not null, "deviceId" varchar not null, "createdAt" varchar not null, "updatedAt" varchar not null, constraint "account_device_pk" primary key ("deviceId", "did"), constraint "account_device_did_fk" foreign key ("did") references "account" ("did") on delete cascade on update cascade, constraint "account_device_device_id_fk" foreign key ("deviceId") references "device" ("id") on delete cascade on update cascade)"#,
        r#"CREATE INDEX "account_device_did_idx" on "account_device" ("did")"#,
        r#"CREATE TABLE "authorized_client" ("did" varchar not null, "clientId" varchar not null, "createdAt" varchar not null, "updatedAt" varchar not null, "data" varchar not null, constraint "authorized_client_pk" primary key ("did", "clientId"), constraint "authorized_client_did_fk" foreign key ("did") references "account" ("did") on delete cascade on update cascade)"#,
        r#"CREATE TABLE "lexicon" ("nsid" varchar primary key, "createdAt" varchar not null, "updatedAt" varchar not null, "lastSucceededAt" varchar, "uri" varchar, "lexicon" varchar)"#,
        r#"CREATE INDEX "lexicon_failures_idx" on "lexicon" ("updatedAt" DESC) WHERE ("lexicon" is NULL)"#,
    ],
    migrations: &["001", "002", "003", "004", "005", "006", "007"],
};

/// `sequencer.sqlite`.
pub const SEQUENCER: Shared = Shared {
    statements: &[
        r#"CREATE TABLE "kysely_migration" ("name" varchar(255) not null primary key, "timestamp" varchar(255) not null)"#,
        r#"CREATE TABLE "kysely_migration_lock" ("id" varchar(255) not null primary key, "is_locked" integer default 0 not null)"#,
        r#"CREATE TABLE "repo_seq" ("seq" integer primary key autoincrement, "did" varchar not null, "eventType" varchar not null, "event" blob not null, "invalidated" int2 default 0 not null, "sequencedAt" varchar not null)"#,
        r#"CREATE INDEX "repo_seq_did_idx" on "repo_seq" ("did")"#,
        r#"CREATE INDEX "repo_seq_event_type_idx" on "repo_seq" ("eventType")"#,
        r#"CREATE INDEX "repo_seq_sequenced_at_index" on "repo_seq" ("sequencedAt")"#,
    ],
    migrations: &["001"],
};

/// `did_cache.sqlite`.
pub const DID_CACHE: Shared = Shared {
    statements: &[
        r#"CREATE TABLE "kysely_migration" ("name" varchar(255) not null primary key, "timestamp" varchar(255) not null)"#,
        r#"CREATE TABLE "kysely_migration_lock" ("id" varchar(255) not null primary key, "is_locked" integer default 0 not null)"#,
        r#"CREATE TABLE "did_doc" ("did" varchar primary key, "doc" text not null, "updatedAt" bigint not null)"#,
    ],
    migrations: &["001"],
};
