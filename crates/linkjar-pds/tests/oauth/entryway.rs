use super::*;

#[tokio::test]
#[allow(clippy::too_many_lines)] // one client, at the entryway and at its member host
async fn a_client_authorized_at_the_entryway_writes_at_the_member_host() {
    let signing_hex = "0808080808080808080808080808080808080808080808080808080808080808";
    let signing = Keypair::from_secret_bytes(
        Algorithm::Secp256k1,
        &data_encoding::HEXLOWER
            .decode(signing_hex.as_bytes())
            .unwrap(),
    )
    .unwrap();
    let public = data_encoding::HEXLOWER.encode(&signing.public_key().to_compressed());
    let rotation_hex = "0707070707070707070707070707070707070707070707070707070707070707";
    let rotation = Keypair::from_secret_bytes(
        Algorithm::Secp256k1,
        &data_encoding::HEXLOWER
            .decode(rotation_hex.as_bytes())
            .unwrap(),
    )
    .unwrap();
    // The two servers have the account of the fixture: the entryway its
    // credentials, the member host its repository.
    let listener = common::http::bind().await;
    let port = listener.local_addr().unwrap().port().to_string();
    let mut entryway = Server::start_on(
        DataDir::new_with(
            1,
            &[
                ("PDS_HOSTNAME", "localhost"),
                ("PDS_PORT", &port),
                ("PDS_SERVICE_DID", "did:web:entryway.test"),
                ("PDS_MEMBER_URL", "https://pds.test"),
                ("PDS_MEMBER_ADMIN_PASSWORD", "member-admin"),
                (
                    "PDS_ENTRYWAY_JWT_SIGNING_KEY_K256_PRIVATE_KEY_HEX",
                    signing_hex,
                ),
                ("PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX", rotation_hex),
            ],
        ),
        listener,
    )
    .await;
    let mut member = Server::start_with(&[
        ("PDS_ENTRYWAY_URL", &entryway.issuer),
        ("PDS_ENTRYWAY_DID", "did:web:entryway.test"),
        ("PDS_ENTRYWAY_ADMIN_TOKEN", common::ADMIN_PASSWORD),
        ("PDS_ENTRYWAY_JWT_VERIFY_KEY_K256_PUBLIC_KEY_HEX", &public),
        ("PDS_ENTRYWAY_PLC_ROTATION_KEY", &rotation.did_key()),
    ])
    .await;
    assert_eq!(member.issuer, "https://pds.test");

    // The client learns from the member host where to be authorized.
    let resource = member
        .send(
            reqwest::Method::GET,
            "/.well-known/oauth-protected-resource",
            None,
            &[],
        )
        .await;
    assert_eq!(
        resource.json()["authorization_servers"],
        json!([entryway.issuer])
    );

    // The authorization at the entryway.
    entryway.push(&[]).await;
    let (_, tokens) = entryway.sign_in(false).await;
    assert_eq!(tokens["sub"], ALICE, "{tokens}");
    let access = tokens["access_token"].as_str().unwrap().to_owned();
    let key = client_key();
    // The entryway takes its own token.
    let own = entryway
        .resource(&key, reqwest::Method::GET, GET_SESSION, &access, None)
        .await;
    assert_eq!(own.status, 200, "{}", own.text);

    // The member host takes it too, with a proof for its own address and
    // a nonce of its own.
    let record = json!({"repo": ALICE, "collection": "com.example.note", "record": {"$type": "com.example.note"}});
    let write = "/xrpc/com.atproto.repo.createRecord";
    let first = member
        .resource(&key, reqwest::Method::POST, write, &access, Some(&record))
        .await;
    assert_eq!(first.status, 401, "{}", first.text);
    first
        .header("dpop-nonce")
        .unwrap()
        .clone_into(&mut member.nonce);
    let written = member
        .resource(&key, reqwest::Method::POST, write, &access, Some(&record))
        .await;
    assert_eq!(written.status, 200, "{}", written.text);
    assert!(
        written.json()["uri"]
            .as_str()
            .unwrap()
            .starts_with(&format!("at://{ALICE}/com.example.note/"))
    );

    // A token with another signature, and a proof of another key.
    let mut forged = access.clone();
    let last = forged.pop().unwrap();
    forged.push(if last == 'A' { 'B' } else { 'A' });
    let refused = member
        .resource(&key, reqwest::Method::POST, write, &forged, Some(&record))
        .await;
    assert_eq!(refused.status, 401, "{}", refused.text);
    let other = Keypair::generate(Algorithm::P256, |bytes| bytes.fill(8));
    let refused = member
        .resource(&other, reqwest::Method::POST, write, &access, Some(&record))
        .await;
    assert_eq!(refused.status, 401, "{}", refused.text);
    // Private status cannot be read by a client, including a client that
    // already holds a valid access token.
    let inspect = json!({"token": access, "kind": "oauth"}).to_string();
    for bearer in [None, Some(access.as_str())] {
        let mut request = entryway
            .http
            .post(format!("{}/oauth/introspect", entryway.origin))
            .header("content-type", "application/json")
            .body(inspect.clone());
        if let Some(bearer) = bearer {
            request = request.bearer_auth(bearer);
        }
        assert_eq!(request.send().await.unwrap().status(), 401);
    }
    assert_eq!(
        entryway
            .json_post(
                "com.atproto.server.requestPasswordReset",
                &json!({"email": "alice@example.com"})
            )
            .await
            .status,
        200
    );
    let reset: String = common::open(&entryway.dir.config.layout.account_db).query_row(
        r#"select "token" from "email_token" where "did" = ?1 and "purpose" = 'reset_password'"#,
        [ALICE], |row| row.get(0),
    ).unwrap();
    assert_eq!(
        entryway
            .json_post(
                "com.atproto.server.resetPassword",
                &json!({"token": reset, "password": "recovered-at-entryway"})
            )
            .await
            .status,
        200
    );
    let refused = member
        .resource(&key, reqwest::Method::POST, write, &access, Some(&record))
        .await;
    assert_eq!(
        refused.status, 401,
        "member accepted an OAuth token revoked at its entryway: {}",
        refused.text
    );
    entryway.password = "recovered-at-entryway".to_owned();
    let (_, recovered) = entryway.sign_in(false).await;
    let written = member
        .resource(
            &key,
            reqwest::Method::POST,
            write,
            recovered["access_token"].as_str().unwrap(),
            Some(&record),
        )
        .await;
    assert_eq!(written.status, 200, "{}", written.text);
}
