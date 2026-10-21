use super::*;

/// An AI client of the MCP server, with the browser of its person.
struct AiClient {
    /// Where the MCP server listens.
    origin: String,
    http: reqwest::Client,
    cookie: Option<String>,
    serial: u32,
}

impl AiClient {
    async fn send(&mut self, request: reqwest::RequestBuilder) -> Answer {
        let request = match &self.cookie {
            Some(cookie) => request.header("cookie", cookie),
            None => request,
        };
        let response = request.send().await.unwrap();
        let headers = response.headers().clone();
        if let Some(set) = headers.get("set-cookie").and_then(|v| v.to_str().ok()) {
            self.cookie = Some(set.split(';').next().unwrap().to_owned());
        }
        Answer {
            status: response.status().as_u16(),
            headers,
            text: response.text().await.unwrap(),
        }
    }

    async fn form(&mut self, path: &str, form: &[(&str, &str)]) -> Answer {
        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(form)
            .finish();
        let request = self
            .http
            .post(format!("{}{path}", self.origin))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(body);
        self.send(request).await
    }

    /// One JSON-RPC request to the MCP endpoint.
    async fn rpc(&mut self, token: Option<&str>, method: &str, params: Value) -> Answer {
        self.serial += 1;
        let message =
            json!({"jsonrpc": "2.0", "id": self.serial, "method": method, "params": params});
        let mut request = self
            .http
            .post(format!("{}/mcp", self.origin))
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .body(message.to_string());
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        self.send(request).await
    }

    /// The result of a tool call.
    async fn tool(&mut self, token: &str, name: &str, arguments: Value) -> Value {
        let answer = self
            .rpc(
                Some(token),
                "tools/call",
                json!({"name": name, "arguments": arguments}),
            )
            .await;
        assert_eq!(answer.status, 200, "{}", answer.text);
        answer.json()["result"].clone()
    }
}

const NOTE: &str = "com.example.note";
const AI_REDIRECT: &str = "http://127.0.0.1/ai-callback";
const MCP_PUBLIC: &str = "https://mcp.example.com";

/// The authorization of an AI client up to its code: the request, the
/// pages of the PDS, and the consent page of the MCP server.
async fn mcp_authorize(
    ai: &mut AiClient,
    server: &mut Server,
    sidecar_id: &str,
    client_id: &str,
    scope: &str,
    decision: &str,
) -> reqwest::Url {
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes()));
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("response_type", "code")
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", AI_REDIRECT)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", "ai-state")
        .append_pair("scope", scope)
        .append_pair("resource", &format!("{MCP_PUBLIC}/mcp"))
        .finish();
    let url = format!("{}/authorize?{query}", ai.origin);
    let back = mcp_browser(
        ai, server, sidecar_id, &url, "Test AI", MCP_PUBLIC, decision,
    )
    .await;
    assert!(back.as_str().starts_with(AI_REDIRECT), "{back}");
    back
}

/// The browser of the person from the authorization URL of an AI client
/// to the redirect back to it: the pages of the PDS, and the consent page
/// of the MCP server, which names the client `name`. `public` is the
/// public address of the MCP server.
async fn mcp_browser(
    ai: &mut AiClient,
    server: &mut Server,
    sidecar_id: &str,
    url: &str,
    name: &str,
    public: &str,
    decision: &str,
) -> reqwest::Url {
    let request = ai.http.get(url);
    let start = ai.send(request).await;
    assert_eq!(start.status, 303, "{}", start.text);
    let to = reqwest::Url::parse(start.header("location").unwrap()).unwrap();
    assert!(
        to.as_str()
            .starts_with(&format!("{}/oauth/authorize?", server.issuer)),
        "{to}"
    );
    let request_uri = to
        .query_pairs()
        .find(|(key, _)| key == "request_uri")
        .unwrap()
        .1
        .into_owned();
    let back = server.authorize_as(sidecar_id, &request_uri, false).await;
    assert!(
        back.as_str()
            .starts_with(&format!("{public}/oauth/pds-callback?")),
        "{back}"
    );
    let path = format!("{}?{}", back.path(), back.query().unwrap());

    // Another browser cannot finish the sign-in.
    let stranger = ai
        .http
        .get(format!("{}{path}", ai.origin))
        .send()
        .await
        .unwrap();
    assert_eq!(stranger.status(), 400);

    // The MCP server asks about the AI client, which the PDS did not name.
    let request = ai.http.get(format!("{}{path}", ai.origin));
    let consent = ai.send(request).await;
    assert_eq!(consent.status, 200, "{}", consent.text);
    assert!(
        consent.text.contains(&format!("<strong>{name}</strong>")),
        "{}",
        consent.text
    );
    assert!(consent.text.contains(&format!("<code>{ALICE}</code>")));
    assert!(consent.text.contains("<code>127.0.0.1</code>"));
    assert!(
        consent
            .header("content-security-policy")
            .unwrap()
            .contains("frame-ancestors 'none'")
    );
    let request_id = consent.field("request").unwrap();
    let csrf = consent.field("csrf").unwrap();

    // The form needs its token.
    let forged = [
        ("request", request_id.as_str()),
        ("csrf", "wrong"),
        ("decision", "allow"),
    ];
    assert_eq!(ai.form("/authorize/decision", &forged).await.status, 403);
    let form = [
        ("request", request_id.as_str()),
        ("csrf", csrf.as_str()),
        ("decision", decision),
    ];
    let decided = ai.form("/authorize/decision", &form).await;
    assert_eq!(decided.status, 303, "{}", decided.text);
    reqwest::Url::parse(decided.header("location").unwrap()).unwrap()
}

