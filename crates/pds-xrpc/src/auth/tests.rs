use super::*;

/// A token as an entryway signs it: ES256K over the header and the
/// claims.
fn entryway_token(key: &pds_identity::Keypair, typ: &str, claims: &serde_json::Value) -> String {
    let head = URL_SAFE_NO_PAD.encode(serde_json::json!({"typ": typ, "alg": "ES256K"}).to_string());
    let body = URL_SAFE_NO_PAD.encode(claims.to_string());
    let signature = key.sign(format!("{head}.{body}").as_bytes());
    format!("{head}.{body}.{}", URL_SAFE_NO_PAD.encode(signature))
}

#[test]
fn a_member_host_takes_the_tokens_of_its_entryway_and_no_other() {
    let key = pds_identity::Keypair::generate(pds_identity::Algorithm::Secp256k1, |bytes| {
        bytes.fill(11);
    });
    let member = AuthConfig {
        entryway: Some(EntrywayAuth {
            did: "did:web:entryway.test".to_owned(),
            key: key.public_key(),
        }),
        ..config()
    };
    let did = "did:plc:aaaaaaaaaaaaaaaaaaaaaaaa";
    let claims = |aud: &str| serde_json::json!({"sub": did, "aud": aud, "scope": SCOPE_ACCESS, "exp": 2_000});
    let with = |token: &str| {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        headers
    };
    // For this server, and for the entryway.
    for aud in ["did:web:pds.test", "did:web:entryway.test"] {
        let token = entryway_token(&key, "at+jwt", &claims(aud));
        let credential = optional_credentials(&with(&token), &member, &[], 1_000).unwrap();
        assert!(matches!(credential, Credentials::Access { did: found, .. } if found == did));
    }
    // Another audience, another key, and a token of the server's own secret.
    let other = entryway_token(&key, "at+jwt", &claims("did:web:other.test"));
    assert!(optional_credentials(&with(&other), &member, &[], 1_000).is_err());
    let stranger = pds_identity::Keypair::generate(pds_identity::Algorithm::Secp256k1, |bytes| {
        bytes.fill(12);
    });
    let forged = entryway_token(&stranger, "at+jwt", &claims("did:web:pds.test"));
    assert!(optional_credentials(&with(&forged), &member, &[], 1_000).is_err());
    let own = mint_access(&config(), did, SCOPE_ACCESS, 1_000);
    assert!(optional_credentials(&with(&own), &config(), &[], 1_000).is_ok());
    // A token of the server's own secret has ended, so the client
    // refreshes. A token of another secret is not one of this server.
    let ended = optional_credentials(&with(&own), &member, &[], 1_000).unwrap_err();
    assert_eq!(ended.error, "ExpiredToken");
    let elsewhere = AuthConfig {
        jwt_secret: b"another".to_vec(),
        ..config()
    };
    let foreign = mint_access(&elsewhere, did, SCOPE_ACCESS, 1_000);
    let refused = optional_credentials(&with(&foreign), &member, &[], 1_000).unwrap_err();
    assert_eq!(refused.error, "InvalidToken");
    // A server without an entryway takes no ES256K token.
    let token = entryway_token(&key, "at+jwt", &claims("did:web:pds.test"));
    assert!(optional_credentials(&with(&token), &config(), &[], 1_000).is_err());
}

fn config() -> AuthConfig {
    AuthConfig {
        jwt_secret: b"secret".to_vec(),
        admin_password: "pa:ss".to_owned(),
        service_did: "did:web:pds.test".to_owned(),
        entryway: None,
        signer: None,
    }
}

fn token(claims: &serde_json::Value, typ: &str, secret: &[u8]) -> String {
    token_with_header(
        &serde_json::json!({"alg": "HS256", "typ": typ}),
        claims,
        secret,
    )
}

/// A token with any header, signed with HMAC-SHA-256 whatever `alg` the
/// header names.
fn token_with_header(
    header: &serde_json::Value,
    claims: &serde_json::Value,
    secret: &[u8],
) -> String {
    let head = URL_SAFE_NO_PAD.encode(header.to_string());
    let body = URL_SAFE_NO_PAD.encode(claims.to_string());
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).unwrap();
    mac.update(format!("{head}.{body}").as_bytes());
    let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    format!("{head}.{body}.{signature}")
}

fn with(value: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(header::AUTHORIZATION, value.parse().unwrap());
    headers
}

fn claims() -> serde_json::Value {
    serde_json::json!({"scope": "com.atproto.access", "sub": "did:plc:aaaaaaaaaaaaaaaaaaaaaaaa", "aud": "did:web:pds.test", "iat": 100, "exp": 200})
}

#[test]
fn no_header_is_no_credential() {
    assert_eq!(
        optional_credentials(&HeaderMap::new(), &config(), &[], 0).unwrap(),
        Credentials::None
    );
}

