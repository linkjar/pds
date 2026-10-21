use super::*;

#[tokio::test]
#[allow(clippy::too_many_lines)] // one end-to-end protocol scenario
async fn email_tokens_confirm_an_address_change_it_and_reset_a_password() {
    let server = Server::start(&[]).await;
    let (_, made) = server
        .create(&json!({"handle": "karen.pds-users.org", "email": "karen@example.com", "password": PASSWORD}))
        .await;
    let (did, access) = (
        text(&made, "did").to_owned(),
        text(&made, "accessJwt").to_owned(),
    );
    let db = || common::open(&server.dir.root.join("account.sqlite"));
    let token = |purpose: &str| -> String {
        db().query_row(
            r#"select "token" from "email_token" where "did" = ?1 and "purpose" = ?2"#,
            [did.as_str(), purpose],
            |row| row.get(0),
        )
        .unwrap()
    };
    let call = |nsid: &'static str, body: Value| {
        let (server, access) = (&server, &access);
        async move { server.post(nsid, Some(access), &body).await }
    };
    let refused = |answer: (u16, Value)| (answer.0, text(&answer.1, "error").to_owned());

    // Confirm the address.
    assert_eq!(
        call("com.atproto.server.requestEmailConfirmation", Value::Null)
            .await
            .0,
        200
    );
    let confirm = token("confirm_email");
    assert_eq!(confirm.len(), 11);
    assert_eq!(confirm, confirm.to_uppercase());
    let confirm_email = "com.atproto.server.confirmEmail";
    assert_eq!(
        refused(
            call(
                confirm_email,
                json!({"email": "other@example.com", "token": confirm})
            )
            .await
        ),
        (400, "InvalidEmail".to_owned())
    );
    assert_eq!(
        refused(
            call(
                confirm_email,
                json!({"email": "karen@example.com", "token": "AAAAA-BBBBB"})
            )
            .await
        ),
        (400, "InvalidToken".to_owned())
    );
    // A token is read in any case, once.
    let (status, body) = call(
        confirm_email,
        json!({"email": "Karen@Example.com", "token": confirm.to_lowercase()}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        refused(
            call(
                confirm_email,
                json!({"email": "karen@example.com", "token": confirm})
            )
            .await
        ),
        (400, "InvalidToken".to_owned())
    );

    // Change the address: a confirmed one needs a token.
    let update = "com.atproto.server.updateEmail";
    let (_, asked) = call("com.atproto.server.requestEmailUpdate", Value::Null).await;
    assert_eq!(asked, json!({"tokenRequired": true}));
    assert_eq!(
        refused(call(update, json!({"email": "karen2@example.com"})).await),
        (400, "TokenRequired".to_owned())
    );
    let (status, body) = call(
        update,
        json!({"email": "alice@example.com", "token": token("update_email")}),
    )
    .await;
    assert_eq!(status, 400, "{body}");
    let (status, body) = call(
        update,
        json!({"email": "Karen2@example.com", "token": token("update_email")}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let (_, asked) = call("com.atproto.server.requestEmailUpdate", Value::Null).await;
    assert_eq!(asked, json!({"tokenRequired": false}));

    // Reset the password: a token past 15 minutes is expired, a fresh one
    // sets the password and ends the sessions.
    let reset = "com.atproto.server.resetPassword";
    let (status, body) = server
        .post(
            "com.atproto.server.requestPasswordReset",
            None,
            &json!({"email": "nobody@example.com"}),
        )
        .await;
    assert_eq!((status, body), (200, Value::Null));
    assert_eq!(
        server
            .post(
                "com.atproto.server.requestPasswordReset",
                None,
                &json!({"email": "KAREN2@example.com"})
            )
            .await
            .0,
        200
    );
    let first = token("reset_password");
    db().execute(
        r#"update "email_token" set "requestedAt" = '2000-01-01T00:00:00.000Z' where "purpose" = 'reset_password'"#,
        [],
    )
    .unwrap();
    assert_eq!(
        refused(
            server
                .post(
                    reset,
                    None,
                    &json!({"token": first, "password": "new-password"})
                )
                .await
        ),
        (400, "ExpiredToken".to_owned())
    );
    server
        .post(
            "com.atproto.server.requestPasswordReset",
            None,
            &json!({"email": "karen2@example.com"}),
        )
        .await;
    let (status, body) = server
        .post(
            reset,
            None,
            &json!({"token": token("reset_password"), "password": "new-password"}),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let login = |password: &'static str| {
        let body = json!({"identifier": did, "password": password});
        let server = &server;
        async move {
            server
                .post("com.atproto.server.createSession", None, &body)
                .await
        }
    };
    assert_eq!(login(PASSWORD).await.0, 401);
    assert_eq!(login("new-password").await.0, 200);
    let (status, body) = server
        .post(
            "com.atproto.server.refreshSession",
            Some(text(&made, "refreshJwt")),
            &Value::Null,
        )
        .await;
    assert_eq!((status, text(&body, "error")), (400, "ExpiredToken"));
}

#[tokio::test]
async fn recovery_prevents_an_in_flight_account_deletion() {
    let server = Server::start(&[]).await;
    let (_, made) = server
        .create(&json!({"handle": "delete-race.pds-users.org", "email": "delete-race@example.com", "password": PASSWORD}))
        .await;
    let did = text(&made, "did").to_owned();
    assert_eq!(
        server
            .post(
                "com.atproto.server.requestAccountDelete",
                Some(text(&made, "accessJwt")),
                &Value::Null
            )
            .await
            .0,
        200
    );
    let token: String = common::open(&server.dir.root.join("account.sqlite"))
        .query_row(
            r#"select "token" from "email_token" where "did" = ?1 and "purpose" = 'delete_account'"#,
            [&did], |row| row.get(0),
        ).unwrap();
    let state = server.state.clone();
    let recovered_did = did.clone();
    let (locked_tx, locked_rx) = tokio::sync::oneshot::channel();
    let (recover_tx, recover_rx) = std::sync::mpsc::channel();
    let recovery = tokio::task::spawn_blocking(move || {
        state.writes().unwrap().with_accounts(|db| {
            locked_tx.send(()).unwrap();
            recover_rx.recv().unwrap();
            let hash = pds_xrpc::password::hash_with_salt(
                "recovered-password",
                "0123456789abcdef0123456789abcdef",
            )
            .unwrap();
            db.update_password(&recovered_did, &hash, "recovered-epoch", None)
                .unwrap();
        });
    });
    locked_rx.await.unwrap();
    let request = server
        .http
        .post(format!(
            "{}/xrpc/com.atproto.server.deleteAccount",
            server.origin
        ))
        .header("content-type", "application/json")
        .body(json!({"did": did, "password": PASSWORD, "token": token}).to_string());
    let deletion = tokio::spawn(async move { request.send().await.unwrap() });
    // The request verifies the password and mail token on its read
    // connection, then waits for the account writer held above.
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    assert!(!deletion.is_finished());
    recover_tx.send(()).unwrap();
    recovery.await.unwrap();
    let response = deletion.await.unwrap();
    assert_eq!(
        response.status(),
        401,
        "stale credentials deleted the recovered account"
    );
    let (status, body) = server
        .post(
            "com.atproto.server.createSession",
            None,
            &json!({"identifier": did, "password": "recovered-password"}),
        )
        .await;
    assert_eq!(
        status, 200,
        "the recovered account must remain usable: {body}"
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // one end-to-end protocol scenario
async fn an_account_is_deleted_by_its_owner_and_by_the_admin() {
    use base64::Engine as _;
    let server = Server::start(&[]).await;
    let make = |name: &'static str| {
        let body = json!({"handle": format!("{name}.pds-users.org"),
                          "email": format!("{name}@example.com"), "password": PASSWORD});
        let server = &server;
        async move { server.create(&body).await.1 }
    };
    let (laura, nolan) = (make("laura").await, make("nolan").await);
    let did = text(&laura, "did").to_owned();
    let access = text(&laura, "accessJwt").to_owned();
    let events = |did: &str| -> Vec<String> {
        common::open(&server.dir.root.join("sequencer.sqlite"))
            .prepare(r#"select "eventType" from "repo_seq" where "did" = ?1 order by "seq""#)
            .unwrap()
            .query_map([did], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };

    assert_eq!(
        server
            .post(
                "com.atproto.server.requestAccountDelete",
                Some(&access),
                &Value::Null
            )
            .await
            .0,
        200
    );
    let token: String = common::open(&server.dir.root.join("account.sqlite"))
        .query_row(
            r#"select "token" from "email_token" where "did" = ?1 and "purpose" = 'delete_account'"#,
            [&did],
            |row| row.get(0),
        )
        .unwrap();
    let delete = "com.atproto.server.deleteAccount";
    let (status, body) = server
        .post(
            delete,
            None,
            &json!({"did": did, "password": "wrong", "token": token}),
        )
        .await;
    assert_eq!(
        (status, text(&body, "message")),
        (401, "Invalid did or password")
    );
    let (status, body) = server
        .post(
            delete,
            None,
            &json!({"did": did, "password": PASSWORD, "token": "AAAAA-BBBBB"}),
        )
        .await;
    assert_eq!((status, text(&body, "error")), (400, "InvalidToken"));
    let (status, body) = server
        .post(
            delete,
            None,
            &json!({"did": did, "password": PASSWORD, "token": token}),
        )
        .await;
    assert_eq!(status, 200, "{body}");

    // The account is gone: its rows, its store, and its events but one.
    assert!(!server.dir.config.layout.actor_store_dir(&did).exists());
    assert_eq!(events(&did), ["account"]);
    let (status, _) = server
        .post(
            "com.atproto.server.createSession",
            None,
            &json!({"identifier": did, "password": PASSWORD}),
        )
        .await;
    assert_eq!(status, 401);
    let (status, body) = server
        .post(
            delete,
            None,
            &json!({"did": did, "password": PASSWORD, "token": token}),
        )
        .await;
    assert_eq!((status, text(&body, "message")), (400, "account not found"));
    let report = linkjar_pds::verify::verify(&server.dir.config).unwrap();
    assert_eq!(report.findings.len(), 0, "{report}");

    // The admin deletes without a password and a token.
    let other = text(&nolan, "did").to_owned();
    let basic = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD
            .encode(format!("admin:{}", common::ADMIN_PASSWORD))
    );
    let response = server
        .http
        .post(format!(
            "{}/xrpc/com.atproto.admin.deleteAccount",
            server.origin
        ))
        .header("authorization", basic)
        .header("content-type", "application/json")
        .body(json!({"did": other}).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(events(&other), ["account"]);
    assert!(!server.dir.config.layout.actor_store_dir(&other).exists());
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // one end-to-end protocol scenario
async fn the_admin_changes_the_email_the_handle_and_the_password_of_an_account() {
    use base64::Engine as _;
    let server = Server::start(&[]).await;
    let (_, made) = server
        .create(&json!({"handle": "oscar.pds-users.org", "email": "oscar@example.com", "password": PASSWORD}))
        .await;
    let did = text(&made, "did").to_owned();
    let basic = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD
            .encode(format!("admin:{}", common::ADMIN_PASSWORD))
    );
    let admin = |nsid: &'static str, body: Value| {
        let request = server
            .http
            .post(format!("{}/xrpc/{nsid}", server.origin))
            .header("authorization", &basic)
            .header("content-type", "application/json")
            .body(body.to_string());
        async move {
            let response = request.send().await.unwrap();
            let status = response.status().as_u16();
            let bytes = response.bytes().await.unwrap();
            (status, common::http::json(&bytes))
        }
    };
    let login = |identifier: &'static str, password: &'static str| {
        let body = json!({"identifier": identifier, "password": password});
        let server = &server;
        async move {
            server
                .post("com.atproto.server.createSession", None, &body)
                .await
                .0
        }
    };

    let email = "com.atproto.admin.updateAccountEmail";
    assert_eq!(
        admin(
            email,
            json!({"account": "oscar.pds-users.org", "email": "Oscar2@example.com"})
        )
        .await
        .0,
        200
    );
    let (status, body) = admin(
        email,
        json!({"account": "nobody.pds-users.org", "email": "x@example.com"}),
    )
    .await;
    assert_eq!(
        (status, text(&body, "message")),
        (400, "Account does not exist: nobody.pds-users.org")
    );
    assert_eq!(login("oscar2@example.com", PASSWORD).await, 200);

    // The admin may give a reserved name; the account may not.
    let (status, body) = admin(
        "com.atproto.admin.updateAccountHandle",
        json!({"did": did, "handle": "admin.pds-users.org"}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(login("admin.pds-users.org", PASSWORD).await, 200);

    let password = "com.atproto.admin.updateAccountPassword";
    assert_eq!(
        admin(password, json!({"did": did, "password": "x".repeat(257)}))
            .await
            .0,
        400
    );
    assert_eq!(
        admin(
            password,
            json!({"did": did, "password": "changed-by-admin"})
        )
        .await
        .0,
        200
    );
    assert_eq!(login("oscar2@example.com", PASSWORD).await, 401);
    assert_eq!(login("oscar2@example.com", "changed-by-admin").await, 200);
    let revoked = server
        .http
        .get(format!(
            "{}/xrpc/com.atproto.server.getSession",
            server.origin
        ))
        .bearer_auth(text(&made, "accessJwt"))
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let revoked: Value = serde_json::from_slice(&revoked).unwrap();
    assert_eq!(revoked["error"], "ExpiredToken");
    let (status, _) = server
        .post(
            "com.atproto.server.refreshSession",
            Some(text(&made, "refreshJwt")),
            &Value::Null,
        )
        .await;
    assert_eq!(status, 400);
}