fn parameter(url: &reqwest::Url, name: &str) -> Option<String> {
    url.query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // one AI client, from its registration to the end of its session
async fn an_ai_client_reaches_a_repository_through_the_mcp_server() {
    // The MCP server is a client that the PDS knows without a fetch.
    let listener = common::http::bind().await;
    let mcp_origin = format!("http://{}", listener.local_addr().unwrap());
    let vars = |pds: &str, internal: &str| -> std::collections::BTreeMap<String, String> {
        [
            ("MCP_PUBLIC_URL", MCP_PUBLIC),
            ("MCP_PDS_PUBLIC_URL", pds),
            ("MCP_PDS_INTERNAL_URL", internal),
            ("MCP_WRITE_COLLECTIONS", NOTE),
            (
                "MCP_CLIENT_KEY_P256_PRIVATE_KEY_HEX",
                "0606060606060606060606060606060606060606060606060606060606060606",
            ),
        ]
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
    };
    let draft = pds_mcp::config::Config::from_vars(&vars("https://pds.test", "http://127.0.0.1:1"))
        .unwrap();
    let sidecar_id = draft.client_id();
    let known = std::env::temp_dir().join(format!(
        "linkjar-pds-known-clients-{}.json",
        std::process::id()
    ));
    let mut clients = serde_json::Map::new();
    clients.insert(sidecar_id.clone(), draft.client_metadata());
    std::fs::write(&known, Value::Object(clients).to_string()).unwrap();
    let mut server = Server::start_with(&[
        ("PDS_OAUTH_KNOWN_CLIENTS_FILE", known.to_str().unwrap()),
        ("PDS_MCP_CLIENT_IDS", &sidecar_id),
    ])
    .await;
    let config = pds_mcp::config::Config::from_vars(&vars(&server.issuer, &server.origin)).unwrap();
    let mcp = pds_mcp::Mcp::new(config).unwrap();
    tokio::spawn(async move { axum::serve(listener, pds_mcp::router(mcp)).await });
    let mut ai = AiClient {
        origin: mcp_origin,
        http: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap(),
        cookie: None,
        serial: 0,
    };

    // Discovery: no token, the challenge, the two metadata documents.
    let challenge = ai.rpc(None, "initialize", json!({})).await;
    assert_eq!(challenge.status, 401);
    let metadata_url = format!("{MCP_PUBLIC}/.well-known/oauth-protected-resource");
    assert_eq!(
        challenge.header("www-authenticate"),
        Some(format!(r#"Bearer resource_metadata="{metadata_url}""#).as_str())
    );
    let request = ai.http.get(format!(
        "{}/.well-known/oauth-protected-resource",
        ai.origin
    ));
    let resource = ai.send(request).await.json();
    assert_eq!(resource["resource"], format!("{MCP_PUBLIC}/mcp"));
    assert_eq!(resource["authorization_servers"], json!([MCP_PUBLIC]));
    let request = ai.http.get(format!(
        "{}/.well-known/oauth-authorization-server",
        ai.origin
    ));
    let issuer = ai.send(request).await.json();
    assert_eq!(
        issuer["registration_endpoint"],
        format!("{MCP_PUBLIC}/register")
    );
    assert_eq!(issuer["code_challenge_methods_supported"], json!(["S256"]));
    assert!(
        issuer["scopes_supported"]
            .as_array()
            .unwrap()
            .contains(&json!("repo:write:com.example.note"))
    );

    // Registration.
    let request = ai
        .http
        .post(format!("{}/register", ai.origin))
        .header("content-type", "application/json")
        .body(json!({"client_name": "Test AI", "redirect_uris": [AI_REDIRECT]}).to_string());
    let registered = ai.send(request).await;
    assert_eq!(registered.status, 201, "{}", registered.text);
    let client_id = registered.json()["client_id"].as_str().unwrap().to_owned();

    // A person who says no gives the client nothing.
    let scope = "account:read repo:read repo:write:com.example.note";
    let denied = mcp_authorize(&mut ai, &mut server, &sidecar_id, &client_id, scope, "deny").await;
    assert_eq!(
        parameter(&denied, "error").as_deref(),
        Some("access_denied")
    );
    assert_eq!(parameter(&denied, "state").as_deref(), Some("ai-state"));

    // A person who allows it gives a code, which is good once with the verifier.
    let back = mcp_authorize(
        &mut ai,
        &mut server,
        &sidecar_id,
        &client_id,
        scope,
        "allow",
    )
    .await;
    assert_eq!(parameter(&back, "iss").as_deref(), Some(MCP_PUBLIC));
    let code = parameter(&back, "code").unwrap();
    let exchange = |verifier: &'static str, code: String, client_id: String| {
        vec![
            ("grant_type", "authorization_code".to_owned()),
            ("code", code),
            ("code_verifier", verifier.to_owned()),
            ("redirect_uri", AI_REDIRECT.to_owned()),
            ("client_id", client_id),
        ]
    };
    let borrowed =
        |form: &[(&'static str, String)]| -> Vec<(&'static str, String)> { form.to_vec() };
    let form = borrowed(&exchange(VERIFIER, code.clone(), client_id.clone()));
    let form: Vec<(&str, &str)> = form.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let tokens = ai.form("/token", &form).await;
    assert_eq!(tokens.status, 200, "{}", tokens.text);
    let tokens = tokens.json();
    assert_eq!(tokens["token_type"], "Bearer");
    assert_eq!(tokens["scope"], scope);
    let access = tokens["access_token"].as_str().unwrap().to_owned();
    let refresh = tokens["refresh_token"].as_str().unwrap().to_owned();
    assert_eq!(ai.form("/token", &form).await.status, 400);

    // The protocol.
    let hello = ai
        .rpc(Some(&access), "initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "0"}}))
        .await;
    assert_eq!(hello.status, 200, "{}", hello.text);
    assert_eq!(hello.json()["result"]["protocolVersion"], "2025-06-18");
    let listed = ai.rpc(Some(&access), "tools/list", json!({})).await.json();
    assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 8);
    let unknown = ai.rpc(Some(&access), "no/such", json!({})).await.json();
    assert_eq!(unknown["error"]["code"], -32601);

    // Reads.
    let who = ai.tool(&access, "whoami", json!({})).await;
    assert_eq!(who["structuredContent"]["did"], ALICE, "{who}");
    let collections = ai.tool(&access, "list_collections", json!({})).await;
    assert!(
        collections["structuredContent"]["collections"]
            .as_array()
            .unwrap()
            .contains(&json!("app.bsky.feed.post"))
    );
    let posts = ai
        .tool(
            &access,
            "list_records",
            json!({"collection": "app.bsky.feed.post", "limit": 1}),
        )
        .await;
    assert_eq!(
        posts["structuredContent"]["records"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "{posts}"
    );

    // A write in the collection of the scope, and none in another.
    let made = ai
        .tool(&access, "create_record", json!({"collection": NOTE, "rkey": "first", "record": {"$type": NOTE, "text": "from an AI client"}}))
        .await;
    assert_eq!(made["isError"], false, "{made}");
    let uri = format!("at://{ALICE}/{NOTE}/first");
    assert_eq!(made["structuredContent"]["uri"], uri.as_str());
    let other = ai
        .rpc(Some(&access), "tools/call", json!({"name": "create_record", "arguments": {"collection": "com.example.other", "record": {"$type": "com.example.other"}}}))
        .await;
    assert_eq!(other.status, 403, "{}", other.text);
    let header = other.header("www-authenticate").unwrap();
    assert!(
        header.contains(r#"error="insufficient_scope""#)
            && header.contains(r#"scope="repo:write:com.example.other""#),
        "{header}"
    );
    let read = ai
        .rpc(Some(&access), "resources/read", json!({"uri": uri}))
        .await
        .json();
    assert!(
        read["result"]["contents"][0]["text"]
            .as_str()
            .unwrap()
            .contains("from an AI client"),
        "{read}"
    );
    let foreign = ai
        .rpc(
            Some(&access),
            "resources/read",
            json!({"uri": format!("at://{}/{NOTE}/first", common::DANA)}),
        )
        .await
        .json();
    assert_eq!(foreign["error"]["code"], -32002);
    // A refusal of the PDS is a result that the model reads.
    let missing = ai
        .tool(
            &access,
            "get_record",
            json!({"collection": NOTE, "rkey": "nope"}),
        )
        .await;
    assert_eq!(missing["isError"], true, "{missing}");

    // The PDS wrote an entry for the write: the sidecar and the AI client.
    let entries = pds_xrpc::audit::AuditLog::open(&server.dir.config.layout.audit_db, None)
        .unwrap()
        .page(None, 1)
        .unwrap();
    assert_eq!(
        entries[0]["action"], "com.atproto.repo.createRecord",
        "{entries:?}"
    );
    assert_eq!(entries[0]["actor"], ALICE);
    assert_eq!(entries[0]["clientId"], sidecar_id.as_str());
    assert_eq!(entries[0]["sidecar"], format!("mcp:{client_id}").as_str());

    // A refresh rotates both tokens. The old refresh token ends the session.
    let rotate = |refresh: &str| {
        vec![
            ("grant_type", "refresh_token".to_owned()),
            ("refresh_token", refresh.to_owned()),
            ("client_id", client_id.clone()),
        ]
    };
    let form = rotate(&refresh);
    let form: Vec<(&str, &str)> = form.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let next = ai.form("/token", &form).await;
    assert_eq!(next.status, 200, "{}", next.text);
    let next_access = next.json()["access_token"].as_str().unwrap().to_owned();
    assert_eq!(ai.rpc(Some(&access), "ping", json!({})).await.status, 401);
    assert_eq!(
        ai.rpc(Some(&next_access), "ping", json!({})).await.status,
        200
    );
    assert_eq!(ai.form("/token", &form).await.status, 400);
    assert_eq!(
        ai.rpc(Some(&next_access), "ping", json!({})).await.status,
        401
    );

    // A revocation ends a session on both sides.
    let back = mcp_authorize(
        &mut ai,
        &mut server,
        &sidecar_id,
        &client_id,
        "repo:read",
        "allow",
    )
    .await;
    let form = exchange(
        VERIFIER,
        parameter(&back, "code").unwrap(),
        client_id.clone(),
    );
    let form: Vec<(&str, &str)> = form.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let tokens = ai.form("/token", &form).await.json();
    let access = tokens["access_token"].as_str().unwrap().to_owned();
    let sessions = |server: &Server| -> i64 {
        common::open(&server.dir.config.layout.account_db)
            .query_row(
                r#"select count(*) from "token" where "did" = ?1"#,
                [ALICE],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!(sessions(&server), 1);
    // Without the scope of the account the tool is refused.
    let who = ai
        .rpc(Some(&access), "tools/call", json!({"name": "whoami"}))
        .await;
    assert_eq!(who.status, 403, "{}", who.text);
    assert_eq!(ai.form("/revoke", &[("token", &access)]).await.status, 200);
    assert_eq!(ai.rpc(Some(&access), "ping", json!({})).await.status, 401);
    assert_eq!(sessions(&server), 0);
    let _ = std::fs::remove_file(&known);
}

/// One line of JSON from the official client, read without blocking the
/// servers that the client talks to.
async fn client_line(
    out: std::sync::Arc<std::sync::Mutex<std::io::BufReader<std::process::ChildStdout>>>,
) -> Value {
    tokio::task::spawn_blocking(move || {
        use std::io::BufRead;
        let mut line = String::new();
        out.lock().unwrap().read_line(&mut line).unwrap();
        let value = serde_json::from_str(&line);
        assert!(value.is_ok(), "not a line of the client: {line:?}");
        value.unwrap()
    })
    .await
    .unwrap()
}

/// The gate of unit 10: the official MCP TypeScript client signs in
/// through the pages of the PDS, uses the tools, refreshes and revokes.
/// It runs when `PDS_TEST_MCP_CLIENT` names a Node binary and the
/// dependencies of `parity/` are installed. With `MCP_INSPECTOR` set to
/// the binary of the MCP Inspector, the Inspector lists the tools too.
#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::too_many_lines)] // one client, from the challenge to the revocation
async fn the_official_mcp_client_signs_in_through_the_pds() {
    let Ok(node) = std::env::var("PDS_TEST_MCP_CLIENT") else {
        return;
    };
    let listener = common::http::bind().await;
    let mcp_origin = format!("http://{}", listener.local_addr().unwrap());
    let vars = |pds: &str, internal: &str| -> std::collections::BTreeMap<String, String> {
        [
            ("MCP_PUBLIC_URL", MCP_PUBLIC),
            ("MCP_PDS_PUBLIC_URL", pds),
            ("MCP_PDS_INTERNAL_URL", internal),
            ("MCP_WRITE_COLLECTIONS", NOTE),
            (
                "MCP_CLIENT_KEY_P256_PRIVATE_KEY_HEX",
                "0707070707070707070707070707070707070707070707070707070707070707",
            ),
        ]
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
    };
    let draft = pds_mcp::config::Config::from_vars(&vars("https://pds.test", "http://127.0.0.1:1"))
        .unwrap();
    let sidecar_id = draft.client_id();
    let known = std::env::temp_dir().join(format!(
        "linkjar-pds-official-client-{}.json",
        std::process::id()
    ));
    let mut clients = serde_json::Map::new();
    clients.insert(sidecar_id.clone(), draft.client_metadata());
    std::fs::write(&known, Value::Object(clients).to_string()).unwrap();
    let mut server = Server::start_with(&[
        ("PDS_OAUTH_KNOWN_CLIENTS_FILE", known.to_str().unwrap()),
        ("PDS_MCP_CLIENT_IDS", &sidecar_id),
    ])
    .await;
    let config = pds_mcp::config::Config::from_vars(&vars(&server.issuer, &server.origin)).unwrap();
    let mcp = pds_mcp::Mcp::new(config).unwrap();
    tokio::spawn(async move { axum::serve(listener, pds_mcp::router(mcp)).await });
    let mut ai = AiClient {
        origin: mcp_origin.clone(),
        http: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap(),
        cookie: None,
        serial: 0,
    };

    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../parity/mcp/official-client.ts"
    );
    let mut child = std::process::Command::new(node)
        .arg(script)
        .arg(format!("{MCP_PUBLIC}/mcp"))
        .arg(&mcp_origin)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let out = std::sync::Arc::new(std::sync::Mutex::new(std::io::BufReader::new(
        child.stdout.take().unwrap(),
    )));
    let mut input = child.stdin.take().unwrap();

    // The SDK found the server, registered, and sends the person to sign in.
    let step = client_line(out.clone()).await;
    assert!(step["authorize"].is_string(), "{step}");
    let url = step["authorize"].as_str().unwrap();
    let url = url.replace(MCP_PUBLIC, &mcp_origin);
    let back = mcp_browser(
        &mut ai,
        &mut server,
        &sidecar_id,
        &url,
        "Official SDK",
        MCP_PUBLIC,
        "allow",
    )
    .await;
    assert!(
        back.as_str()
            .starts_with("http://127.0.0.1/official-callback?"),
        "{back}"
    );
    std::io::Write::write_all(&mut input, format!("{back}\n").as_bytes()).unwrap();

    let done = client_line(out).await;
    let results = &done["done"];
    assert_eq!(results["registered"], true, "{done}");
    assert_eq!(
        results["authorizeResource"],
        format!("{MCP_PUBLIC}/mcp"),
        "{done}"
    );
    assert_eq!(results["state"], true, "{done}");
    // The SDK asks for every scope of `scopes_supported` (SEP-835) when the
    // challenge names none, and the person allows them on the consent page.
    let every = "account:read repo:read repo:write repo:write:com.example.note";
    assert_eq!(results["authorizeScope"], every, "{done}");
    assert_eq!(results["scope"], every, "{done}");
    assert_eq!(results["tools"].as_array().unwrap().len(), 8, "{done}");
    assert_eq!(results["did"], ALICE, "{done}");
    let uri = format!("at://{ALICE}/{NOTE}/official");
    assert_eq!(results["created"], uri.as_str(), "{done}");
    assert_eq!(results["read"], true, "{done}");
    assert_eq!(results["prompts"], 2, "{done}");
    if std::env::var_os("MCP_INSPECTOR").is_some() {
        assert_eq!(results["inspectorTools"], 8, "{done}");
    }
    assert_eq!(results["refresh"], "AUTHORIZED", "{done}");
    assert_eq!(results["rotated"], true, "{done}");
    assert_eq!(results["oldAccess"], 401, "{done}");
    assert_eq!(results["newAccess"], 200, "{done}");
    assert_eq!(results["revoked"], 200, "{done}");
    assert_eq!(results["afterRevocation"], 401, "{done}");
    assert!(child.wait().unwrap().success());
    let _ = std::fs::remove_file(&known);
}
