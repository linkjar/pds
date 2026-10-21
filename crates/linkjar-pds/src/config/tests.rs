use super::*;

fn vars(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

#[test]
fn production_cannot_disable_ssrf_protection() {
    assert!(Config::from_vars(&vars(&[("PDS_DISABLE_SSRF_PROTECTION", "true")])).is_err());
    assert!(
        Config::from_vars(&vars(&[
            ("PDS_DISABLE_SSRF_PROTECTION", "true"),
            ("PDS_DEV_MODE", "true")
        ]))
        .is_ok()
    );
}

#[test]
fn imports_have_a_finite_positive_default_limit() {
    assert_eq!(
        Config::from_vars(&vars(&[])).unwrap().max_import_size,
        Some(64 * 1024 * 1024)
    );
    assert!(Config::from_vars(&vars(&[("PDS_MAX_REPO_IMPORT_SIZE", "0")])).is_err());
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
    assert!(!config.disable_wal_auto_checkpoint);
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
        // Accepted and not read: a warning, not an unknown variable.
        ("PDS_OTLP_ENDPOINT", "http://otel.test:4318"),
        ("PDS_INVITE_INTERVAL", "604800000"),
        ("PDS_ERROR_COLOR", ""),
        // Accepted and read.
        ("PDS_SQLITE_DISABLE_WAL_AUTO_CHECKPOINT", "true"),
    ]))
    .unwrap();
    assert_eq!(
        config.unknown_variables,
        ["PDS_EXTERNAL_APPLE_CLIENT_TOKEN", "PDS_JWT_SECERT"]
    );
    assert_eq!(
        config.unread_variables,
        ["PDS_INVITE_INTERVAL", "PDS_OTLP_ENDPOINT"]
    );
    assert!(config.disable_wal_auto_checkpoint);
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
