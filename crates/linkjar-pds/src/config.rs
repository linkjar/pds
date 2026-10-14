//! Configuration from the environment (`docs/SPEC.md` §14.1, Appendix C).
//!
//! As of unit 3 this module reads the variables the read-only server uses, with the
//! Reference's defaults. An invalid value stops the start. The other
//! variables of Appendix C are accepted and not yet read; each arrives with
//! the unit that uses it. A variable that Appendix C refuses stops the start.
//! A `PDS_*` variable that Appendix C does not name is kept in
//! [`Config::unknown_variables`], and the commands warn about it.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use pds_storage::DataLayout;

/// A variable holds a value the server cannot use, or one it needs is
/// absent.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConfigError {
    /// A required variable is not set.
    #[error("{0} is required")]
    Missing(&'static str),
    /// One or two of the three hCaptcha variables are set.
    #[error("Partial hCaptcha config, must set site key, secret key and token salt")]
    PartialCaptcha,
    /// A variable does not parse.
    #[error("{name}={value:?}: {reason}")]
    Invalid {
        /// The variable.
        name: &'static str,
        /// Its value.
        value: String,
        /// Why it is refused.
        reason: &'static str,
    },
    /// A variable is set that this version refuses (Appendix C).
    #[error("{name} is set: {reason}")]
    Refused {
        /// The variable.
        name: String,
        /// Why it is refused.
        reason: &'static str,
    },
}

/// The variables of Appendix C with a fixed name, and the four fixed names
/// under "New in the Candidate".
const KNOWN: &[&str] = &[
    // Service
    "PDS_HOSTNAME",
    "PDS_PORT",
    "PDS_SERVICE_DID",
    "PDS_SERVICE_NAME",
    "PDS_VERSION",
    "PDS_HOME_URL",
    "PDS_SERVICE_HANDLE_DOMAINS",
    "PDS_DEV_MODE",
    "PDS_CONTACT_EMAIL_ADDRESS",
    "PDS_PRIVACY_POLICY_URL",
    "PDS_TERMS_OF_SERVICE_URL",
    "PDS_SUPPORT_URL",
    // Storage
    "PDS_DATA_DIRECTORY",
    "PDS_ACCOUNT_DB_LOCATION",
    "PDS_SEQUENCER_DB_LOCATION",
    "PDS_DID_CACHE_DB_LOCATION",
    "PDS_ACTOR_STORE_DIRECTORY",
    "PDS_ACTOR_STORE_CACHE_SIZE",
    "PDS_SQLITE_DISABLE_WAL_AUTO_CHECKPOINT",
    "PDS_BLOBSTORE_DISK_LOCATION",
    "PDS_BLOBSTORE_DISK_TMP_LOCATION",
    "PDS_BLOB_UPLOAD_LIMIT",
    "PDS_MAX_REPO_IMPORT_SIZE",
    "PDS_ACCEPTING_REPO_IMPORTS",
    // Identity
    "PDS_DID_PLC_URL",
    "PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX",
    "PDS_PLC_ROTATION_KEY_KMS_KEY_ID",
    "PDS_RECOVERY_DID_KEY",
    "PDS_DID_CACHE_STALE_TTL",
    "PDS_DID_CACHE_MAX_TTL",
    "PDS_ID_RESOLVER_TIMEOUT",
    "PDS_HANDLE_BACKUP_NAMESERVERS",
    "PDS_ENABLE_DID_DOC_WITH_SESSION",
    // Auth and OAuth
    "PDS_JWT_SECRET",
    "PDS_DPOP_SECRET",
    "PDS_ADMIN_PASSWORD",
    "PDS_OAUTH_TRUSTED_CLIENTS",
    "PDS_LEXICON_AUTHORITY_DID",
    // Invites and abuse
    "PDS_INVITE_REQUIRED",
    "PDS_INVITE_EPOCH",
    "PDS_INVITE_INTERVAL",
    "PDS_HCAPTCHA_SITE_KEY",
    "PDS_HCAPTCHA_SECRET_KEY",
    "PDS_HCAPTCHA_TOKEN_SALT",
    "PDS_RATE_LIMITS_ENABLED",
    "PDS_RATE_LIMIT_BYPASS_KEY",
    "PDS_RATE_LIMIT_BYPASS_IPS",
    // Firehose and proxy
    "PDS_CRAWLERS",
    "PDS_MAX_SUBSCRIPTION_BUFFER",
    "PDS_REPO_BACKFILL_LIMIT_MS",
    "PDS_BSKY_APP_VIEW_URL",
    "PDS_BSKY_APP_VIEW_DID",
    "PDS_BSKY_APP_VIEW_CDN_URL_PATTERN",
    "PDS_FETCH_MAX_RESPONSE_SIZE",
    "PDS_DISABLE_SSRF_PROTECTION",
    // Moderation and mail
    "PDS_MOD_SERVICE_URL",
    "PDS_MOD_SERVICE_DID",
    "PDS_REPORT_SERVICE_URL",
    "PDS_REPORT_SERVICE_DID",
    "PDS_EMAIL_SMTP_URL",
    "PDS_EMAIL_FROM_ADDRESS",
    "PDS_EMAIL_DISABLE_CONFIRMATION_LINK",
    "PDS_MODERATION_EMAIL_ADDRESS",
    "PDS_MODERATION_EMAIL_SMTP_URL",
    // Branding
    "PDS_LOGO_URL",
    "PDS_PRIMARY_COLOR",
    "PDS_ERROR_COLOR",
    "PDS_WARNING_COLOR",
    "PDS_SUCCESS_COLOR",
    "PDS_INFO_COLOR",
    "PDS_BACKGROUND_LIGHT_URL",
    "PDS_BACKGROUND_DARK_URL",
    // New in the Candidate
    "PDS_FAIL_POINTS",
    "PDS_PROFILE",
    "PDS_AUDIT_CHECKPOINT_REPO",
    "PDS_METRICS_ADDRESS",
    "PDS_OPS_ADDRESS",
    "PDS_OPS_CLIENT_IDS",
    "PDS_MCP_CLIENT_IDS",
    "PDS_OAUTH_KNOWN_CLIENTS_FILE",
    "PDS_POSTGRES_URL",
    "PDS_POSTGRES_SCHEMA_PREFIX",
    "PDS_OTLP_ENDPOINT",
];

/// The families that Appendix C names with a `*`.
const KNOWN_PREFIXES: &[&str] = &["PDS_BLOBSTORE_S3_", "PDS_PROXY_", "PDS_AUDIT_KEY_"];

/// The variables of a member host behind an entryway (SPEC §21.3).
const ENTRYWAY: &[&str] = &[
    "PDS_ENTRYWAY_URL",
    "PDS_ENTRYWAY_DID",
    "PDS_ENTRYWAY_JWT_VERIFY_KEY_K256_PUBLIC_KEY_HEX",
    "PDS_ENTRYWAY_PLC_ROTATION_KEY",
    "PDS_ENTRYWAY_ADMIN_TOKEN",
    // An entryway: its member host, and the key of its tokens.
    "PDS_MEMBER_URL",
    "PDS_MEMBER_INTERNAL_URL",
    "PDS_MEMBER_ADMIN_PASSWORD",
    "PDS_MEMBER_DID",
    "PDS_ENTRYWAY_JWT_SIGNING_KEY_K256_PRIVATE_KEY_HEX",
];

