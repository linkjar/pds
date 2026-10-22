use super::*;

/// What a stand-in entryway was asked: the path, the credential and the
/// body of each request.
type Asked = Arc<Mutex<Vec<(String, Option<String>, String)>>>;

/// An entryway that records each request and answers with the method it
/// was asked.
async fn entryway() -> (String, Asked) {
    let asked = Asked::default();
    let listener = common::http::bind().await;
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let app = axum::Router::new()
        .fallback(
            |State(asked): State<Asked>, request: axum::extract::Request| async move {
                let path = request.uri().path().to_owned();
                let credential = request
                    .headers()
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned);
                let body = axum::body::to_bytes(request.into_body(), 65_536)
                    .await
                    .unwrap();
                let body = String::from_utf8_lossy(&body).into_owned();
                if path == "/oauth/introspect" {
                    use base64::Engine as _;
                    let input: Value = serde_json::from_str(&body).unwrap();
                    let claims = input["token"].as_str().unwrap().split('.').nth(1).unwrap();
                    let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD
                        .decode(claims)
                        .unwrap();
                    let claims: Value = serde_json::from_slice(&claims).unwrap();
                    let answer =
                        json!({"active": true, "kind": input["kind"], "sub": claims["sub"]})
                            .to_string();
                    return ([("content-type", "application/json")], answer);
                }
                asked.lock().unwrap().push((path.clone(), credential, body));
                let answer = json!({"answeredBy": "entryway", "method": path}).to_string();
                ([("content-type", "application/json")], answer)
            },
        )
        .with_state(Arc::clone(&asked));
    tokio::spawn(async move { axum::serve(listener, app).await });
    (origin, asked)
}

