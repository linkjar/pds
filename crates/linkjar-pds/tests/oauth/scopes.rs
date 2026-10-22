use super::*;

const APPVIEW_DID: &str = "did:web:appview.test";
const GET_PREFERENCES: &str = "/xrpc/app.bsky.actor.getPreferences";
const PUT_PREFERENCES: &str = "/xrpc/app.bsky.actor.putPreferences";

/// A server that keeps preferences itself, and a token of a client that
/// asked for `scope`.
async fn signed_in_with(scope: &str) -> (Server, String) {
    let mut server = Server::start_with(&[
        // Nothing listens there: the server answers preferences itself.
        ("PDS_BSKY_APP_VIEW_URL", "http://127.0.0.1:9"),
        ("PDS_BSKY_APP_VIEW_DID", APPVIEW_DID),
    ])
    .await;
    server.scope = scope.to_owned();
    // The first request learns the DPoP nonce.
    server.push(&[]).await;
    let (_, tokens) = server.sign_in(true).await;
    let token = tokens["access_token"].as_str().unwrap().to_owned();
    (server, token)
}

// A grant to read the preferences does not let a client replace them.
#[tokio::test(flavor = "multi_thread")]
async fn a_grant_to_read_preferences_does_not_write_them() {
    let read =
        format!("atproto rpc:app.bsky.actor.getPreferences?aud={APPVIEW_DID}%23bsky_appview");
    let (mut server, token) = signed_in_with(&read).await;
    let key = client_key();
    let got = server
        .resource(&key, reqwest::Method::GET, GET_PREFERENCES, &token, None)
        .await;
    assert_eq!(got.status, 200, "{}", got.text);
    let body = json!({"preferences": [{"$type": "app.bsky.actor.defs#adultContentPref", "enabled": true}]});
    let put = server
        .resource(
            &key,
            reqwest::Method::POST,
            PUT_PREFERENCES,
            &token,
            Some(&body),
        )
        .await;
    assert_eq!(put.status, 403, "{}", put.text);
    assert_eq!(put.json()["error"], "ScopeMissingError", "{}", put.text);

    let write =
        format!("{read} rpc:app.bsky.actor.putPreferences?aud={APPVIEW_DID}%23bsky_appview");
    let (mut server, token) = signed_in_with(&write).await;
    let put = server
        .resource(
            &key,
            reqwest::Method::POST,
            PUT_PREFERENCES,
            &token,
            Some(&body),
        )
        .await;
    assert_eq!(put.status, 200, "{}", put.text);
}
