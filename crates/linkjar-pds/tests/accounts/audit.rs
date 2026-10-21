use super::*;

#[tokio::test]
async fn account_and_admin_actions_are_in_an_audit_log_that_verifies() {
    use base64::Engine as _;
    let server = Server::start(&[
        ("PDS_ADMIN_PASSWORD", common::ADMIN_PASSWORD),
        (
            "PDS_AUDIT_KEY_ED25519_PRIVATE_KEY_HEX",
            "0909090909090909090909090909090909090909090909090909090909090909",
        ),
    ])
    .await;
    let account =
        json!({"handle": "judy.pds-users.org", "email": "judy@example.com", "password": PASSWORD});
    let (status, made) = server.create(&account).await;
    assert_eq!(status, 200, "{made}");
    let did = text(&made, "did").to_owned();
    // A refused creation, a repository write and an admin action.
    assert_eq!(server.create(&account).await.0, 400);
    let record = json!({"repo": did, "collection": "com.example.note", "record": {"$type": "com.example.note"}});
    let (status, _) = server
        .post(
            "com.atproto.repo.createRecord",
            Some(text(&made, "accessJwt")),
            &record,
        )
        .await;
    assert_eq!(status, 200);
    let basic = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD
            .encode(format!("admin:{}", common::ADMIN_PASSWORD))
    );
    let takedown = json!({
        "subject": {"$type": "com.atproto.admin.defs#repoRef", "did": did},
        "takedown": {"applied": true, "ref": "case-1"},
    });
    let response = server
        .http
        .post(format!(
            "{}/xrpc/com.atproto.admin.updateSubjectStatus",
            server.origin
        ))
        .header("authorization", &basic)
        .header("content-type", "application/json")
        .body(takedown.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);

    // The log has the three audited actions and not the repository write.
    let key = pds_audit::Signer::from_seed(&[9; 32]);
    let log = pds_xrpc::audit::AuditLog::open(&server.dir.config.layout.audit_db, key).unwrap();
    log.checkpoint("2026-10-13T00:00:00.000Z").unwrap().unwrap();
    let bundle = log.export(1, u64::MAX).unwrap();
    let entries: Vec<pds_audit::Entry> = bundle["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(entry.as_str().unwrap())
                .unwrap();
            pds_audit::Entry::from_bytes(&bytes).unwrap()
        })
        .collect();
    let seen: Vec<(&str, &str, Option<&str>, &str)> = entries
        .iter()
        .map(|entry| {
            let what = &entry.what;
            (
                what.action.as_str(),
                what.actor.as_str(),
                what.subject.as_deref(),
                what.outcome.as_str(),
            )
        })
        .collect();
    let key_id = seen_key(&entries);
    assert!(key_id.is_some_and(|id| id.starts_with("ed25519:")));
    let created = "com.atproto.server.createAccount";
    assert_eq!(
        seen,
        [
            (created, "anonymous", Some(did.as_str()), "ok"),
            (created, "anonymous", None, "InvalidRequest"),
            (
                "com.atproto.admin.updateSubjectStatus",
                "admin",
                Some(did.as_str()),
                "ok"
            ),
            ("audit.key", "server", key_id, "ok"),
        ]
    );
    let report = pds_audit::verify::verify(&bundle);
    assert!(report.ok(), "{:?}", report.problems);
    assert_eq!((report.entries, report.checkpoints), (4, 1));
}

/// The subject of the key entry: the id of the key, whatever it is.
fn seen_key(entries: &[pds_audit::Entry]) -> Option<&str> {
    entries
        .iter()
        .find(|entry| entry.what.action == "audit.key")
        .and_then(|entry| entry.what.subject.as_deref())
}

#[tokio::test]
async fn a_checkpoint_is_published_as_a_record_of_the_operator() {
    let dir = DataDir::new_with(
        1,
        &[
            (
                "PDS_AUDIT_KEY_ED25519_PRIVATE_KEY_HEX",
                "0909090909090909090909090909090909090909090909090909090909090909",
            ),
            ("PDS_AUDIT_CHECKPOINT_REPO", common::ALICE),
        ],
    );
    let state = app_state(&dir.config).unwrap();
    let action = |name: &str| pds_audit::Draft {
        actor: "admin".to_owned(),
        action: name.to_owned(),
        outcome: "ok".to_owned(),
        ..pds_audit::Draft::default()
    };
    pds_xrpc::audit::record(&state, action("first")).await;
    pds_xrpc::audit::checkpoint(&state).await;
    // Nothing new: no second checkpoint and no second record.
    pds_xrpc::audit::checkpoint(&state).await;
    pds_xrpc::audit::record(&state, action("second")).await;
    pds_xrpc::audit::checkpoint(&state).await;

    let store = pds_storage::ActorDb::open(&dir.config.layout.actor_db(common::ALICE)).unwrap();
    let records = store
        .list_records(
            common::ALICE,
            "io.linkjar.pds.audit.checkpoint",
            10,
            true,
            None,
        )
        .unwrap();
    assert_eq!(records.len(), 2);
    let fields = |row: &pds_storage::RecordRow| match pds_types::Data::from_dag_cbor(&row.content) {
        Ok(pds_types::Data::Object(fields)) => fields,
        other => panic!("not a record: {other:?}"),
    };
    let (first, second) = (fields(&records[0]), fields(&records[1]));
    let number =
        |fields: &std::collections::BTreeMap<String, pds_types::Data>, key: &str| match fields
            .get(key)
        {
            Some(pds_types::Data::Integer(n)) => *n,
            other => panic!("{key}: {other:?}"),
        };
    let bytes =
        |fields: &std::collections::BTreeMap<String, pds_types::Data>, key: &str| match fields
            .get(key)
        {
            Some(pds_types::Data::Bytes(bytes)) => bytes.0.clone(),
            other => panic!("{key}: {other:?}"),
        };
    // The first covers the entry and the entry of its key.
    assert_eq!(
        (number(&first, "firstSeq"), number(&first, "lastSeq")),
        (1, 2)
    );
    assert_eq!(
        (number(&second, "firstSeq"), number(&second, "lastSeq")),
        (3, 3)
    );
    assert_eq!(bytes(&first, "previousRoot"), vec![0; 32]);
    assert_eq!(bytes(&second, "previousRoot"), bytes(&first, "root"));
    assert_eq!(bytes(&first, "signature").len(), 64);
    // The record went out on the firehose as a commit of the repository.
    let events: i64 = common::open(&dir.config.layout.sequencer_db)
        .query_row(
            r#"select count(*) from "repo_seq" where "did" = ?1 and "eventType" = 'append'"#,
            [common::ALICE],
            |row| row.get(0),
        )
        .unwrap();
    assert!(events >= 2, "{events}");
}
