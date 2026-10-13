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
    "PDS_METRICS_ADDRESS",
    "PDS_OPS_ADDRESS",
    "PDS_OTLP_ENDPOINT",
];

/// The families that Appendix C names with a `*`.
const KNOWN_PREFIXES: &[&str] = &["PDS_BLOBSTORE_S3_", "PDS_PROXY_", "PDS_AUDIT_KEY_"];

/// The entryway variables: "Refused until the entryway unit ships".
const ENTRYWAY: &[&str] = &[
    "PDS_ENTRYWAY_URL",
    "PDS_ENTRYWAY_DID",
    "PDS_ENTRYWAY_JWT_VERIFY_KEY_K256_PUBLIC_KEY_HEX",
    "PDS_ENTRYWAY_PLC_ROTATION_KEY",
    "PDS_ENTRYWAY_ADMIN_TOKEN",
];

/// Whether Appendix C names the variable.
fn is_known(name: &str) -> bool {
    KNOWN.contains(&name)
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
    if ENTRYWAY.contains(&name) {
        Some("this version has no entryway mode")
    } else if name.starts_with("PDS_REDIS_SCRATCH_") {
        Some("version 1 has no Redis scratch store")
    } else {
        None
    }
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
    /// `PDS_REPO_BACKFILL_LIMIT_MS`, default one day.
    pub repo_backfill_limit: Duration,
    /// `PDS_FAIL_POINTS`: fail points to arm (SPEC §19.2). Read in dev mode
    /// only.
    pub fail_points: Option<String>,
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
    pub fn from_vars(vars: &BTreeMap<String, String>) -> Result<Self, ConfigError> {
        let env = Env(vars);
        let mut unknown_variables = Vec::new();
        for name in vars.keys().filter(|name| name.starts_with("PDS_")) {
            if let Some(reason) = refusal(name) {
                // An empty value is an unset variable, as everywhere else.
                if env.0.get(name).is_some_and(|value| !value.is_empty()) {
                    return Err(ConfigError::Refused {
                        name: name.clone(),
                        reason,
                    });
                }
            } else if !is_known(name) {
                unknown_variables.push(name.clone());
            }
        }
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
            account_db: path("PDS_ACCOUNT_DB_LOCATION", default_layout.account_db),
            sequencer_db: path("PDS_SEQUENCER_DB_LOCATION", default_layout.sequencer_db),
            did_cache_db: path("PDS_DID_CACHE_DB_LOCATION", default_layout.did_cache_db),
            // The Candidate's own file has no variable: it stays beside
            // the account database.
            ops_db: default_layout.ops_db,
            actor_dir: path("PDS_ACTOR_STORE_DIRECTORY", default_layout.actor_dir),
        };
        let mut crawlers = Vec::new();
        for crawler in env
            .text("PDS_CRAWLERS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|crawler| !crawler.is_empty())
        {
            if !crawler.starts_with("http://") && !crawler.starts_with("https://") {
                return Err(ConfigError::Invalid {
                    name: "PDS_CRAWLERS",
                    value: crawler.to_owned(),
                    reason: "expected http or https URLs separated by commas",
                });
            }
            crawlers.push(crawler.trim_end_matches('/').to_owned());
        }
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
            crawlers,
            max_subscription_buffer: env.number("PDS_MAX_SUBSCRIPTION_BUFFER")?.unwrap_or(500),
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
            blob_dir: env.text("PDS_BLOBSTORE_DISK_LOCATION").map(PathBuf::from),
            jwt_secret: env.text("PDS_JWT_SECRET"),
            admin_password: env.text("PDS_ADMIN_PASSWORD"),
            plc_url: env
                .url("PDS_DID_PLC_URL")?
                .unwrap_or_else(|| "https://plc.directory".to_owned()),
            did_cache_stale_ttl: env.millis("PDS_DID_CACHE_STALE_TTL", Duration::from_hours(1))?,
            did_cache_max_ttl: env.millis("PDS_DID_CACHE_MAX_TTL", Duration::from_hours(24))?,
            resolver_timeout: env.millis("PDS_ID_RESOLVER_TIMEOUT", Duration::from_secs(3))?,
            appview_url: env.url("PDS_BSKY_APP_VIEW_URL")?,
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
            ("PDS_BLOBSTORE_S3_BUCKET", "x"),
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
}
