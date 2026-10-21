use super::*;

const LINKJAR: &[(&str, &str)] = &[
    ("PDS_PROFILE", "linkjar"),
    ("PDS_SERVICE_HANDLE_DOMAINS", ".linkjar.social"),
];

#[tokio::test]
async fn the_linkjar_profile_reserves_names_and_allows_one_hosted_rename() {
    let server = Server::start(LINKJAR).await;
    let account = |label: &str| {
        json!({
            "handle": format!("{label}.linkjar.social"),
            "email": format!("{}@example.com", label.replace('-', "")),
            "password": PASSWORD,
        })
    };
    for label in [
        "linkjar", "pds2", "api-prod", "link-jar", "l1nkj4r", "supp0rt",
    ] {
        let (status, body) = server.create(&account(label)).await;
        assert_eq!(
            (status, text(&body, "error"), text(&body, "message")),
            (400, "HandleNotAvailable", "Reserved handle"),
            "{label}"
        );
    }
    let (status, _) = server.create(&account("jared")).await;
    assert_eq!(
        status, 200,
        "a name that only contains a service word is free"
    );

    let (status, created) = server.create(&account("fresh")).await;
    assert_eq!(status, 200, "{created}");
    let did = text(&created, "did").to_owned();
    let access = text(&created, "accessJwt").to_owned();
    let update = "com.atproto.identity.updateHandle";
    let rename = |handle: &str| json!({ "handle": format!("{handle}.linkjar.social") });
    // A password account has no policy row: it renames as upstream allows.
    for name in ["fresh2", "fresh3"] {
        let (status, body) = server.post(update, Some(&access), &rename(name)).await;
        assert_eq!(status, 200, "{name}: {body}");
    }
    // An account that an identity provider created has a row.
    let accounts = common::open(&server.dir.config.layout.account_db);
    let created_at = pds_types::iso_millis(
        i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap(),
    );
    accounts
        .execute(
            r#"insert into "linkjar_handle_policy" ("did", "createdAt") values (?1, ?2)"#,
            [&did, &created_at],
        )
        .unwrap();
    let (status, _) = server.post(update, Some(&access), &rename("fresh3")).await;
    assert_eq!(status, 200, "an unchanged handle spends nothing");
    let (status, body) = server.post(update, Some(&access), &rename("chosen")).await;
    assert_eq!(status, 200, "{body}");
    let (status, _) = server.post(update, Some(&access), &rename("chosen")).await;
    assert_eq!(status, 200, "the chosen target stays available");
    let (status, body) = server.post(update, Some(&access), &rename("another")).await;
    assert_eq!(
        (status, text(&body, "error")),
        (400, "HandleNotAvailable"),
        "{body}"
    );
    assert!(text(&body, "message").starts_with("One hosted handle change is available"));
    let claimed: Option<String> = accounts
        .query_row(
            r#"select "renameHandle" from "linkjar_handle_policy" where "did" = ?1"#,
            [&did],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(claimed.as_deref(), Some("chosen.linkjar.social"));
    let migrations: i64 = accounts
        .query_row(
            r#"select count(*) from "kysely_migration" where "name" like '007_-linkjar-%'"#,
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(migrations, 4);
}

#[tokio::test]
async fn a_handle_host_serves_the_did_and_a_redirect_and_nothing_else() {
    let server = Server::start(LINKJAR).await;
    let (status, created) = server
        .create(&json!({"handle": "nova.linkjar.social", "email": "nova@example.com", "password": PASSWORD}))
        .await;
    assert_eq!(status, 200, "{created}");
    let did = text(&created, "did").to_owned();
    let get = |host: &'static str, path: &'static str| {
        let request = server
            .http
            .get(format!("{}{path}", server.origin))
            .header("host", host)
            .header("cookie", "session=x");
        async move {
            let response = request.send().await.unwrap();
            let status = response.status().as_u16();
            let headers = response.headers().clone();
            (status, headers, response.text().await.unwrap())
        }
    };
    let host = "nova.linkjar.social";
    let (status, headers, body) = get(host, "/.well-known/atproto-did").await;
    assert_eq!((status, body.as_str()), (200, did.as_str()));
    // The proxy in front adds the headers to the answers it forwards.
    assert!(headers.get("content-security-policy").is_none());
    for path in [
        "/.well-known/oauth-authorization-server",
        "/xrpc/com.atproto.server.describeServer",
        "/xrpc/_health",
    ] {
        let (status, headers, body) = get(host, path).await;
        assert_eq!(status, 404, "{path}");
        assert_eq!(body, "Use the PDS service endpoint at pds.test", "{path}");
        assert_eq!(
            headers["content-security-policy"],
            "default-src 'none'; sandbox"
        );
        assert_eq!(headers["x-content-type-options"], "nosniff");
        assert_eq!(headers["referrer-policy"], "no-referrer");
    }
    let page = format!("https://linkjar.io/jar/{}", did.replace(':', "%3A"));
    for path in [
        "/",
        "/anything/else?x=1",
        "/oauth/authorize",
        "/account",
        "/.well-known/linkjar-profile",
    ] {
        let (status, headers, _) = get(host, path).await;
        assert_eq!(status, 302, "{path}");
        assert_eq!(headers["location"].to_str().unwrap(), page, "{path}");
        assert!(headers.get("set-cookie").is_none());
    }
    // A handle nobody holds, a reserved one, and the service host.
    assert_eq!(get("nobody.linkjar.social", "/").await.0, 404);
    assert_eq!(get("support.linkjar.social", "/").await.0, 404);
    let (status, _, body) = get("pds.test", "/").await;
    assert_eq!(status, 200);
    assert!(body.contains("AT Protocol Personal Data Server"));
    assert_eq!(get("pds.test", "/.well-known/linkjar-profile").await.0, 404);
}

#[tokio::test]
async fn a_configured_captcha_closes_the_legacy_sign_up_in_the_linkjar_profile() {
    let captcha = [
        ("PDS_HCAPTCHA_SITE_KEY", "site"),
        ("PDS_HCAPTCHA_SECRET_KEY", "secret"),
        ("PDS_HCAPTCHA_TOKEN_SALT", "salt"),
    ];
    let account = json!({"handle": "fresh.linkjar.social", "email": "fresh@example.com", "password": PASSWORD});
    let mut vars = LINKJAR.to_vec();
    vars.extend_from_slice(&captcha);
    let server = Server::start(&vars).await;
    let (status, body) = server.create(&account).await;
    assert_eq!(
        (status, text(&body, "error"), text(&body, "message")),
        (
            400,
            "InvalidRequest",
            "Create an account through the OAuth signup page"
        )
    );
    // The stock profile keeps the method open, as upstream does.
    let mut vars = vec![("PDS_SERVICE_HANDLE_DOMAINS", ".linkjar.social")];
    vars.extend_from_slice(&captcha);
    let stock = Server::start(&vars).await;
    assert_eq!(stock.create(&account).await.0, 200);
}
