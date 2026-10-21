use super::*;

#[tokio::test]
async fn the_pages_wear_the_brand_of_the_configuration() {
    let mut server = Server::start_with(&[
        ("PDS_PROFILE", "linkjar"),
        ("PDS_SERVICE_NAME", "LinkJar"),
        ("PDS_HOME_URL", "https://linkjar.io"),
        ("PDS_TERMS_OF_SERVICE_URL", "https://linkjar.io/terms"),
        ("PDS_SUPPORT_URL", "https://linkjar.io/support"),
    ])
    .await;
    let page = server
        .send(reqwest::Method::GET, "/account", None, &[])
        .await;
    assert_eq!(page.status, 200);
    assert!(
        page.text.contains("<title>Your account · LinkJar</title>"),
        "{}",
        page.text
    );
    assert!(page.text.contains(
        "<header><a class=\"lj-brand\" href=\"https://linkjar.io\"><img src=\"/oauth/assets/linkjar-mark.png\" alt=\"\">LinkJar</a></header>"
    ));
    assert!(page.text.contains(
        "<nav><a href=\"https://linkjar.io\">Home</a><a href=\"https://linkjar.io/terms\">Terms of Service</a><a href=\"https://linkjar.io/support\">Support</a></nav>"
    ));
    let mark = server
        .send(
            reqwest::Method::GET,
            "/oauth/assets/linkjar-mark.png",
            None,
            &[],
        )
        .await;
    assert_eq!(
        (mark.status, mark.header("content-type")),
        (200, Some("image/png"))
    );
}
