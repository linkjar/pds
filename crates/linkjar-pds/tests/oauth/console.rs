use super::*;

/// A browser at the console: it keeps the cookie and follows no redirect.
struct Browser {
    origin: String,
    http: reqwest::Client,
    cookie: Option<String>,
}

impl Browser {
    async fn send(&mut self, request: reqwest::RequestBuilder) -> Answer {
        let request = match &self.cookie {
            Some(cookie) => request.header("cookie", cookie),
            None => request,
        };
        let response = request.send().await.unwrap();
        let headers = response.headers().clone();
        if let Some(set) = headers.get("set-cookie").and_then(|v| v.to_str().ok()) {
            let pair = set.split(';').next().unwrap().to_owned();
            self.cookie = (!pair.ends_with('=')).then_some(pair);
        }
        Answer {
            status: response.status().as_u16(),
            headers,
            text: response.text().await.unwrap(),
        }
    }

    async fn get(&mut self, path: &str) -> Answer {
        let request = self.http.get(format!("{}{path}", self.origin));
        self.send(request).await
    }

    async fn post(&mut self, path: &str, form: &[(&str, &str)]) -> Answer {
        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(form)
            .finish();
        let request = self
            .http
            .post(format!("{}{path}", self.origin))
            .header("content-type", "application/x-www-form-urlencoded")
            .header("origin", &self.origin)
            .body(body);
        self.send(request).await
    }