#[test]
fn the_admin_password_may_hold_a_colon() {
    let good = STANDARD.encode("admin:pa:ss");
    assert_eq!(
        optional_credentials(&with(&format!("Basic {good}")), &config(), &[], 0).unwrap(),
        Credentials::Admin
    );
    let bad = STANDARD.encode("admin:nope");
    let error =
        optional_credentials(&with(&format!("basic {bad}")), &config(), &[], 0).unwrap_err();
    assert_eq!(
        (error.status.as_u16(), error.error.as_str()),
        (401, "AuthenticationRequired")
    );
}

#[test]
fn an_access_token_names_its_account() {
    let headers = with(&format!("Bearer {}", token(&claims(), "at+jwt", b"secret")));
    let got = optional_credentials(&headers, &config(), &[], 150).unwrap();
    assert!(got.is_admin_or("did:plc:aaaaaaaaaaaaaaaaaaaaaaaa"));
    assert!(!got.is_admin_or("did:plc:bbbbbbbbbbbbbbbbbbbbbbbb"));
}

#[test]
fn refusals_carry_the_reference_names() {
    let check = |headers: HeaderMap, now: i64, error: &str, message: &str| {
        let got = optional_credentials(&headers, &config(), &[], now).unwrap_err();
        assert_eq!((got.error.as_str(), got.message.as_str()), (error, message));
    };
    let good = token(&claims(), "at+jwt", b"secret");
    check(
        with(&format!("Bearer {good}")),
        200,
        "ExpiredToken",
        "Token has expired",
    );
    check(
        with("Bearer abc"),
        0,
        "InvalidToken",
        "Token could not be verified",
    );
    check(
        with(&format!("Bearer {}", token(&claims(), "at+jwt", b"other"))),
        150,
        "InvalidToken",
        "Token could not be verified",
    );
    check(
        with(&format!(
            "Bearer {}",
            token(&claims(), "refresh+jwt", b"secret")
        )),
        150,
        "InvalidToken",
        "Token could not be verified",
    );
    check(
        with("Weird abc"),
        0,
        "InvalidToken",
        "Unsupported authorization type: Weird",
    );
    check(
        with("Bearer a b"),
        0,
        "InvalidToken",
        "Malformed authorization header",
    );
    let mut refresh = claims();
    refresh["scope"] = "com.atproto.refresh".into();
    check(
        with(&format!("Bearer {}", token(&refresh, "at+jwt", b"secret"))),
        150,
        "InvalidToken",
        "Bad token scope",
    );
}

/// The messages are the ones this server gives. The Reference's wording
/// for these cases is not compared here.
#[test]
fn a_token_that_fails_a_check_is_refused() {
    let check = |authorization: &str, message: &str| {
        let got = optional_credentials(&with(authorization), &config(), &[], 150).unwrap_err();
        assert_eq!(
            (got.error.as_str(), got.message.as_str()),
            ("InvalidToken", message),
            "{authorization}"
        );
    };
    let bearer = |header: &serde_json::Value, claims: &serde_json::Value| {
        format!("Bearer {}", token_with_header(header, claims, b"secret"))
    };
    let good_header = serde_json::json!({"alg": "HS256", "typ": "at+jwt"});
    let with_claim = |name: &str, value: serde_json::Value| {
        let mut changed = claims();
        changed[name] = value;
        bearer(&good_header, &changed)
    };

    // An unsigned token: `alg` is `none` and the signature is empty.
    let none = serde_json::json!({"alg": "none", "typ": "at+jwt"});
    let head = URL_SAFE_NO_PAD.encode(none.to_string());
    let body = URL_SAFE_NO_PAD.encode(claims().to_string());
    check(
        &format!("Bearer {head}.{body}."),
        "Token could not be verified",
    );
    // The signature is good for the secret; only `alg` is wrong.
    check(&bearer(&none, &claims()), "Token could not be verified");
    check(
        &bearer(
            &serde_json::json!({"alg": "HS512", "typ": "at+jwt"}),
            &claims(),
        ),
        "Token could not be verified",
    );
    check(
        &with_claim("aud", "did:web:other.test".into()),
        "Token could not be verified",
    );
    check(
        &with_claim("exp", "soon".into()),
        "Token could not be verified",
    );
    check(
        &with_claim("lxm", "com.atproto.sync.getRepo".into()),
        "Malformed token",
    );
    check(
        &with_claim("cnf", serde_json::json!({"jkt": "abc"})),
        "Malformed token",
    );
    check(&with_claim("sub", "alice".into()), "Malformed token");
    check(&with_claim("jti", 1.into()), "Malformed token");
    check("DPoP abc", "Token could not be verified");
}

/// A recorded decision: a token without `exp` does not expire.
#[test]
fn a_token_without_an_expiry_is_accepted() {
    let mut eternal = claims();
    eternal.as_object_mut().unwrap().remove("exp");
    let headers = with(&format!("Bearer {}", token(&eternal, "at+jwt", b"secret")));
    assert_eq!(
        optional_credentials(&headers, &config(), &[], i64::MAX).unwrap(),
        Credentials::Access {
            did: "did:plc:aaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
            scope: "com.atproto.access".to_owned(),
        }
    );
}