/// A token as an entryway signs it.
fn entryway_token(key: &pds_identity::Keypair, claims: &Value) -> String {
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let head = URL_SAFE_NO_PAD.encode(json!({"typ": "at+jwt", "alg": "ES256K"}).to_string());
    let body = URL_SAFE_NO_PAD.encode(claims.to_string());
    let signature = key.sign(format!("{head}.{body}").as_bytes());
    format!("{head}.{body}.{}", URL_SAFE_NO_PAD.encode(signature))
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // one end-to-end protocol scenario
async fn a_member_host_keeps_repositories_and_leaves_accounts_to_its_entryway() {
    use pds_identity::{Algorithm, Keypair};
    let (entryway_url, asked) = entryway().await;
    let jwt_key = Keypair::generate(Algorithm::Secp256k1, |bytes| bytes.fill(21));
    let rotation = Keypair::generate(Algorithm::Secp256k1, |bytes| bytes.fill(22));
    let jwt_public = data_encoding::HEXLOWER.encode(&jwt_key.public_key().to_compressed());
    let server = Server::start(&[
        ("PDS_ADMIN_PASSWORD", common::ADMIN_PASSWORD),
        ("PDS_ENTRYWAY_URL", &entryway_url),
        ("PDS_ENTRYWAY_DID", "did:web:entryway.test"),
        (
            "PDS_ENTRYWAY_JWT_VERIFY_KEY_K256_PUBLIC_KEY_HEX",
            &jwt_public,
        ),
        ("PDS_ENTRYWAY_PLC_ROTATION_KEY", &rotation.did_key()),
        ("PDS_ENTRYWAY_ADMIN_TOKEN", "entryway-admin"),
        // A member host has no rotation key of its own.
        ("PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX", ""),
    ])
    .await;
    let create = "com.atproto.server.createAccount";

    // The entryway makes an account: it asks the host for a signing key,
    // signs the operation that creates the DID, and hands both over.
    let (status, reserved) = server
        .post("com.atproto.server.reserveSigningKey", None, &json!({}))
        .await;
    assert_eq!(status, 200, "{reserved}");
    let signing_key = text(&reserved, "signingKey").to_owned();
    let genesis = pds_identity::plc::create_op(
        &signing_key,
        &[rotation.did_key()],
        "mia.pds-users.org",
        "https://pds.test",
        &rotation,
    );
    let account =
        json!({"did": genesis.did, "handle": "mia.pds-users.org", "plcOp": genesis.operation});

    // What the host refuses: no operation, an operation that another key
    // signed, and a handle that the operation does not name.
    let plain =
        json!({"handle": "mia.pds-users.org", "email": "mia@example.com", "password": PASSWORD});
    let (status, refused) = server.post(create, None, &plain).await;
    assert_eq!(status, 400, "{refused}");
    assert_eq!(
        text(&refused, "message"),
        "non-entryway pds requires bringing a DID and plcOp"
    );
    let stranger = Keypair::generate(Algorithm::Secp256k1, |bytes| bytes.fill(23));
    let forged = pds_identity::plc::create_op(
        &signing_key,
        &[rotation.did_key()],
        "mia.pds-users.org",
        "https://pds.test",
        &stranger,
    );
    let forged =
        json!({"did": forged.did, "handle": "mia.pds-users.org", "plcOp": forged.operation});
    let (status, refused) = server.post(create, None, &forged).await;
    assert_eq!(
        (status, text(&refused, "error")),
        (400, "IncompatibleDidDoc"),
        "{refused}"
    );
    let mut renamed = account.clone();
    renamed["handle"] = json!("other.pds-users.org");
    let (status, refused) = server.post(create, None, &renamed).await;
    assert_eq!(
        (status, text(&refused, "error")),
        (400, "IncompatibleDidDoc"),
        "{refused}"
    );

    let before = server.ops.lock().unwrap().len();
    let (status, made) = server.post(create, None, &account).await;
    assert_eq!(status, 200, "{made}");
    assert_eq!(text(&made, "did"), genesis.did);
    // The host opens no session, sends the operation to the directory,
    // keeps the repository, and keeps no credential.
    assert_eq!(text(&made, "accessJwt"), "");
    assert_eq!(server.ops.lock().unwrap().len(), before + 1);
    let accounts = common::open(&server.dir.config.layout.account_db);
    let count = |table: &str| -> i64 {
        accounts
            .query_row(
                &format!(r#"select count(*) from "{table}" where "did" = ?1"#),
                [&genesis.did],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!(
        (count("actor"), count("account"), count("repo_root")),
        (1, 0, 1)
    );
    // The reserved key is the account's now: a second creation finds none.
    let (status, refused) = server.post(create, None, &account).await;
    assert_eq!(status, 400, "{refused}");

    // A token of the entryway writes to the repository. A token that the
    // host's own secret signed has ended.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let token = entryway_token(
        &jwt_key,
        &json!({"sub": genesis.did, "aud": "did:web:pds.test", "scope": "com.atproto.access", "exp": now + 600}),
    );
    let record = json!({"repo": genesis.did, "collection": "com.example.note", "record": {"$type": "com.example.note"}});
    let (status, written) = server
        .post("com.atproto.repo.createRecord", Some(&token), &record)
        .await;
    assert_eq!(status, 200, "{written}");
    let forged = entryway_token(
        &stranger,
        &json!({"sub": genesis.did, "aud": "did:web:pds.test", "scope": "com.atproto.access", "exp": now + 600}),
    );
    let (status, _) = server
        .post("com.atproto.repo.createRecord", Some(&forged), &record)
        .await;
    assert_eq!(status, 400);

    // Sessions and credentials go to the entryway, with the credential
    // of the client.
    let login = json!({"identifier": "mia.pds-users.org", "password": PASSWORD});
    let (status, answer) = server
        .post("com.atproto.server.createSession", None, &login)
        .await;
    assert_eq!(status, 200, "{answer}");
    assert_eq!(answer["answeredBy"], "entryway");
    let (status, _) = server
        .post(
            "com.atproto.server.refreshSession",
            Some("a-refresh-token"),
            &json!({}),
        )
        .await;
    assert_eq!(status, 200);
    {
        let asked = asked.lock().unwrap();
        assert_eq!(asked[0].0, "/xrpc/com.atproto.server.createSession");
        assert_eq!(serde_json::from_str::<Value>(&asked[0].2).unwrap(), login);
        assert_eq!(asked[1].1.as_deref(), Some("Bearer a-refresh-token"));
    }
    // An admin method of the entryway needs the admin of this host, and
    // goes on with the admin credential of the entryway.
    let change = json!({"did": genesis.did, "password": "a-new-password"});
    let (status, _) = server
        .post(
            "com.atproto.admin.updateAccountPassword",
            Some(&token),
            &change,
        )
        .await;
    assert_eq!(status, 401);
    let passed = server
        .http
        .post(format!(
            "{}/xrpc/com.atproto.admin.updateAccountPassword",
            server.origin
        ))
        .basic_auth("admin", Some(common::ADMIN_PASSWORD))
        .header("content-type", "application/json")
        .body(change.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(passed.status(), 200);
    {
        use base64::Engine as _;
        let basic = base64::engine::general_purpose::STANDARD.encode("admin:entryway-admin");
        let asked = asked.lock().unwrap();
        assert_eq!(
            asked.len(),
            3,
            "the refused request did not reach the entryway"
        );
        assert_eq!(
            asked[2].1.as_deref(),
            Some(format!("Basic {basic}").as_str())
        );
    }

    // The mark of an entryway call keeps a method on this host only with
    // the admin password: a client that sets it is still passed on.
    let rename = json!({"handle": "victim.example.com"});
    let marked = server
        .http
        .post(format!(
            "{}/xrpc/com.atproto.identity.updateHandle",
            server.origin
        ))
        .bearer_auth(&token)
        .header(pds_xrpc::api::entryway::FROM_ENTRYWAY, "1")
        .header("content-type", "application/json")
        .body(rename.to_string())
        .send()
        .await
        .unwrap();
    let (status, answer) = common::http::json_response(marked).await;
    assert_eq!(status, 200, "{answer}");
    assert_eq!(answer["answeredBy"], "entryway");
    assert_eq!(asked.lock().unwrap().len(), 4);
    let handle: Option<String> = accounts
        .query_row(
            r#"select "handle" from "actor" where "did" = ?1"#,
            [&genesis.did],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(handle.as_deref(), Some("mia.pds-users.org"));

    // The host answers for no handle and is no authorization server.
    let get = |path: &'static str| {
        let request = server
            .http
            .get(format!("{}{path}", server.origin))
            .header("host", "mia.pds-users.org");
        async move { request.send().await.unwrap() }
    };
    assert_eq!(get("/.well-known/atproto-did").await.status(), 404);
    assert_eq!(get("/oauth/authorize").await.status(), 404);
    assert_eq!(
        get("/.well-known/oauth-authorization-server")
            .await
            .status(),
        404
    );
    let resource = get("/.well-known/oauth-protected-resource").await;
    let resource: Value = serde_json::from_slice(&resource.bytes().await.unwrap()).unwrap();
    assert_eq!(resource["authorization_servers"], json!([entryway_url]));
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // one end-to-end protocol scenario
async fn an_entryway_keeps_the_accounts_of_a_member_host() {
    use pds_identity::{Algorithm, Keypair};
    let (plc_url, ops) = plc().await;
    let member_listener = common::http::bind().await;
    let entryway_listener = common::http::bind().await;
    let member_origin = format!("http://{}", member_listener.local_addr().unwrap());
    let entryway_origin = format!("http://{}", entryway_listener.local_addr().unwrap());
    let rotation_hex = "0707070707070707070707070707070707070707070707070707070707070707";
    let rotation = Keypair::from_secret_bytes(
        Algorithm::Secp256k1,
        &data_encoding::HEXLOWER
            .decode(rotation_hex.as_bytes())
            .unwrap(),
    )
    .unwrap();
    let signing_hex = "0808080808080808080808080808080808080808080808080808080808080808";
    let signing = Keypair::from_secret_bytes(
        Algorithm::Secp256k1,
        &data_encoding::HEXLOWER
            .decode(signing_hex.as_bytes())
            .unwrap(),
    )
    .unwrap();
    let signing_public = data_encoding::HEXLOWER.encode(&signing.public_key().to_compressed());

    // The member host is `pds.test`. The entryway is `entryway.test`.
    let member = Server::start_on(
        member_listener,
        &plc_url,
        Arc::clone(&ops),
        &[
            ("PDS_ADMIN_PASSWORD", "member-admin"),
            ("PDS_ENTRYWAY_URL", &entryway_origin),
            ("PDS_ENTRYWAY_DID", "did:web:entryway.test"),
            (
                "PDS_ENTRYWAY_JWT_VERIFY_KEY_K256_PUBLIC_KEY_HEX",
                &signing_public,
            ),
            ("PDS_ENTRYWAY_PLC_ROTATION_KEY", &rotation.did_key()),
            ("PDS_ENTRYWAY_ADMIN_TOKEN", common::ADMIN_PASSWORD),
            ("PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX", ""),
        ],
    )
    .await;
    let entryway = Server::start_on(
        entryway_listener,
        &plc_url,
        Arc::clone(&ops),
        &[
            ("PDS_HOSTNAME", "entryway.test"),
            ("PDS_ADMIN_PASSWORD", common::ADMIN_PASSWORD),
            ("PDS_MEMBER_URL", "https://pds.test"),
            ("PDS_MEMBER_INTERNAL_URL", &member_origin),
            ("PDS_MEMBER_ADMIN_PASSWORD", "member-admin"),
            (
                "PDS_ENTRYWAY_JWT_SIGNING_KEY_K256_PRIVATE_KEY_HEX",
                signing_hex,
            ),
            ("PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX", rotation_hex),
        ],
    )
    .await;

    // A sign-up at the entryway: the credentials stay there, the
    // repository is on the member host, and the directory hears once.
    let before = ops.lock().unwrap().len();
    let account =
        json!({"handle": "nora.pds-users.org", "email": "nora@example.com", "password": PASSWORD});
    let (status, made) = entryway.create(&account).await;
    assert_eq!(status, 200, "{made}");
    let did = text(&made, "did").to_owned();
    let access = text(&made, "accessJwt").to_owned();
    assert_eq!(ops.lock().unwrap().len(), before + 1);
    let operation = ops.lock().unwrap().last().unwrap().1.clone();
    assert_eq!(
        operation["services"]["atproto_pds"]["endpoint"],
        "https://pds.test"
    );
    let rows = |server: &Server, table: &str| -> i64 {
        common::open(&server.dir.config.layout.account_db)
            .query_row(
                &format!(r#"select count(*) from "{table}" where "did" = ?1"#),
                [&did],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!(
        (rows(&entryway, "account"), rows(&entryway, "repo_root")),
        (1, 0)
    );
    assert_eq!(
        (rows(&member, "account"), rows(&member, "repo_root")),
        (0, 1)
    );
    assert!(member.dir.config.layout.actor_db(&did).exists());
    assert!(!entryway.dir.config.layout.actor_db(&did).exists());

    // The token of the entryway writes at the member host, and at the
    // entryway, which passes the request on.
    let record = |n: u8| json!({"repo": did, "collection": "com.example.note", "record": {"$type": "com.example.note", "n": n}});
    let (status, body) = member
        .post("com.atproto.repo.createRecord", Some(&access), &record(1))
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = entryway
        .post("com.atproto.repo.createRecord", Some(&access), &record(2))
        .await;
    assert_eq!(status, 200, "{body}");
    let store = pds_storage::ActorDb::open(&member.dir.config.layout.actor_db(&did)).unwrap();
    assert_eq!(store.counts().unwrap().0, 2);

    // The entryway serves the firehose of the member host: it has commits,
    // which the entryway alone never writes.
    let firehose = entryway.origin.replacen("http://", "ws://", 1)
        + "/xrpc/com.atproto.sync.subscribeRepos?cursor=0";
    let (mut socket, _) = tokio_tungstenite::connect_async(firehose.as_str())
        .await
        .unwrap();
    let commit = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        use futures_util::StreamExt as _;
        while let Some(Ok(frame)) = socket.next().await {
            if let tokio_tungstenite::tungstenite::Message::Binary(bytes) = frame
                && bytes.windows(7).any(|window| window == b"#commit")
            {
                return true;
            }
        }
        false
    })
    .await;
    assert_eq!(commit, Ok(true));
    drop(socket);

    // A blob larger than a buffered request goes through the entryway and
    // back as a stream.
    let blob: Vec<u8> = (0..2 * 1024 * 1024).map(|i: u32| (i % 251) as u8).collect();
    let uploaded = entryway
        .http
        .post(format!(
            "{}/xrpc/com.atproto.repo.uploadBlob",
            entryway.origin
        ))
        .bearer_auth(&access)
        .header("content-type", "application/octet-stream")
        .body(blob.clone())
        .send()
        .await
        .unwrap();
    let (status, body) = (uploaded.status(), uploaded.text().await.unwrap());
    assert_eq!(status, 200, "{body}");
    let uploaded: Value = serde_json::from_str(&body).unwrap();
    let cid = uploaded["blob"]["ref"]["$link"]
        .as_str()
        .unwrap()
        .to_owned();
    let file = json!({"repo": did, "collection": "com.example.file", "record": {"$type": "com.example.file", "file": uploaded["blob"]}});
    let (status, body) = entryway
        .post("com.atproto.repo.createRecord", Some(&access), &file)
        .await;
    assert_eq!(status, 200, "{body}");
    let fetched = entryway
        .http
        .get(format!(
            "{}/xrpc/com.atproto.sync.getBlob?did={did}&cid={cid}",
            entryway.origin
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(fetched.status(), 200);
    assert_eq!(fetched.bytes().await.unwrap().as_ref(), blob.as_slice());

    // A client of the member host signs in and refreshes there: the
    // entryway answers.
    let login = json!({"identifier": "nora.pds-users.org", "password": PASSWORD});
    let (status, session) = member
        .post("com.atproto.server.createSession", None, &login)
        .await;
    assert_eq!(status, 200, "{session}");
    assert_eq!(text(&session, "did"), did);
    let wrong = json!({"identifier": "nora.pds-users.org", "password": "not-the-password"});
    assert_eq!(
        member
            .post("com.atproto.server.createSession", None, &wrong)
            .await
            .0,
        401
    );
    let (status, refreshed) = member
        .post(
            "com.atproto.server.refreshSession",
            Some(text(&session, "refreshJwt")),
            &json!({}),
        )
        .await;
    assert_eq!(status, 200, "{refreshed}");
    let access = text(&refreshed, "accessJwt").to_owned();
    let (status, body) = member
        .post("com.atproto.repo.createRecord", Some(&access), &record(3))
        .await;
    assert_eq!(status, 200, "{body}");

    // A handle change goes to the entryway, which writes the directory
    // and tells the member host.
    let rename = json!({"handle": "nora2.pds-users.org"});
    let (status, body) = member
        .post("com.atproto.identity.updateHandle", Some(&access), &rename)
        .await;
    assert_eq!(status, 200, "{body}");
    let handle = |server: &Server| -> Option<String> {
        common::open(&server.dir.config.layout.account_db)
            .query_row(
                r#"select "handle" from "actor" where "did" = ?1"#,
                [&did],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!(handle(&entryway).as_deref(), Some("nora2.pds-users.org"));
    assert_eq!(handle(&member).as_deref(), Some("nora2.pds-users.org"));
    assert_eq!(
        ops.lock().unwrap().last().unwrap().1["alsoKnownAs"],
        json!(["at://nora2.pds-users.org"])
    );

    // The authorization server of the entryway protects the member host.
    let metadata: Value = entryway
        .http
        .get(format!(
            "{}/.well-known/oauth-authorization-server",
            entryway.origin
        ))
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .map(|bytes| serde_json::from_slice(&bytes).unwrap())
        .unwrap();
    assert_eq!(
        metadata["protected_resources"],
        json!(["https://entryway.test", "https://pds.test"])
    );

    // The admin of the member host reads the account where it is kept:
    // the entryway has the email address.
    let info = member
        .http
        .get(format!(
            "{}/xrpc/com.atproto.admin.getAccountInfo?did={did}",
            member.origin
        ))
        .basic_auth("admin", Some("member-admin"))
        .send()
        .await
        .unwrap();
    assert_eq!(info.status(), 200);
    let info: Value = serde_json::from_slice(&info.bytes().await.unwrap()).unwrap();
    assert_eq!(info["email"], "nora@example.com", "{info}");

    // A deactivation at the member host goes to the entryway, which keeps
    // the status and has the member host announce it. The activation
    // checks the identity against the rotation key of the entryway.
    let deactivated = |server: &Server| -> bool {
        common::open(&server.dir.config.layout.account_db)
            .query_row(
                r#"select "deactivatedAt" is not null from "actor" where "did" = ?1"#,
                [&did],
                |row| row.get(0),
            )
            .unwrap()
    };
    let (status, body) = member
        .post(
            "com.atproto.server.deactivateAccount",
            Some(&access),
            &json!({}),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!((deactivated(&entryway), deactivated(&member)), (true, true));
    let (status, body) = member
        .post(
            "com.atproto.server.activateAccount",
            Some(&access),
            &json!({}),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        (deactivated(&entryway), deactivated(&member)),
        (false, false)
    );

    // A session from before the entryway: its refresh token, signed with
    // the secret of the old server for the old server, is taken once.
    let old = pds_xrpc::auth::AuthConfig {
        jwt_secret: common::JWT_SECRET.as_bytes().to_vec(),
        admin_password: String::new(),
        service_did: "did:web:pds.test".to_owned(),
        entryway: None,
        signer: None,
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let now = i64::try_from(now).unwrap();
    let (legacy, expires) = pds_xrpc::auth::mint_refresh(&old, &did, "legacy-session", now);
    common::open(&entryway.dir.config.layout.account_db)
        .execute(
            r#"insert into "refresh_token" ("id", "did", "expiresAt") values ('legacy-session', ?1, ?2)"#,
            [&did, &pds_types::iso_millis(expires * 1000)],
        )
        .unwrap();
    let old_access = pds_xrpc::auth::mint_access(&old, &did, "com.atproto.access", now);
    let (status, ended) = member
        .post(
            "com.atproto.repo.createRecord",
            Some(&old_access),
            &record(4),
        )
        .await;
    assert_eq!(
        (status, text(&ended, "error")),
        (400, "ExpiredToken"),
        "{ended}"
    );
    let (status, renewed) = member
        .post(
            "com.atproto.server.refreshSession",
            Some(&legacy),
            &json!({}),
        )
        .await;
    assert_eq!(status, 200, "{renewed}");
    let (status, body) = member
        .post(
            "com.atproto.repo.createRecord",
            Some(text(&renewed, "accessJwt")),
            &record(4),
        )
        .await;
    assert_eq!(status, 200, "{body}");

    // The admin of the member host renames the account: the entryway
    // keeps the handle and tells the member host, which does not pass
    // that call back.
    let renamed = member
        .http
        .post(format!(
            "{}/xrpc/com.atproto.admin.updateAccountHandle",
            member.origin
        ))
        .basic_auth("admin", Some("member-admin"))
        .header("content-type", "application/json")
        .body(json!({"did": did, "handle": "nora3.pds-users.org"}).to_string())
        .send()
        .await
        .unwrap();
    let (status, body) = (renamed.status(), renamed.text().await.unwrap());
    assert_eq!(status, 200, "{body}");
    assert_eq!(handle(&entryway).as_deref(), Some("nora3.pds-users.org"));
    assert_eq!(handle(&member).as_deref(), Some("nora3.pds-users.org"));

    // A takedown at the member host reaches the entryway, which refuses a
    // session of the account.
    let takedown = |applied: bool| {
        json!({
            "subject": {"$type": "com.atproto.admin.defs#repoRef", "did": did},
            "takedown": {"applied": applied, "ref": "parity"},
        })
    };
    let status = member
        .http
        .post(format!(
            "{}/xrpc/com.atproto.admin.updateSubjectStatus",
            member.origin
        ))
        .basic_auth("admin", Some("member-admin"))
        .header("content-type", "application/json")
        .body(takedown(true).to_string())
        .send()
        .await
        .unwrap();
    let (status, body) = (status.status(), status.text().await.unwrap());
    assert_eq!(status, 200, "{body}");
    let login = json!({"identifier": "nora3.pds-users.org", "password": PASSWORD});
    let (status, refused) = member
        .post("com.atproto.server.createSession", None, &login)
        .await;
    assert_eq!(
        (status, text(&refused, "error")),
        (401, "AccountTakedown"),
        "{refused}"
    );

    // The admin of the entryway lifts it, and the member host learns it.
    let taken = |server: &Server| -> bool {
        common::open(&server.dir.config.layout.account_db)
            .query_row(
                r#"select "takedownRef" is not null from "actor" where "did" = ?1"#,
                [&did],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!((taken(&entryway), taken(&member)), (true, true));
    let lifted = entryway
        .http
        .post(format!(
            "{}/xrpc/com.atproto.admin.updateSubjectStatus",
            entryway.origin
        ))
        .basic_auth("admin", Some(common::ADMIN_PASSWORD))
        .header("content-type", "application/json")
        .body(takedown(false).to_string())
        .send()
        .await
        .unwrap();
    let (status, body) = (lifted.status(), lifted.text().await.unwrap());
    assert_eq!(status, 200, "{body}");
    assert_eq!((taken(&entryway), taken(&member)), (false, false));
    let (status, body) = member
        .post("com.atproto.server.createSession", None, &login)
        .await;
    assert_eq!(status, 200, "{body}");

    // A sign-up that stopped after the member host created the repository:
    // `reconcile` finds it, keeps it while it is young, and removes it on
    // request after that.
    let (status, orphan) = entryway
        .create(&json!({"handle": "orphan.pds-users.org", "email": "orphan@example.com", "password": PASSWORD}))
        .await;
    assert_eq!(status, 200, "{orphan}");
    let orphan = text(&orphan, "did").to_owned();
    let entryway_db = common::open(&entryway.dir.config.layout.account_db);
    for table in ["account", "actor"] {
        entryway_db
            .execute(
                &format!(r#"delete from "{table}" where "did" = ?1"#),
                [&orphan],
            )
            .unwrap();
    }
    let now = linkjar_pds::serve::now_ms().unsigned_abs() * 1000;
    let report = linkjar_pds::reconcile::reconcile_at(&entryway.dir.config, true, now)
        .await
        .unwrap();
    assert_eq!(report.only_at_member, std::slice::from_ref(&orphan));
    assert_eq!(report.young, std::slice::from_ref(&orphan));
    assert_eq!(report.removed.len(), 0);
    assert!(member.dir.config.layout.actor_db(&orphan).exists());
    let later = now + 60 * 60 * 1_000_000;
    let report = linkjar_pds::reconcile::reconcile_at(&entryway.dir.config, true, later)
        .await
        .unwrap();
    assert_eq!(report.removed, std::slice::from_ref(&orphan));
    assert_eq!(report.only_at_entryway.len(), 0, "{report}");
    assert!(!member.dir.config.layout.actor_db(&orphan).exists());
    let report = linkjar_pds::reconcile::reconcile_at(&entryway.dir.config, false, later)
        .await
        .unwrap();
    assert!(report.is_clean(), "{report}");

    // Recovery at the credential owner also ends access at the member.
    let (status, _) = entryway
        .post(
            "com.atproto.server.requestPasswordReset",
            None,
            &json!({"email": "nora@example.com"}),
        )
        .await;
    assert_eq!(status, 200);
    let reset: String = common::open(&entryway.dir.config.layout.account_db).query_row(
        r#"select "token" from "email_token" where "did" = ?1 and "purpose" = 'reset_password'"#,
        [&did], |row| row.get(0),
    ).unwrap();
    assert_eq!(
        entryway
            .post(
                "com.atproto.server.resetPassword",
                None,
                &json!({"token": reset, "password": "recovered-at-entryway"})
            )
            .await
            .0,
        200
    );
    let (status, refused) = member
        .post("com.atproto.repo.createRecord", Some(&access), &record(5))
        .await;
    assert_eq!(
        (status, text(&refused, "error")),
        (400, "ExpiredToken"),
        "member accepted the entryway's revoked access token: {refused}"
    );
    let (status, recovered) = member
        .post(
            "com.atproto.server.createSession",
            None,
            &json!({"identifier": did, "password": "recovered-at-entryway"}),
        )
        .await;
    assert_eq!(status, 200, "{recovered}");
    let (status, written) = member
        .post(
            "com.atproto.repo.createRecord",
            Some(text(&recovered, "accessJwt")),
            &record(6),
        )
        .await;
    assert_eq!(status, 200, "{written}");

    // The admin of the entryway deletes the account on both servers.
    let gone = entryway
        .http
        .post(format!(
            "{}/xrpc/com.atproto.admin.deleteAccount",
            entryway.origin
        ))
        .basic_auth("admin", Some(common::ADMIN_PASSWORD))
        .header("content-type", "application/json")
        .body(json!({"did": did}).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(gone.status(), 200);
    assert_eq!(
        (rows(&entryway, "account"), rows(&member, "repo_root")),
        (0, 0)
    );
    assert!(!member.dir.config.layout.actor_db(&did).exists());
}