    /// Signs in through the pages of the PDS. Gives the answer of the
    /// console to the redirect.
    async fn sign_in(&mut self, server: &mut Server, console_id: &str) -> Answer {
        let start = self.get("/login").await;
        assert_eq!(start.status, 303, "{}", start.text);
        let to = reqwest::Url::parse(start.header("location").unwrap()).unwrap();
        assert!(
            to.as_str()
                .starts_with(&format!("{}/oauth/authorize?", server.issuer))
        );
        let request_uri = to
            .query_pairs()
            .find(|(key, _)| key == "request_uri")
            .unwrap()
            .1
            .into_owned();
        let back = server.authorize_as(console_id, &request_uri, false).await;
        assert!(back.as_str().starts_with(&self.origin), "{back}");
        self.get(&format!("{}?{}", back.path(), back.query().unwrap()))
            .await
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // one operator, from the sign-in to the sign-out
async fn an_operator_signs_in_to_the_console_and_acts_through_it() {
    // The console listens first: its address is in its client id, which
    // the PDS must know.
    let listener = common::http::bind().await;
    let console_origin = format!("http://{}", listener.local_addr().unwrap());
    let vars = |pds: &str, internal: &str, ops: &str| {
        [
            ("CONSOLE_PUBLIC_URL", console_origin.as_str()),
            ("CONSOLE_PDS_PUBLIC_URL", pds),
            ("CONSOLE_PDS_INTERNAL_URL", internal),
            ("CONSOLE_OPS_URL", ops),
        ]
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
    };
    let draft = pds_console::config::Config::from_vars(&vars(
        "https://pds.test",
        "http://127.0.0.1:1",
        "http://127.0.0.1:1",
    ))
    .unwrap();
    let console_id = draft.client_id();
    let mut server = Server::start_with(&[
        ("PDS_PROFILE", "linkjar"),
        ("PDS_ADMIN_PASSWORD", common::ADMIN_PASSWORD),
        ("PDS_OPS_CLIENT_IDS", &console_id),
    ])
    .await;
    let config = pds_console::config::Config::from_vars(&vars(
        &server.issuer,
        &server.origin,
        &server.ops_origin,
    ))
    .unwrap();
    assert_eq!(config.client_id(), console_id);
    let console = pds_console::Console::new(config).unwrap();
    tokio::spawn(async move {
        pds_console::serve(
            listener,
            pds_console::router(console),
            std::future::pending(),
        )
        .await
    });
    let mut browser = Browser {
        origin: console_origin.clone(),
        http: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap(),
        cookie: None,
    };

    // Without a session every page sends the browser to the sign-in page.
    let page = browser.get("/").await;
    assert_eq!(
        (page.status, page.header("location")),
        (303, Some("/sign-in"))
    );
    assert_eq!(browser.get("/health").await.text, "ok");
    let page = browser.get("/sign-in").await;
    assert!(page.text.contains(r#"href="/login""#));
    assert!(
        page.header("content-security-policy")
            .unwrap()
            .contains("default-src 'none'")
    );

    // An account without a role gets no session.
    let refused = browser.sign_in(&mut server, &console_id).await;
    assert_eq!(refused.status, 403, "{}", refused.text);
    assert!(refused.text.contains("holds no operator role"));
    assert!(browser.cookie.is_none());

    // With the role of an operator the console opens.
    let grant = json!({"did": ALICE, "role": "operator"});
    assert_eq!(
        server
            .admin("internal.ops.grantRole", "joins the team", &grant)
            .await,
        200
    );
    let signed = browser.sign_in(&mut server, &console_id).await;
    assert_eq!(
        (signed.status, signed.header("location")),
        (303, Some("/")),
        "{}",
        signed.text
    );
    assert!(signed.header("set-cookie").unwrap().contains("HttpOnly"));
    let home = browser.get("/").await;
    assert_eq!(home.status, 200, "{}", home.text);
    // The sidebar names the operator; the tiles hold the numbers of the
    // server; the live region asks the browser to connect for the stream.
    assert!(
        home.text.contains(&format!("<small>{ALICE}</small>")),
        "{}",
        home.text
    );
    assert!(home.text.contains(r#"data-tone="accent">operator</span>"#));
    assert!(
        home.text.contains(
            r#"<span data-slot="tile-label">Accounts</span><strong data-slot="tile-value">3"#
        ),
        "{}",
        home.text
    );
    assert!(home.text.contains("No subscriber is connected."));
    assert!(home.text.contains("::topcoat::connect"), "{}", home.text);
    assert!(
        home.text.contains("'unsafe-eval'")
            || home
                .header("content-security-policy")
                .unwrap()
                .contains("'unsafe-eval'")
    );
    let csrf = home.field("csrf").unwrap();

    // The live view: the browser's runtime socket to the console carries
    // the snapshot of the page, then a swap with the numbers of the stream
    // of the ops listener, which the console reads with the session.
    {
        use futures_util::{SinkExt as _, StreamExt as _};
        use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest as _};
        let mut request = format!("{}/", console_origin.replacen("http", "ws", 1))
            .into_client_request()
            .unwrap();
        let headers = request.headers_mut();
        headers.insert("sec-websocket-protocol", "topcoat-runtime".parse().unwrap());
        headers.insert("cookie", browser.cookie.clone().unwrap().parse().unwrap());
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        let run = json!({ "run": 1, "method": "GET", "path": "/", "headers": {}, "body": "" });
        socket
            .send(Message::Text(run.to_string().into()))
            .await
            .unwrap();
        let mut seen = Vec::new();
        let read = async {
            while let Some(Ok(Message::Text(text))) = socket.next().await {
                let envelope: Value = serde_json::from_str(&text).unwrap();
                let frame = envelope["frame"].clone();
                assert_ne!(frame["t"], "error", "{text}");
                let streaming = frame["t"] == "swap"
                    && frame["html"]
                        .as_str()
                        .is_some_and(|html| html.contains(r#"data-state="streaming""#));
                seen.push(frame);
                if streaming {
                    break;
                }
            }
        };
        tokio::time::timeout(std::time::Duration::from_secs(15), read)
            .await
            .expect("a swap of the live view");
        assert_eq!(seen[0]["t"], "snapshot", "{seen:?}");
        let last = seen.last().unwrap()["html"].as_str().unwrap();
        assert!(
            last.contains(
                r#"<span data-slot="tile-label">Accounts</span><strong data-slot="tile-value">3"#
            ),
            "{last}"
        );
    }

    // The pages.
    let list = browser.get("/accounts").await;
    assert!(
        list.text
            .contains(&format!("/accounts/{}", common::DANA.replace(':', "%3A")))
    );
    let dana = format!("/accounts/{}", common::DANA.replace(':', "%3A"));
    // A search by handle finds the account in the listing.
    let found = browser.get("/accounts?q=DANA").await;
    assert_eq!(found.status, 200, "{}", found.text);
    assert!(found.text.contains(&dana), "{}", found.text);
    assert!(
        !found
            .text
            .contains(&format!("/accounts/{}", ALICE.replace(':', "%3A")))
    );
    assert!(
        browser
            .get("/accounts?q=nobody")
            .await
            .text
            .contains("No account matches.")
    );
    let card = browser.get(&dana).await;
    assert_eq!(card.status, 200, "{}", card.text);
    assert!(card.text.contains(r#"value="takedown""#));
    assert!(browser.get("/roles").await.text.contains(ALICE));
    assert!(
        browser
            .get("/domains")
            .await
            .text
            .contains("No email domain is blocked.")
    );

    // An action needs the token of the session, a reason, and for a
    // takedown the DID typed again.
    let takedown = |csrf: &'static str, reason: &'static str, confirm: &'static str| {
        [
            ("csrf", csrf),
            ("action", "takedown"),
            ("did", common::DANA),
            ("reason", reason),
            ("confirm", confirm),
        ]
    };
    assert_eq!(
        browser
            .post("/act", &takedown("wrong", "spam", common::DANA))
            .await
            .status,
        403
    );
    let csrf: &'static str = Box::leak(csrf.into_boxed_str());
    assert_eq!(
        browser
            .post("/act", &takedown(csrf, "", common::DANA))
            .await
            .status,
        400
    );
    assert_eq!(
        browser
            .post("/act", &takedown(csrf, "spam", "did:plc:other"))
            .await
            .status,
        400
    );
    let done = browser
        .post("/act", &takedown(csrf, "spam links", common::DANA))
        .await;
    assert_eq!(done.status, 303, "{}", done.text);
    assert_eq!(
        done.header("location"),
        Some(format!("{dana}?done=takedown").as_str())
    );
    let card = browser.get(&format!("{dana}?done=takedown")).await;
    assert!(card.text.contains("The account is taken down."));
    assert!(card.text.contains(r#"value="reinstate""#));

    // The PDS wrote the entry: the operator, the client, the reason.
    let log = browser.get("/audit").await;
    assert_eq!(log.status, 200, "{}", log.text);
    assert!(log.text.contains("com.atproto.admin.updateSubjectStatus"));
    assert!(log.text.contains("spam links"));
    let entries = pds_xrpc::audit::AuditLog::open(&server.dir.config.layout.audit_db, None)
        .unwrap()
        .page(None, 1)
        .unwrap();
    assert_eq!(entries[0]["actor"], ALICE);
    assert_eq!(entries[0]["clientId"], console_id.as_str());
    assert_eq!(entries[0]["sidecar"], "console");
    assert_eq!(entries[0]["reason"], "spam links");

    // A blocked domain, through the form.
    let block = [
        ("csrf", csrf),
        ("action", "block-domain"),
        ("domain", "spam.example"),
        ("reason", "a wave"),
    ];
    let done = browser.post("/act", &block).await;
    assert_eq!(
        done.header("location"),
        Some("/domains?done=block-domain"),
        "{}",
        done.text
    );
    assert!(
        browser
            .get("/domains")
            .await
            .text
            .contains("<code>spam.example</code>")
    );

    // The sign-out ends the session at the console.
    let out = browser.post("/sign-out", &[("csrf", csrf)]).await;
    assert_eq!(
        (out.status, out.header("location")),
        (303, Some("/sign-in"))
    );
    assert!(browser.cookie.is_none());
    assert_eq!(browser.get("/").await.status, 303);
}