#[test]
fn a_takendown_token_is_read_only_where_the_method_allows_it() {
    let mut taken = claims();
    taken["scope"] = SCOPE_TAKENDOWN.into();
    let headers = with(&format!("Bearer {}", token(&taken, "at+jwt", b"secret")));
    assert!(optional_credentials(&headers, &config(), &[], 150).is_err());
    assert!(optional_credentials(&headers, &config(), &[SCOPE_TAKENDOWN], 150).is_ok());
}

fn request_state() -> crate::AppState {
    use crate::{AppState, ServerConfig};
    use pds_identity::{DidResolver, ResolverConfig};
    use pds_storage::{DataLayout, ReadPool};
    use std::path::Path;
    use std::time::Duration;
    let http = reqwest::Client::new();
    let config = ServerConfig {
        dev_mode: false,
        hostname: "pds.test".to_owned(),
        version: Some("0.0.0-test".to_owned()),
        handle_domains: vec![".test".to_owned()],
        invite_required: false,
        blob_upload_limit: 1024,
        privacy_policy_url: None,
        terms_of_service_url: None,
        branding: crate::state::Branding::default(),
        contact_email: None,
        appview_url: None,
        appview_did: None,
        mod_service: None,
        report_service: None,
        proxy_timeout: Duration::from_secs(1),
        auth: AuthConfig {
            jwt_secret: b"secret".to_vec(),
            admin_password: "password".to_owned(),
            service_did: "did:web:pds.test".to_owned(),
            entryway: None,
            signer: None,
        },
        identity: None,
        mail: crate::mail::MailConfig::default(),
        rate_limits: crate::ratelimit::RateLimitConfig::default(),
        oauth: crate::oauth::OAuthConfig::default(),
        ops_client_ids: Vec::new(),
        mcp_client_ids: Vec::new(),
        entryway: None,
        member_host: None,
        audit_repo: None,
        profile: std::sync::Arc::new(crate::ext::Stock),
        appview_cdn_url_pattern: None,
    };
    let resolver = DidResolver::new(
        ResolverConfig {
            plc_url: "http://127.0.0.1:9".to_owned(),
            timeout: Duration::from_secs(1),
            stale_ttl: Duration::from_secs(1),
            max_ttl: Duration::from_secs(1),
            dev_mode: true,
        },
        http.clone(),
        None,
        || 0,
    );
    // No route under test opens the data directory.
    let layout = DataLayout::new(Path::new("/nonexistent/pds-xrpc-auth-cache-test"));
    let reads = ReadPool::new(layout, 0);
    AppState::new(config, reads, None, resolver, http, None, || 0)
}

#[tokio::test]
async fn request_claims_do_not_cross_tokens_configs_times_or_tasks() {
    let state = request_state();
    let config = &state.config().auth;
    let signed = token(&claims(), "at+jwt", b"secret");
    let header = with(&format!("Bearer {signed}"));
    let checked = verify(&signed, config, "at+jwt", Some(150)).unwrap();
    with_request(async {
        REQUEST_CREDENTIAL.with(|slot| {
            *slot.borrow_mut() = Some(RequestCredential {
                state: state.clone(),
                token: signed.clone(),
                checked_at: 150,
                claims: checked,
            });
        });
        assert!(optional_credentials(&header, config, &[], 150).is_ok());
        let bad_token = with(&format!("Bearer {}", token(&claims(), "at+jwt", b"other")));
        assert!(optional_credentials(&bad_token, config, &[], 150).is_err());
        let mut other_config = config.clone();
        other_config.jwt_secret = b"other".to_vec();
        assert!(optional_credentials(&header, &other_config, &[], 150).is_err());
        assert_eq!(
            optional_credentials(&header, config, &[], 250)
                .unwrap_err()
                .error,
            "ExpiredToken"
        );
        // A restricted scope is still checked by each caller, even when
        // the cryptographic result came from the current request.
        let mut wrong_scope = claims();
        wrong_scope["scope"] = "custom.scope".into();
        let wrong_scope = token(&wrong_scope, "at+jwt", b"secret");
        let header = with(&format!("Bearer {wrong_scope}"));
        REQUEST_CREDENTIAL.with(|slot| {
            *slot.borrow_mut() = Some(RequestCredential {
                state: state.clone(),
                checked_at: 150,
                claims: verify(&wrong_scope, config, "at+jwt", Some(150)).unwrap(),
                token: wrong_scope,
            });
        });
        assert_eq!(
            optional_credentials(&header, config, &[], 150)
                .unwrap_err()
                .message,
            "Bad token scope"
        );
        assert!(optional_credentials(&header, config, &["custom.scope"], 150).is_ok());
        assert!(
            tokio::spawn(async { REQUEST_CREDENTIAL.try_with(|_| ()).is_err() })
                .await
                .unwrap()
        );
    })
    .await;
    assert!(REQUEST_CREDENTIAL.try_with(|_| ()).is_err());
}
