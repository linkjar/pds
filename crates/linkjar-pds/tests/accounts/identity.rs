use super::*;

#[tokio::test]
async fn recovery_prevents_plc_signing_after_a_slow_directory_response() {
    let gate = Arc::new(LastOperationGate::default());
    let (plc, ops) = plc_with_last_gate(Some(Arc::clone(&gate))).await;
    let listener = common::http::bind().await;
    let server = Server::start_on(listener, &plc, ops, &[]).await;
    let (_, made) = server
        .create(&json!({"handle": "plc-race.pds-users.org", "email": "plc-race@example.com", "password": PASSWORD}))
        .await;
    let did = text(&made, "did").to_owned();
    assert_eq!(
        server
            .post(
                "com.atproto.identity.requestPlcOperationSignature",
                Some(text(&made, "accessJwt")),
                &Value::Null
            )
            .await
            .0,
        200
    );
    let token: String = common::open(&server.dir.root.join("account.sqlite"))
        .query_row(
            r#"select "token" from "email_token" where "did" = ?1 and "purpose" = 'plc_operation'"#,
            [&did],
            |row| row.get(0),
        )
        .unwrap();
    let request = server
        .http
        .post(format!(
            "{}/xrpc/com.atproto.identity.signPlcOperation",
            server.origin
        ))
        .bearer_auth(text(&made, "accessJwt"))
        .header("content-type", "application/json")
        .body(
            json!({"token": token,
            "rotationKeys": ["did:key:zQ3shokFTS3brHcDQrn82RUDfCZESWL1ZdCEJwekUDPQiYBme"]})
            .to_string(),
        );
    let signing = tokio::spawn(async move { request.send().await.unwrap() });
    tokio::time::timeout(std::time::Duration::from_secs(5), gate.entered.notified())
        .await
        .unwrap();
    let recovered = server
        .http
        .post(format!(
            "{}/xrpc/com.atproto.admin.updateAccountPassword",
            server.origin
        ))
        .basic_auth("admin", Some(common::ADMIN_PASSWORD))
        .header("content-type", "application/json")
        .body(json!({"did": did, "password": "recovered-password"}).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(recovered.status(), 200);
    gate.release.notify_one();
    let response = signing.await.unwrap();
    assert_eq!(
        response.status(),
        400,
        "stale authorization signed a new PLC operation after recovery"
    );
    let body: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    assert_eq!(body["error"], "ExpiredToken");
    assert!(body.get("operation").is_none());
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // one end-to-end protocol scenario
async fn a_plc_operation_is_recommended_signed_and_submitted() {
    let server = Server::start(&[]).await;
    let (_, made) = server
        .create(&json!({"handle": "petra.pds-users.org", "email": "petra@example.com", "password": PASSWORD}))
        .await;
    let (did, access) = (
        text(&made, "did").to_owned(),
        text(&made, "accessJwt").to_owned(),
    );
    let genesis = server.ops.lock().unwrap()[0].1.clone();

    let response = server
        .http
        .get(format!(
            "{}/xrpc/com.atproto.identity.getRecommendedDidCredentials",
            server.origin
        ))
        .bearer_auth(&access)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let recommended: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    assert_eq!(recommended["alsoKnownAs"], genesis["alsoKnownAs"]);
    assert_eq!(
        recommended["verificationMethods"],
        genesis["verificationMethods"]
    );
    assert_eq!(recommended["rotationKeys"], genesis["rotationKeys"]);
    assert_eq!(recommended["services"], genesis["services"]);

    // Signing needs the token of the mail.
    let sign = "com.atproto.identity.signPlcOperation";
    let (status, body) = server.post(sign, Some(&access), &json!({})).await;
    assert_eq!(
        (status, text(&body, "message")),
        (
            400,
            "email confirmation token required to sign PLC operations"
        )
    );
    assert_eq!(
        server
            .post(
                "com.atproto.identity.requestPlcOperationSignature",
                Some(&access),
                &Value::Null
            )
            .await
            .0,
        200
    );
    let token: String = common::open(&server.dir.root.join("account.sqlite"))
        .query_row(
            r#"select "token" from "email_token" where "did" = ?1 and "purpose" = 'plc_operation'"#,
            [&did],
            |row| row.get(0),
        )
        .unwrap();
    let extra = "did:key:zQ3shokFTS3brHcDQrn82RUDfCZESWL1ZdCEJwekUDPQiYBme";
    let mut keys = vec![json!(extra)];
    keys.extend(genesis["rotationKeys"].as_array().unwrap().iter().cloned());
    let (status, signed) = server
        .post(
            sign,
            Some(&access),
            &json!({"token": token, "rotationKeys": keys}),
        )
        .await;
    assert_eq!(status, 200, "{signed}");
    let operation = signed["operation"].clone();
    assert_eq!(operation["rotationKeys"][0], extra);
    assert_eq!(operation["alsoKnownAs"], genesis["alsoKnownAs"]);
    assert!(operation["prev"].is_string() && operation["sig"].is_string());
    // The token is used up.
    let (status, body) = server
        .post(sign, Some(&access), &json!({"token": token}))
        .await;
    assert_eq!((status, text(&body, "error")), (400, "InvalidToken"));

    // Submitting keeps the account on this server.
    let submit = "com.atproto.identity.submitPlcOperation";
    let with = |change: &dyn Fn(&mut Value)| {
        let mut op = operation.clone();
        change(&mut op);
        json!({ "operation": op })
    };
    for (body, message) in [
        (
            json!({"operation": {"type": "plc_tombstone"}}),
            "Invalid operation",
        ),
        (
            with(&|op| op["rotationKeys"] = json!([extra])),
            "Rotation keys do not include server's rotation key",
        ),
        (
            with(&|op| op["services"]["atproto_pds"]["endpoint"] = json!("https://elsewhere.test")),
            "Incorrect endpoint on atproto_pds service",
        ),
        (
            with(&|op| op["verificationMethods"]["atproto"] = json!(extra)),
            "Incorrect signing key",
        ),
        (
            with(&|op| op["alsoKnownAs"] = json!(["at://someone.else"])),
            "Incorrect handle in alsoKnownAs",
        ),
    ] {
        let (status, answer) = server.post(submit, Some(&access), &body).await;
        assert_eq!((status, text(&answer, "message")), (400, message));
    }
    let (status, body) = server.post(submit, Some(&access), &with(&|_| {})).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(server.ops.lock().unwrap().len(), 2);
    let last: String = common::open(&server.dir.root.join("sequencer.sqlite"))
        .query_row(
            r#"select "eventType" from "repo_seq" where "did" = ?1 order by "seq" desc limit 1"#,
            [&did],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(last, "identity");
}

#[tokio::test]
async fn a_signing_key_is_reserved_once_for_a_did() {
    let server = Server::start(&[]).await;
    let reserve = "com.atproto.server.reserveSigningKey";
    let did = json!({"did": "did:plc:aaaaaaaaaaaaaaaaaaaaaaaz"});
    let (status, first) = server.post(reserve, None, &did).await;
    assert_eq!(status, 200, "{first}");
    let key = first["signingKey"].as_str().unwrap();
    assert!(key.starts_with("did:key:zQ3s"), "{key}");
    let (_, again) = server.post(reserve, None, &did).await;
    assert_eq!(again["signingKey"], first["signingKey"]);
    // Without a DID each call reserves another key.
    let (_, one) = server.post(reserve, None, &json!({})).await;
    let (_, two) = server.post(reserve, None, &json!({})).await;
    assert_ne!(one["signingKey"], two["signingKey"]);
    let (status, body) = server.post(reserve, None, &json!({"did": "../x"})).await;
    assert_eq!(
        (status, body["error"].as_str()),
        (400, Some("InvalidRequest"))
    );
}