/// What makes a server an entryway (SPEC §21.3): the member host that
/// keeps the repositories, and the key that signs the tokens.
#[derive(Clone)]
pub struct MemberHostConfig {
    /// `PDS_MEMBER_URL`: the public address of the host.
    pub url: String,
    /// `PDS_MEMBER_INTERNAL_URL`: where this server reaches it. Default:
    /// the public address.
    pub internal_url: String,
    /// `PDS_MEMBER_ADMIN_PASSWORD`.
    pub admin_password: String,
    /// `PDS_MEMBER_DID`: the DID of the host, which signed the sessions
    /// from before the entryway. Default: `did:web:` and the host name of
    /// `PDS_MEMBER_URL`.
    pub did: String,
    /// `PDS_ENTRYWAY_JWT_SIGNING_KEY_K256_PRIVATE_KEY_HEX`: 32 bytes.
    pub signing_key: Vec<u8>,
}

impl std::fmt::Debug for MemberHostConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemberHostConfig")
            .field("url", &self.url)
            .field("did", &self.did)
            .finish_non_exhaustive()
    }
}

/// The entryway of a member host.
#[derive(Clone)]
pub struct EntrywayConfig {
    /// `PDS_ENTRYWAY_URL`, without a slash at its end.
    pub url: String,
    /// `PDS_ENTRYWAY_DID`.
    pub did: String,
    /// `PDS_ENTRYWAY_JWT_VERIFY_KEY_K256_PUBLIC_KEY_HEX`: the key that
    /// signs the tokens of the entryway.
    pub jwt_key: pds_identity::PublicKey,
    /// `PDS_ENTRYWAY_PLC_ROTATION_KEY`: a `did:key`.
    pub plc_rotation_key: String,
    /// `PDS_ENTRYWAY_ADMIN_TOKEN`.
    pub admin_token: Option<String>,
}

impl std::fmt::Debug for EntrywayConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EntrywayConfig")
            .field("url", &self.url)
            .field("did", &self.did)
            .finish_non_exhaustive()
    }
}

/// Whether Appendix C names the variable.
fn is_known(name: &str) -> bool {
    KNOWN.contains(&name)
        || ENTRYWAY.contains(&name)
        || KNOWN_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
        // `PDS_EXTERNAL_<PROVIDER>_CLIENT_ID` and `_CLIENT_SECRET`.
        || name.strip_prefix("PDS_EXTERNAL_").is_some_and(|rest| {
            ["APPLE", "GOOGLE", "GITHUB"].iter().any(|provider| {
                rest.strip_prefix(provider)
                    .is_some_and(|field| matches!(field, "_CLIENT_ID" | "_CLIENT_SECRET"))
            })
        })
}

/// Why Appendix C refuses the variable, when it does.
fn refusal(name: &str) -> Option<&'static str> {
    if name.starts_with("PDS_REDIS_SCRATCH_") {
        Some("version 1 has no Redis scratch store")
    } else {
        None
    }
}

/// Which profile the server runs (SPEC §12.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileName {
    /// The Stock Reference's behaviour.
    Stock,
    /// LinkJar's behaviour (SPEC §13).
    Linkjar,
}

/// The settings of the read-only server.
#[derive(Clone)]
pub struct Config {
    /// `PDS_HOSTNAME`, default `localhost`.
    pub hostname: String,
    /// `PDS_PORT`, default 2583.
    pub port: u16,
    /// `PDS_SERVICE_DID`, default `did:web:<hostname>`.
    pub service_did: String,
    /// `PDS_VERSION`.
    pub version: Option<String>,
    /// `PDS_SERVICE_HANDLE_DOMAINS`.
    pub handle_domains: Vec<String>,
    /// `PDS_DEV_MODE`.
    pub dev_mode: bool,
    /// `PDS_INVITE_REQUIRED`, default true.
    pub invite_required: bool,
    /// `PDS_BLOB_UPLOAD_LIMIT`, default 5 MiB.
    pub blob_upload_limit: u64,
    /// `PDS_PRIVACY_POLICY_URL`.
    pub privacy_policy_url: Option<String>,
    /// `PDS_TERMS_OF_SERVICE_URL`.
    pub terms_of_service_url: Option<String>,
    /// `PDS_CONTACT_EMAIL_ADDRESS`.
    pub contact_email: Option<String>,
    /// The database and actor store locations.
    pub layout: DataLayout,
    /// `PDS_BLOBSTORE_DISK_LOCATION`.
    pub blob_dir: Option<PathBuf>,
    /// `PDS_BLOBSTORE_S3_*`: the S3-compatible blob store. It and
    /// [`Config::blob_dir`] are not both set.
    pub blob_s3: Option<pds_blob::S3Config>,
    /// `PDS_BLOBSTORE_DISK_TMP_LOCATION`, default `tempt` under the blob
    /// directory.
    pub blob_tmp_dir: Option<PathBuf>,
    /// `PDS_ACCEPTING_REPO_IMPORTS`, default true.
    pub accepting_imports: bool,
    /// `PDS_MAX_REPO_IMPORT_SIZE` in bytes.
    pub max_import_size: Option<u64>,
    /// `PDS_CRAWLERS`: the hosts to notify, each without a trailing slash.
    pub crawlers: Vec<String>,
    /// `PDS_MAX_SUBSCRIPTION_BUFFER`, default 500 events.
    pub max_subscription_buffer: u32,
    /// `PDS_ACTOR_STORE_CACHE_SIZE`, default 100 stores.
    pub actor_store_cache_size: usize,
    /// `PDS_REPO_BACKFILL_LIMIT_MS`, default one day.
    pub repo_backfill_limit: Duration,
    /// `PDS_FAIL_POINTS`: fail points to arm (SPEC §19.2). Read in dev mode
    /// only.
    pub fail_points: Option<String>,
    /// `PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX`: the key that signs the
    /// PLC operations of the accounts. Without it no account is created.
    pub plc_rotation_key: Option<Vec<u8>>,
    /// `PDS_RECOVERY_DID_KEY`.
    pub recovery_did_key: Option<String>,
    /// The mail settings: `PDS_EMAIL_*` and the branding variables.
    pub mail: pds_xrpc::mail::MailConfig,
    /// `PDS_RATE_LIMITS_ENABLED`, `PDS_RATE_LIMIT_BYPASS_KEY` and
    /// `PDS_RATE_LIMIT_BYPASS_IPS`.
    pub rate_limits: pds_xrpc::ratelimit::RateLimitConfig,
    /// `PDS_AUDIT_KEY_ED25519_PRIVATE_KEY_HEX`: the 32-byte seed of the
    /// key that signs audit checkpoints (SPEC §20.3).
    pub audit_key: Option<Vec<u8>>,
    /// `PDS_AUDIT_CHECKPOINT_REPO`: the DID of the operator's account
    /// here whose repository publishes the checkpoints.
    pub audit_repo: Option<String>,
    /// `PDS_METRICS_ADDRESS`: where the private metrics listener binds
    /// (SPEC §14.4). Unset, the server has none.
    pub metrics_address: Option<std::net::SocketAddr>,
    /// `PDS_OPS_ADDRESS`: where the private ops listener binds (SPEC
    /// §22.1). Unset: no ops listener.
    pub ops_address: Option<std::net::SocketAddr>,
    /// `PDS_OPS_CLIENT_IDS`: the OAuth clients whose sessions the ops
    /// listener takes, separated by commas.
    pub ops_client_ids: Vec<String>,
    /// `PDS_MCP_CLIENT_IDS`: the OAuth client ids of the MCP sidecars of
    /// this server, separated by commas (SPEC §22.2).
    pub mcp_client_ids: Vec<String>,
    /// The entryway, when the server is a member host (SPEC §21.3).
    pub entryway: Option<EntrywayConfig>,
    /// The member host, when the server is an entryway (SPEC §21.3).
    pub member_host: Option<MemberHostConfig>,
    /// `PDS_PROFILE` (SPEC §12.1).
    pub profile: ProfileName,
    /// `PDS_EXTERNAL_<PROVIDER>_CLIENT_ID` and `_CLIENT_SECRET`, for the
    /// providers that have both (SPEC §13.1).
    pub external_providers: Vec<pds_ext_linkjar::providers::Provider>,
    /// The OAuth settings (SPEC §11).
    pub oauth: pds_xrpc::oauth::OAuthConfig,
    /// `PDS_BSKY_APP_VIEW_CDN_URL_PATTERN`.
    pub appview_cdn_url_pattern: Option<String>,
    /// `PDS_JWT_SECRET`.
    pub jwt_secret: Option<String>,
    /// `PDS_ADMIN_PASSWORD`.
    pub admin_password: Option<String>,
    /// `PDS_DID_PLC_URL`, default `https://plc.directory`.
    pub plc_url: String,
    /// `PDS_DID_CACHE_STALE_TTL` in milliseconds, default one hour.
    pub did_cache_stale_ttl: Duration,
    /// `PDS_DID_CACHE_MAX_TTL` in milliseconds, default one day.
    pub did_cache_max_ttl: Duration,
    /// `PDS_ID_RESOLVER_TIMEOUT` in milliseconds, default three seconds.
    pub resolver_timeout: Duration,
    /// `PDS_BSKY_APP_VIEW_URL`.
    pub appview_url: Option<String>,
    /// `PDS_BSKY_APP_VIEW_DID`.
    pub appview_did: Option<String>,
    /// `PDS_MOD_SERVICE_URL` and `PDS_MOD_SERVICE_DID`.
    pub mod_service: Option<(String, String)>,
    /// `PDS_REPORT_SERVICE_URL` and `PDS_REPORT_SERVICE_DID`.
    pub report_service: Option<(String, String)>,
    /// `PDS_PROXY_HEADERS_TIMEOUT` in milliseconds, default ten seconds.
    pub proxy_timeout: Duration,
    /// The `PDS_*` variables that are set and that Appendix C does not
    /// name, in order (SPEC §14.1).
    pub unknown_variables: Vec<String>,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Secrets stay out of every log (SPEC §14.4).
        f.debug_struct("Config")
            .field("hostname", &self.hostname)
            .field("port", &self.port)
            .field("service_did", &self.service_did)
            .field("layout", &self.layout)
            .field("blob_dir", &self.blob_dir)
            .field("blob_s3", &self.blob_s3)
            .field("plc_url", &self.plc_url)
            .field("appview_url", &self.appview_url)
            .finish_non_exhaustive()
    }
}

