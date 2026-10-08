//! The generated API types round-trip real wire data: a Bluesky post with an
//! embed union, a `createRecord` output and a `describeServer` output.

use pds_types::generated::app::bsky::feed::post::Post;
use pds_types::generated::app::bsky::feed::post::PostEmbed;
use pds_types::generated::com::atproto::repo::create_record;
use pds_types::generated::com::atproto::server::describe_server;

#[test]
fn post_with_external_embed_round_trips() {
    let json = r#"{
      "$type": "app.bsky.feed.post",
      "text": "hello",
      "createdAt": "2026-10-08T12:00:00.000Z",
      "langs": ["en", "de-CH"],
      "embed": {
        "$type": "app.bsky.embed.external",
        "external": { "uri": "https://example.com", "title": "Example", "description": "" }
      }
    }"#;
    let post: Post = serde_json::from_str(json).expect("post parses");
    assert_eq!(post.text, "hello");
    assert_eq!(post.langs.as_ref().map(Vec::len), Some(2));
    match post.embed.as_ref().expect("embed") {
        PostEmbed::External(e) => assert_eq!(e.external.uri.as_str(), "https://example.com"),
        other => panic!("unexpected embed {other:?}"),
    }
    let back = serde_json::to_value(&post).expect("serializes");
    assert_eq!(back["embed"]["$type"], "app.bsky.embed.external");
    assert_eq!(back["createdAt"], "2026-10-08T12:00:00.000Z");
    assert!(
        back.get("reply").is_none(),
        "absent optional fields are omitted"
    );
}

#[test]
fn unknown_embed_lands_in_the_open_union_variant() {
    let json = r#"{"text":"x","createdAt":"2026-10-08T12:00:00Z","embed":{"$type":"com.example.embed","a":1}}"#;
    let post: Post = serde_json::from_str(json).expect("post parses");
    assert!(matches!(post.embed, Some(PostEmbed::Other(_))));
    let back = serde_json::to_value(&post).expect("serializes");
    assert_eq!(back["embed"]["$type"], "com.example.embed");
}

#[test]
fn create_record_output_and_describe_server() {
    let out: create_record::Output = serde_json::from_str(
        r#"{"uri":"at://did:plc:abc/app.bsky.feed.post/3jzfcijpj2z2a","cid":"bafyreiclp443lavogvhj3d2ob2cxbfuscni2k5jk7bebjzg7khl3esabwq","validationStatus":"valid"}"#,
    )
    .expect("output parses");
    assert_eq!(out.uri.collection().unwrap().as_str(), "app.bsky.feed.post");
    assert_eq!(create_record::NSID, "com.atproto.repo.createRecord");
    assert!(create_record::ERRORS.contains(&"InvalidSwap"));

    let ds: describe_server::Output = serde_json::from_str(
        r#"{"did":"did:web:pds.example","availableUserDomains":[".example"],"inviteCodeRequired":false}"#,
    )
    .expect("describeServer parses");
    assert_eq!(ds.did.method(), "web");
}

#[test]
fn invalid_syntax_in_a_typed_field_is_rejected() {
    let bad = r#"{"text":"x","createdAt":"not a datetime"}"#;
    assert!(serde_json::from_str::<Post>(bad).is_err());
}