struct Env<'a>(&'a BTreeMap<String, String>);

impl Env<'_> {
    fn text(&self, name: &'static str) -> Option<String> {
        self.0.get(name).filter(|value| !value.is_empty()).cloned()
    }

    fn boolean(&self, name: &'static str) -> Result<Option<bool>, ConfigError> {
        match self.text(name).as_deref() {
            None => Ok(None),
            Some("true" | "1") => Ok(Some(true)),
            Some("false" | "0") => Ok(Some(false)),
            Some(other) => Err(ConfigError::Invalid {
                name,
                value: other.to_owned(),
                reason: "expected true, false, 1 or 0",
            }),
        }
    }

    fn number<T: std::str::FromStr>(&self, name: &'static str) -> Result<Option<T>, ConfigError> {
        self.text(name)
            .map(|value| {
                value.parse().map_err(|_| ConfigError::Invalid {
                    name,
                    value,
                    reason: "expected a whole number in range",
                })
            })
            .transpose()
    }

    fn millis(&self, name: &'static str, default: Duration) -> Result<Duration, ConfigError> {
        Ok(self
            .number::<u64>(name)?
            .map_or(default, Duration::from_millis))
    }

    /// The S3-compatible blob store, when `PDS_BLOBSTORE_S3_BUCKET` names
    /// one. The checks and their order are the Reference's (`config.ts` at
    /// the pin); the Candidate also needs the region and the credentials
    /// at the start, where the Reference's client looks for them at the
    /// first request.
    fn blob_s3(&self) -> Result<Option<pds_blob::S3Config>, ConfigError> {
        let Some(bucket) = self.text("PDS_BLOBSTORE_S3_BUCKET") else {
            return Ok(None);
        };
        if self.text("PDS_BLOBSTORE_DISK_LOCATION").is_some() {
            return Err(ConfigError::Refused {
                name: "PDS_BLOBSTORE_S3_BUCKET".to_owned(),
                reason: "Cannot set both S3 and disk blobstore env vars",
            });
        }
        // The Reference's client falls back to these variables.
        let first = |names: &[&'static str]| names.iter().find_map(|name| self.text(name));
        let key_id = self.text("PDS_BLOBSTORE_S3_ACCESS_KEY_ID");
        let secret = self.text("PDS_BLOBSTORE_S3_SECRET_ACCESS_KEY");
        let (access_key_id, secret_access_key, session_token) = match (key_id, secret) {
            (Some(key_id), Some(secret)) => (key_id, secret, None),
            (Some(_), None) => {
                return Err(ConfigError::Missing("PDS_BLOBSTORE_S3_SECRET_ACCESS_KEY"));
            }
            (None, Some(_)) => return Err(ConfigError::Missing("PDS_BLOBSTORE_S3_ACCESS_KEY_ID")),
            (None, None) => (
                first(&["AWS_ACCESS_KEY_ID"])
                    .ok_or(ConfigError::Missing("PDS_BLOBSTORE_S3_ACCESS_KEY_ID"))?,
                first(&["AWS_SECRET_ACCESS_KEY"])
                    .ok_or(ConfigError::Missing("PDS_BLOBSTORE_S3_SECRET_ACCESS_KEY"))?,
                first(&["AWS_SESSION_TOKEN"]),
            ),
        };
        let region = first(&[
            "PDS_BLOBSTORE_S3_REGION",
            "AWS_REGION",
            "AWS_DEFAULT_REGION",
        ])
        .ok_or(ConfigError::Missing("PDS_BLOBSTORE_S3_REGION"))?;
        // The Reference reads zero as unset.
        let positive = |name| {
            Ok::<_, ConfigError>(
                self.number::<u64>(name)?
                    .filter(|millis| *millis > 0)
                    .map(Duration::from_millis),
            )
        };
        Ok(Some(pds_blob::S3Config {
            bucket,
            region,
            endpoint: self.url("PDS_BLOBSTORE_S3_ENDPOINT")?,
            force_path_style: self
                .boolean("PDS_BLOBSTORE_S3_FORCE_PATH_STYLE")?
                .unwrap_or(false),
            access_key_id,
            secret_access_key,
            session_token,
            upload_timeout: positive("PDS_BLOBSTORE_S3_UPLOAD_TIMEOUT_MS")?
                .unwrap_or(Duration::from_secs(20)),
            request_timeout: positive("PDS_BLOBSTORE_S3_REQUEST_TIMEOUT_MS")?,
        }))
    }

    /// The settings of the rate limits. They run unless
    /// `PDS_RATE_LIMITS_ENABLED` says otherwise, as at the pin.
    fn rate_limits(&self) -> Result<pds_xrpc::ratelimit::RateLimitConfig, ConfigError> {
        Ok(pds_xrpc::ratelimit::RateLimitConfig {
            enabled: self.boolean("PDS_RATE_LIMITS_ENABLED")?.unwrap_or(false),
            bypass_key: self.text("PDS_RATE_LIMIT_BYPASS_KEY"),
            bypass_ips: self
                .text("PDS_RATE_LIMIT_BYPASS_IPS")
                .map(|list| {
                    list.split(',')
                        .map(|ip| ip.trim().to_owned())
                        .filter(|ip| !ip.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
        })
    }

    /// A service the server sends requests to: its URL and its DID.
    fn service(
        &self,
        url: &'static str,
        did: &'static str,
    ) -> Result<Option<(String, String)>, ConfigError> {
        Ok(self.url(url)?.zip(self.text(did)))
    }

    /// The mail settings: `PDS_EMAIL_*` and the branding variables.
    fn mail(&self) -> Result<pds_xrpc::mail::MailConfig, ConfigError> {
        Ok(pds_xrpc::mail::MailConfig {
            smtp_url: self.text("PDS_EMAIL_SMTP_URL"),
            from_address: self.text("PDS_EMAIL_FROM_ADDRESS"),
            service_name: self.text("PDS_SERVICE_NAME"),
            home_url: self.text("PDS_HOME_URL"),
            logo_url: self.text("PDS_LOGO_URL"),
            primary_color: self.text("PDS_PRIMARY_COLOR"),
            disable_confirmation_link: self
                .boolean("PDS_EMAIL_DISABLE_CONFIRMATION_LINK")?
                .unwrap_or(false),
            moderation_smtp_url: self.text("PDS_MODERATION_EMAIL_SMTP_URL"),
            moderation_address: self.text("PDS_MODERATION_EMAIL_ADDRESS"),
            linkjar_templates: self.profile()? == ProfileName::Linkjar,
        })
    }

    /// The identity providers of the `linkjar` profile. A provider with
    /// one of its two variables stops the start, as in the patch.
    fn external_providers(&self) -> Result<Vec<pds_ext_linkjar::providers::Provider>, ConfigError> {
        use pds_ext_linkjar::providers::{Provider, ProviderId};
        const NAMES: [(ProviderId, &str, &str); 3] = [
            (
                ProviderId::Apple,
                "PDS_EXTERNAL_APPLE_CLIENT_ID",
                "PDS_EXTERNAL_APPLE_CLIENT_SECRET",
            ),
            (
                ProviderId::Google,
                "PDS_EXTERNAL_GOOGLE_CLIENT_ID",
                "PDS_EXTERNAL_GOOGLE_CLIENT_SECRET",
            ),
            (
                ProviderId::GitHub,
                "PDS_EXTERNAL_GITHUB_CLIENT_ID",
                "PDS_EXTERNAL_GITHUB_CLIENT_SECRET",
            ),
        ];
        let mut providers = Vec::new();
        if self.profile()? != ProfileName::Linkjar {
            return Ok(providers);
        }
        for (id, client_id, client_secret) in NAMES {
            match (self.text(client_id), self.text(client_secret)) {
                (Some(client_id), Some(client_secret)) => providers.push(Provider {
                    id,
                    client_id,
                    client_secret,
                }),
                (None, None) => {}
                (Some(_), None) => return Err(ConfigError::Missing(client_secret)),
                (None, Some(_)) => return Err(ConfigError::Missing(client_id)),
            }
        }
        Ok(providers)
    }

    /// `PDS_PROFILE`: `stock` when unset.
    fn profile(&self) -> Result<ProfileName, ConfigError> {
        match self.text("PDS_PROFILE").as_deref() {
            None | Some("stock") => Ok(ProfileName::Stock),
            Some("linkjar") => Ok(ProfileName::Linkjar),
            Some(other) => Err(ConfigError::Invalid {
                name: "PDS_PROFILE",
                value: other.to_owned(),
                reason: "expected stock or linkjar",
            }),
        }
    }

    /// The hCaptcha settings: all three variables, or none. The stock
    /// profile reads one or two as none, as upstream does. The `linkjar`
    /// profile stops the start (patch 099c).
    fn hcaptcha(&self) -> Result<Option<pds_xrpc::oauth::captcha::Hcaptcha>, ConfigError> {
        let settings = (
            self.text("PDS_HCAPTCHA_SITE_KEY"),
            self.text("PDS_HCAPTCHA_SECRET_KEY"),
            self.text("PDS_HCAPTCHA_TOKEN_SALT"),
        );
        match settings {
            (Some(site_key), Some(secret_key), Some(token_salt)) => {
                Ok(Some(pds_xrpc::oauth::captcha::Hcaptcha {
                    site_key,
                    secret_key,
                    token_salt,
                }))
            }
            (None, None, None) => Ok(None),
            _ if self.profile()? == ProfileName::Linkjar => Err(ConfigError::PartialCaptcha),
            _ => Ok(None),
        }
    }

    /// The OAuth settings.
    fn oauth(&self) -> Result<pds_xrpc::oauth::OAuthConfig, ConfigError> {
        Ok(pds_xrpc::oauth::OAuthConfig {
            dpop_secret: self.text("PDS_DPOP_SECRET"),
            trusted_clients: self
                .text("PDS_OAUTH_TRUSTED_CLIENTS")
                .map(|list| {
                    list.split(',')
                        .map(|client| client.trim().to_owned())
                        .filter(|client| !client.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
            disable_ssrf_protection: self
                .boolean("PDS_DISABLE_SSRF_PROTECTION")?
                .unwrap_or(false),
            hcaptcha: self.hcaptcha()?,
            known_clients: self.known_clients()?,
        })
    }

    /// `PDS_POSTGRES_URL`: the shared tier on Postgres (SPEC §21.2). The
    /// schemas are named with `PDS_POSTGRES_SCHEMA_PREFIX`, `pds` when it
    /// is not set. A file location for a database of the tier is refused
    /// beside it: the server has no second place to fall back to.
    fn shared_tier(&self) -> Result<Option<pds_storage::SharedTier>, ConfigError> {
        const URL: &str = "PDS_POSTGRES_URL";
        const PREFIX: &str = "PDS_POSTGRES_SCHEMA_PREFIX";
        let Some(url) = self.text(URL) else {
            return Ok(None);
        };
        // The value holds a password, so an error does not repeat it.
        let target = pds_storage::PgTarget::parse(&url).map_err(|reason| ConfigError::Invalid {
            name: URL,
            value: "(not shown)".to_owned(),
            reason,
        })?;
        for file in [
            "PDS_ACCOUNT_DB_LOCATION",
            "PDS_SEQUENCER_DB_LOCATION",
            "PDS_DID_CACHE_DB_LOCATION",
        ] {
            if let Some(value) = self.text(file) {
                return Err(ConfigError::Invalid {
                    name: file,
                    value,
                    reason: "PDS_POSTGRES_URL is set: the database is in Postgres and has no file",
                });
            }
        }
        let prefix = self.text(PREFIX).unwrap_or_else(|| "pds".to_owned());
        let shape_ok = prefix.len() <= 40
            && prefix.starts_with(|c: char| c.is_ascii_lowercase())
            && prefix
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !shape_ok {
            return Err(ConfigError::Invalid {
                name: PREFIX,
                value: prefix,
                reason: "expected a lower-case letter, then at most 39 lower-case letters, digits and underscores",
            });
        }
        Ok(Some(pds_storage::SharedTier::new(target, &prefix)))
    }

    /// The member host of an entryway. `PDS_MEMBER_URL` needs the admin
    /// password of the host and the signing key of the entryway. A server
    /// is an entryway or a member host, not both.
    fn member_host(&self) -> Result<Option<MemberHostConfig>, ConfigError> {
        const URL: &str = "PDS_MEMBER_URL";
        const KEY: &str = "PDS_ENTRYWAY_JWT_SIGNING_KEY_K256_PRIVATE_KEY_HEX";
        let Some(url) = self.text(URL) else {
            return Ok(None);
        };
        let refused = |reason| ConfigError::Refused {
            name: URL.to_owned(),
            reason,
        };
        if self.text("PDS_ENTRYWAY_URL").is_some() {
            return Err(refused(
                "a server is an entryway or a member host, not both",
            ));
        }
        if self
            .text("PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX")
            .is_none()
        {
            return Err(refused(
                "an entryway signs PLC operations: set PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX",
            ));
        }
        let (Some(admin_password), Some(signing_key)) = (
            self.text("PDS_MEMBER_ADMIN_PASSWORD"),
            self.secret_bytes(KEY)?,
        ) else {
            return Err(refused(
                "an entryway needs PDS_MEMBER_ADMIN_PASSWORD and PDS_ENTRYWAY_JWT_SIGNING_KEY_K256_PRIVATE_KEY_HEX",
            ));
        };
        if pds_identity::Keypair::from_secret_bytes(
            pds_identity::Algorithm::Secp256k1,
            &signing_key,
        )
        .is_err()
        {
            return Err(ConfigError::Invalid {
                name: KEY,
                value: "(hidden)".to_owned(),
                reason: "expected a secp256k1 private key",
            });
        }
        let url = url.trim_end_matches('/').to_owned();
        let host = url
            .strip_prefix("https://")
            .or_else(|| url.strip_prefix("http://"))
            .filter(|host| !host.is_empty() && !host.contains('/'))
            .ok_or_else(|| ConfigError::Invalid {
                name: URL,
                value: url.clone(),
                reason: "expected an http or https origin without a path",
            })?;
        let did = self
            .text("PDS_MEMBER_DID")
            .unwrap_or_else(|| format!("did:web:{}", host.replace(':', "%3A")));
        Ok(Some(MemberHostConfig {
            internal_url: self.text("PDS_MEMBER_INTERNAL_URL").map_or_else(
                || url.clone(),
                |internal| internal.trim_end_matches('/').to_owned(),
            ),
            url,
            admin_password,
            did,
            signing_key,
        }))
    }

    /// The entryway of a member host. With `PDS_ENTRYWAY_URL` the DID, the
    /// key of the tokens and the PLC rotation key are required, as in the
    /// Reference.
    fn entryway(&self) -> Result<Option<EntrywayConfig>, ConfigError> {
        const URL: &str = "PDS_ENTRYWAY_URL";
        const KEY: &str = "PDS_ENTRYWAY_JWT_VERIFY_KEY_K256_PUBLIC_KEY_HEX";
        let Some(url) = self.text(URL) else {
            return Ok(None);
        };
        let all = (
            self.text("PDS_ENTRYWAY_DID"),
            self.text(KEY),
            self.text("PDS_ENTRYWAY_PLC_ROTATION_KEY"),
        );
        let (Some(did), Some(key), Some(plc_rotation_key)) = all else {
            return Err(ConfigError::Refused {
                name: URL.to_owned(),
                reason: "if entryway url is configured, must include all required entryway configuration",
            });
        };
        let invalid = |name, value: &str, reason| ConfigError::Invalid {
            name,
            value: value.to_owned(),
            reason,
        };
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return Err(invalid(URL, &url, "expected an http or https URL"));
        }
        if did.parse::<pds_types::Did>().is_err() {
            return Err(invalid("PDS_ENTRYWAY_DID", &did, "expected a DID"));
        }
        if !plc_rotation_key.starts_with("did:key:") {
            return Err(invalid(
                "PDS_ENTRYWAY_PLC_ROTATION_KEY",
                &plc_rotation_key,
                "expected a did:key",
            ));
        }
        let jwt_key = data_encoding::HEXLOWER_PERMISSIVE
            .decode(key.as_bytes())
            .ok()
            .and_then(|bytes| {
                pds_identity::PublicKey::from_sec1_bytes(pds_identity::Algorithm::Secp256k1, &bytes)
                    .ok()
            })
            .ok_or_else(|| invalid(KEY, &key, "expected a secp256k1 public key in hexadecimal"))?;
        Ok(Some(EntrywayConfig {
            url: url.trim_end_matches('/').to_owned(),
            did,
            jwt_key,
            plc_rotation_key,
            admin_token: self.text("PDS_ENTRYWAY_ADMIN_TOKEN"),
        }))
    }

    /// The values of a variable that holds a list with commas.
    fn list(&self, name: &'static str) -> Vec<String> {
        self.text(name)
            .map(|list| {
                list.split(',')
                    .map(|item| item.trim().to_owned())
                    .filter(|item| !item.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// `PDS_OAUTH_KNOWN_CLIENTS_FILE`: a JSON object from each client id
    /// to the metadata document of the client. A confidential client has
    /// its keys in `jwks`: the server fetches nothing for a known client.
    fn known_clients(&self) -> Result<Vec<(String, serde_json::Value)>, ConfigError> {
        const NAME: &str = "PDS_OAUTH_KNOWN_CLIENTS_FILE";
        let Some(path) = self.text(NAME) else {
            return Ok(Vec::new());
        };
        let invalid = |reason| ConfigError::Invalid {
            name: NAME,
            value: path.clone(),
            reason,
        };
        let bytes = std::fs::read(&path).map_err(|_| invalid("the file cannot be read"))?;
        let serde_json::Value::Object(clients) =
            serde_json::from_slice(&bytes).map_err(|_| invalid("the file is not JSON"))?
        else {
            return Err(invalid("expected a JSON object from client id to metadata"));
        };
        let mut known = Vec::new();
        for (client_id, document) in clients {
            let client = pds_xrpc::oauth::validate_client(&client_id, &document)
                .map_err(|_| invalid("a client document does not pass the checks"))?;
            if client.auth_method() == "private_key_jwt" && client.metadata.get("jwks").is_none() {
                return Err(invalid("a confidential client needs its keys in \"jwks\""));
            }
            known.push((client_id, document));
        }
        Ok(known)
    }

    /// `PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX` as its 32 bytes.
    fn rotation_key(&self) -> Result<Option<Vec<u8>>, ConfigError> {
        self.secret_bytes("PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX")
    }

    /// A 32-byte secret that a variable gives as 64 hexadecimal digits.
    fn secret_bytes(&self, name: &'static str) -> Result<Option<Vec<u8>>, ConfigError> {
        let Some(hex) = self.text(name) else {
            return Ok(None);
        };
        data_encoding::HEXLOWER_PERMISSIVE
            .decode(hex.as_bytes())
            .ok()
            .filter(|bytes| bytes.len() == 32)
            .map(Some)
            .ok_or_else(|| ConfigError::Invalid {
                name,
                // The value is a secret and stays out of the message.
                value: "(hidden)".to_owned(),
                reason: "expected 64 hexadecimal digits",
            })
    }

    /// The `PDS_*` variables that Appendix C does not name. A variable that
    /// it refuses is an error.
    fn unknown_variables(&self) -> Result<Vec<String>, ConfigError> {
        let mut unknown = Vec::new();
        for name in self.0.keys().filter(|name| name.starts_with("PDS_")) {
            if let Some(reason) = refusal(name) {
                // An empty value is an unset variable, as everywhere else.
                if self.0.get(name).is_some_and(|value| !value.is_empty()) {
                    return Err(ConfigError::Refused {
                        name: name.clone(),
                        reason,
                    });
                }
            } else if !is_known(name) {
                unknown.push(name.clone());
            }
        }
        Ok(unknown)
    }

    /// A list of URLs separated by commas, each without its trailing slash.
    fn urls(&self, name: &'static str) -> Result<Vec<String>, ConfigError> {
        self.text(name)
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .map(|url| {
                if url.starts_with("http://") || url.starts_with("https://") {
                    Ok(url.trim_end_matches('/').to_owned())
                } else {
                    Err(ConfigError::Invalid {
                        name,
                        value: url.to_owned(),
                        reason: "expected http or https URLs separated by commas",
                    })
                }
            })
            .collect()
    }

    fn url(&self, name: &'static str) -> Result<Option<String>, ConfigError> {
        self.text(name)
            .map(|value| {
                if value.starts_with("http://") || value.starts_with("https://") {
                    Ok(value.trim_end_matches('/').to_owned())
                } else {
                    Err(ConfigError::Invalid {
                        name,
                        value,
                        reason: "expected an http or https URL",
                    })
                }
            })
            .transpose()
    }
}

impl Config {
    /// Reads the configuration from `vars`.
    // A table: one entry for each setting.
    #[allow(clippy::too_many_lines)]
    pub fn from_vars(vars: &BTreeMap<String, String>) -> Result<Self, ConfigError> {
        let env = Env(vars);
        let unknown_variables = env.unknown_variables()?;
        let hostname = env
            .text("PDS_HOSTNAME")
            .unwrap_or_else(|| "localhost".to_owned());
        let service_did = env
            .text("PDS_SERVICE_DID")
            .unwrap_or_else(|| format!("did:web:{hostname}"));
        if service_did.parse::<pds_types::Did>().is_err() {
            return Err(ConfigError::Invalid {
                name: "PDS_SERVICE_DID",
                value: service_did,
                reason: "expected a DID",
            });
        }
        let handle_domains = match env.text("PDS_SERVICE_HANDLE_DOMAINS") {
            Some(list) => list
                .split(',')
                .map(str::trim)
                .filter(|domain| !domain.is_empty())
                .map(str::to_owned)
                .collect(),
            None if hostname == "localhost" => vec![".test".to_owned()],
            None => vec![format!(".{hostname}")],
        };
        if let Some(bad) = handle_domains
            .iter()
            .find(|domain| !domain.starts_with('.'))
        {
            return Err(ConfigError::Invalid {
                name: "PDS_SERVICE_HANDLE_DOMAINS",
                value: bad.clone(),
                reason: "each domain starts with a dot",
            });
        }
        let data_dir = env.text("PDS_DATA_DIRECTORY").map(PathBuf::from);
        let default_layout = DataLayout::new(
            data_dir
                .as_deref()
                .unwrap_or_else(|| std::path::Path::new("")),
        );
        let path =
            |name: &'static str, default: PathBuf| env.text(name).map_or(default, PathBuf::from);
        let layout = DataLayout {
            shared: env.shared_tier()?,
            account_db: path("PDS_ACCOUNT_DB_LOCATION", default_layout.account_db),
            sequencer_db: path("PDS_SEQUENCER_DB_LOCATION", default_layout.sequencer_db),
            did_cache_db: path("PDS_DID_CACHE_DB_LOCATION", default_layout.did_cache_db),
            // The Candidate's own file has no variable: it stays beside
            // the account database.
            ops_db: default_layout.ops_db,
            audit_db: default_layout.audit_db,
            actor_dir: path("PDS_ACTOR_STORE_DIRECTORY", default_layout.actor_dir),
        };
        let dev_mode = env.boolean("PDS_DEV_MODE")?.unwrap_or(false);
        Ok(Self {
            port: env.number("PDS_PORT")?.unwrap_or(2583),
            version: env.text("PDS_VERSION"),
            dev_mode,
            blob_tmp_dir: env
                .text("PDS_BLOBSTORE_DISK_TMP_LOCATION")
                .map(PathBuf::from),
            accepting_imports: env.boolean("PDS_ACCEPTING_REPO_IMPORTS")?.unwrap_or(true),
            max_import_size: env.number("PDS_MAX_REPO_IMPORT_SIZE")?,
            crawlers: env.urls("PDS_CRAWLERS")?,
            max_subscription_buffer: env.number("PDS_MAX_SUBSCRIPTION_BUFFER")?.unwrap_or(500),
            actor_store_cache_size: env.number("PDS_ACTOR_STORE_CACHE_SIZE")?.unwrap_or(100),
            repo_backfill_limit: env
                .millis("PDS_REPO_BACKFILL_LIMIT_MS", Duration::from_hours(24))?,
            // Production ignores the hook (SPEC §3.3).
            fail_points: env.text("PDS_FAIL_POINTS").filter(|_| dev_mode),
            invite_required: env.boolean("PDS_INVITE_REQUIRED")?.unwrap_or(true),
            blob_upload_limit: env
                .number("PDS_BLOB_UPLOAD_LIMIT")?
                .unwrap_or(5 * 1024 * 1024),
            privacy_policy_url: env.text("PDS_PRIVACY_POLICY_URL"),
            terms_of_service_url: env.text("PDS_TERMS_OF_SERVICE_URL"),
            contact_email: env.text("PDS_CONTACT_EMAIL_ADDRESS"),
            blob_s3: env.blob_s3()?,
            blob_dir: env.text("PDS_BLOBSTORE_DISK_LOCATION").map(PathBuf::from),
            plc_rotation_key: env.rotation_key()?,
            recovery_did_key: env.text("PDS_RECOVERY_DID_KEY"),
            mail: env.mail()?,
            rate_limits: env.rate_limits()?,
            audit_key: env.secret_bytes("PDS_AUDIT_KEY_ED25519_PRIVATE_KEY_HEX")?,
            audit_repo: env.text("PDS_AUDIT_CHECKPOINT_REPO"),
            metrics_address: env
                .text("PDS_METRICS_ADDRESS")
                .map(|address| {
                    address.parse().map_err(|_| ConfigError::Invalid {
                        name: "PDS_METRICS_ADDRESS",
                        value: address,
                        reason: "expected an address and a port, such as 127.0.0.1:9464",
                    })
                })
                .transpose()?,
            ops_address: env
                .text("PDS_OPS_ADDRESS")
                .map(|address| {
                    address.parse().map_err(|_| ConfigError::Invalid {
                        name: "PDS_OPS_ADDRESS",
                        value: address,
                        reason: "expected an address and a port, such as 127.0.0.1:2584",
                    })
                })
                .transpose()?,
            ops_client_ids: env.list("PDS_OPS_CLIENT_IDS"),
            mcp_client_ids: env.list("PDS_MCP_CLIENT_IDS"),
            entryway: env.entryway()?,
            member_host: env.member_host()?,
            profile: env.profile()?,
            external_providers: env.external_providers()?,
            oauth: env.oauth()?,
            appview_cdn_url_pattern: env.text("PDS_BSKY_APP_VIEW_CDN_URL_PATTERN"),
            jwt_secret: env.text("PDS_JWT_SECRET"),
            admin_password: env.text("PDS_ADMIN_PASSWORD"),
            plc_url: env
                .url("PDS_DID_PLC_URL")?
                .unwrap_or_else(|| "https://plc.directory".to_owned()),
            did_cache_stale_ttl: env.millis("PDS_DID_CACHE_STALE_TTL", Duration::from_hours(1))?,
            did_cache_max_ttl: env.millis("PDS_DID_CACHE_MAX_TTL", Duration::from_hours(24))?,
            resolver_timeout: env.millis("PDS_ID_RESOLVER_TIMEOUT", Duration::from_secs(3))?,
            appview_url: env.url("PDS_BSKY_APP_VIEW_URL")?,
            appview_did: env.text("PDS_BSKY_APP_VIEW_DID"),
            mod_service: env.service("PDS_MOD_SERVICE_URL", "PDS_MOD_SERVICE_DID")?,
            report_service: env.service("PDS_REPORT_SERVICE_URL", "PDS_REPORT_SERVICE_DID")?,
            proxy_timeout: env.millis("PDS_PROXY_HEADERS_TIMEOUT", Duration::from_secs(10))?,
            hostname,
            service_did,
            handle_domains,
            layout,
            unknown_variables,
        })
    }

    /// Reads the configuration from the process environment.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_vars(&std::env::vars().collect())
    }

    /// The blob store that the configuration names: the disk store, the
    /// S3-compatible store, or none. The calls of the S3-compatible store
    /// run on `handle`.
    pub fn blob_store(
        &self,
        handle: &tokio::runtime::Handle,
    ) -> Result<Option<pds_blob::BlobStore>, pds_blob::BlobError> {
        if let Some(s3) = &self.blob_s3 {
            let clock: pds_blob::Clock = std::sync::Arc::new(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |elapsed| {
                        i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
                    })
            });
            let store = pds_blob::S3BlobStore::new(s3.clone(), handle.clone(), clock)?;
            return Ok(Some(store.into()));
        }
        Ok(self.blob_dir.clone().map(|dir| {
            let store = pds_blob::DiskBlobStore::new(dir);
            match &self.blob_tmp_dir {
                Some(tmp) => store.with_tmp(tmp).into(),
                None => store.into(),
            }
        }))
    }

    /// The URL the server is reached at, as the Reference derives it: HTTPS
    /// on the host name, and HTTP with the port on `localhost`.
    pub fn public_url(&self) -> String {
        if self.hostname == "localhost" {
            format!("http://localhost:{}", self.port)
        } else {
            format!("https://{}", self.hostname)
        }
    }

    /// The secrets `serve` needs. `verify` and `export-car` run without
    /// them.
    pub fn serve_secrets(&self) -> Result<(&str, &str), ConfigError> {
        let jwt = self
            .jwt_secret
            .as_deref()
            .ok_or(ConfigError::Missing("PDS_JWT_SECRET"))?;
        let admin = self
            .admin_password
            .as_deref()
            .ok_or(ConfigError::Missing("PDS_ADMIN_PASSWORD"))?;
        Ok((jwt, admin))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn defaults_are_the_reference_defaults() {
        let config = Config::from_vars(&vars(&[])).unwrap();
        assert_eq!(config.hostname, "localhost");
        assert_eq!(config.port, 2583);
        assert_eq!(config.service_did, "did:web:localhost");
        assert_eq!(config.handle_domains, [".test"]);
        assert!(config.invite_required);
        assert_eq!(config.blob_upload_limit, 5 * 1024 * 1024);
        assert_eq!(config.plc_url, "https://plc.directory");
        assert_eq!(config.layout.account_db, PathBuf::from("account.sqlite"));
    }

    #[test]
    fn a_hostname_gives_the_did_the_domain_and_the_paths() {
        let config = Config::from_vars(&vars(&[
            ("PDS_HOSTNAME", "pds.example.com"),
            ("PDS_DATA_DIRECTORY", "/data"),
            ("PDS_ACTOR_STORE_DIRECTORY", "/actors"),
            ("PDS_INVITE_REQUIRED", "false"),
        ]))
        .unwrap();
        assert_eq!(config.service_did, "did:web:pds.example.com");
        assert_eq!(config.handle_domains, [".pds.example.com"]);
        assert_eq!(
            config.layout.account_db,
            PathBuf::from("/data/account.sqlite")
        );
        assert_eq!(config.layout.actor_dir, PathBuf::from("/actors"));
        assert!(!config.invite_required);
    }

    #[test]
    fn the_s3_compatible_store_is_configured_as_on_the_reference() {
        let r2 = [
            ("PDS_BLOBSTORE_S3_BUCKET", "blobs"),
            ("PDS_BLOBSTORE_S3_REGION", "auto"),
            ("PDS_BLOBSTORE_S3_ENDPOINT", "https://account.r2.example"),
            ("PDS_BLOBSTORE_S3_FORCE_PATH_STYLE", "true"),
            ("PDS_BLOBSTORE_S3_ACCESS_KEY_ID", "id"),
            ("PDS_BLOBSTORE_S3_SECRET_ACCESS_KEY", "hidden"),
        ];
        let config = Config::from_vars(&vars(&r2)).unwrap();
        let s3 = config.blob_s3.clone().unwrap();
        assert_eq!(s3.bucket, "blobs");
        assert_eq!(s3.region, "auto");
        assert!(s3.force_path_style);
        assert_eq!(s3.upload_timeout, Duration::from_secs(20));
        assert_eq!(s3.stall_timeout(), Duration::from_secs(15));
        assert_eq!(config.unknown_variables, Vec::<String>::new());
        // The secret stays out of every log.
        assert!(!format!("{config:?}").contains("hidden"));

        let without = |name: &str| {
            let kept: Vec<(&str, &str)> = r2.iter().copied().filter(|(n, _)| *n != name).collect();
            Config::from_vars(&vars(&kept))
        };
        for name in [
            "PDS_BLOBSTORE_S3_REGION",
            "PDS_BLOBSTORE_S3_ACCESS_KEY_ID",
            "PDS_BLOBSTORE_S3_SECRET_ACCESS_KEY",
        ] {
            assert_eq!(without(name).unwrap_err(), ConfigError::Missing(name));
        }
        assert!(
            without("PDS_BLOBSTORE_S3_BUCKET")
                .unwrap()
                .blob_s3
                .is_none()
        );

        let mut both = r2.to_vec();
        both.push(("PDS_BLOBSTORE_DISK_LOCATION", "/blobs"));
        assert!(matches!(
            Config::from_vars(&vars(&both)),
            Err(ConfigError::Refused { .. })
        ));

        // The variables of the AWS clients are the fallback.
        let aws = Config::from_vars(&vars(&[
            ("PDS_BLOBSTORE_S3_BUCKET", "blobs"),
            ("AWS_REGION", "eu-central-1"),
            ("AWS_ACCESS_KEY_ID", "id"),
            ("AWS_SECRET_ACCESS_KEY", "hidden"),
            ("AWS_SESSION_TOKEN", "token"),
            ("PDS_BLOBSTORE_S3_UPLOAD_TIMEOUT_MS", "4000"),
        ]))
        .unwrap()
        .blob_s3
        .unwrap();
        assert_eq!(aws.region, "eu-central-1");
        assert_eq!(aws.session_token.as_deref(), Some("token"));
        assert_eq!(aws.endpoint, None);
        assert_eq!(aws.stall_timeout(), Duration::from_secs(6));
    }

    #[test]
    fn an_invalid_value_is_refused() {
        for (name, value) in [
            ("PDS_PORT", "70000"),
            ("PDS_DEV_MODE", "yes"),
            ("PDS_DID_PLC_URL", "plc.directory"),
            ("PDS_SERVICE_HANDLE_DOMAINS", "example.com"),
            ("PDS_SERVICE_DID", "pds.example.com"),
        ] {
            assert!(
                matches!(Config::from_vars(&vars(&[(name, value)])), Err(ConfigError::Invalid { name: n, .. }) if n == name),
                "{name}={value}"
            );
        }
    }

    #[test]
    fn an_unknown_variable_is_kept_and_a_known_one_is_not() {
        let config = Config::from_vars(&vars(&[
            ("PDS_JWT_SECERT", "x"),
            ("PDS_EXTERNAL_APPLE_CLIENT_TOKEN", "x"),
            ("PDS_EXTERNAL_APPLE_CLIENT_ID", "x"),
            ("PDS_BLOBSTORE_S3_REGION", "x"),
            ("PDS_PROXY_MAX_RETRIES", "x"),
            ("PDS_AUDIT_KEY_ID", "x"),
            ("PDS_PROFILE", "stock"),
            ("PDS_CRAWLERS", "https://relay.test"),
            ("LOG_LEVEL", "debug"),
        ]))
        .unwrap();
        assert_eq!(
            config.unknown_variables,
            ["PDS_EXTERNAL_APPLE_CLIENT_TOKEN", "PDS_JWT_SECERT"]
        );
    }

    #[test]
    fn a_refused_variable_stops_the_start() {
        // An entryway needs its DID, its token key and its rotation key.
        let key = pds_identity::Keypair::generate(pds_identity::Algorithm::Secp256k1, |bytes| {
            bytes.fill(9);
        });
        let public = data_encoding::HEXLOWER.encode(&key.public_key().to_compressed());
        let rotation = key.did_key();
        let member = [
            ("PDS_ENTRYWAY_URL", "https://entryway.test/"),
            ("PDS_ENTRYWAY_DID", "did:web:entryway.test"),
            (
                "PDS_ENTRYWAY_JWT_VERIFY_KEY_K256_PUBLIC_KEY_HEX",
                public.as_str(),
            ),
            ("PDS_ENTRYWAY_PLC_ROTATION_KEY", rotation.as_str()),
        ];
        let entryway = Config::from_vars(&vars(&member)).unwrap().entryway.unwrap();
        assert_eq!(entryway.url, "https://entryway.test");
        assert_eq!(entryway.admin_token, None);
        assert!(!format!("{entryway:?}").contains("jwt_key"));
        for (index, wrong) in ["ftp://e.test", "not-a-did", "abcd", "not-a-key"]
            .iter()
            .enumerate()
        {
            let mut changed = member;
            changed[index].1 = wrong;
            assert!(
                matches!(
                    Config::from_vars(&vars(&changed)),
                    Err(ConfigError::Invalid { .. })
                ),
                "{wrong}"
            );
        }
        for name in ["PDS_ENTRYWAY_URL", "PDS_REDIS_SCRATCH_ADDRESS"] {
            assert!(
                matches!(Config::from_vars(&vars(&[(name, "x")])), Err(ConfigError::Refused { name: n, .. }) if n == name),
                "{name}"
            );
            assert!(Config::from_vars(&vars(&[(name, "")])).is_ok(), "{name}");
        }
    }

    #[test]
    fn secrets_do_not_reach_debug_output() {
        let config = Config::from_vars(&vars(&[
            ("PDS_JWT_SECRET", "hunter2"),
            ("PDS_ADMIN_PASSWORD", "hunter3"),
        ]))
        .unwrap();
        let shown = format!("{config:?}");
        assert!(!shown.contains("hunter"));
        assert!(config.serve_secrets().is_ok());
    }

    #[test]
    fn a_partial_captcha_stops_the_linkjar_profile_and_not_the_stock_one() {
        let base = [
            ("PDS_HCAPTCHA_SITE_KEY", "site"),
            ("PDS_HCAPTCHA_SECRET_KEY", "secret-value"),
        ];
        let stock = Config::from_vars(&vars(&base)).unwrap();
        assert!(stock.oauth.hcaptcha.is_none());
        let mut linkjar = base.to_vec();
        linkjar.push(("PDS_PROFILE", "linkjar"));
        let refused = Config::from_vars(&vars(&linkjar))
            .err()
            .unwrap()
            .to_string();
        assert!(refused.starts_with("Partial hCaptcha config"), "{refused}");
        assert!(!refused.contains("secret-value"));
        linkjar.push(("PDS_HCAPTCHA_TOKEN_SALT", "salt"));
        assert!(
            Config::from_vars(&vars(&linkjar))
                .unwrap()
                .oauth
                .hcaptcha
                .is_some()
        );
        // A provider with one of its two variables stops the start too.
        let half = [
            ("PDS_PROFILE", "linkjar"),
            ("PDS_EXTERNAL_GOOGLE_CLIENT_ID", "x"),
        ];
        assert!(Config::from_vars(&vars(&half)).is_err());
        assert!(Config::from_vars(&vars(&[("PDS_PROFILE", "other")])).is_err());
    }
}
