use std::path::PathBuf;

use serde_json::{Value, json};

use super::*;
use crate::model::LexiconDoc;

/// Vectors that do not pass, as `<file>: <case>`, with the finding that
/// explains each. A listed case must still fail: remove the entry when
/// it starts to pass.
const KNOWN_FAILURES: &[(&str, &str)] = &[];

/// Records one vector outcome, checked against `KNOWN_FAILURES`.
fn check(case: &str, passes: bool, detail: &str, problems: &mut Vec<String>) {
    let known = KNOWN_FAILURES.iter().find(|(name, _)| *name == case);
    match (known, passes) {
        (None, true) | (Some(_), false) => {}
        (None, false) => problems.push(format!("{case}: {detail}")),
        (Some((_, finding)), true) => problems.push(format!(
            "{case}: passes, remove it from KNOWN_FAILURES ({finding})"
        )),
    }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn cases(path: &str) -> Vec<Value> {
    let text = std::fs::read_to_string(repo_root().join(path)).expect("vector file");
    serde_json::from_str(&text).expect("vector JSON")
}

fn parse(value: &Value) -> Result<Data, String> {
    let text = serde_json::to_string(value).expect("JSON text");
    Data::from_json_str(&text).map_err(|e| e.to_string())
}

fn data(value: &Value) -> Data {
    parse(value).expect("a data model value")
}

fn catalog() -> Corpus {
    Corpus::load_dir(&repo_root().join("interop/lexicon/catalog")).expect("catalog")
}

fn lexicons() -> Corpus {
    Corpus::load_dir(&repo_root().join("lexicons")).expect("lexicons")
}

fn code(rule: &Rule) -> &'static str {
    match rule {
        Rule::Type { .. } => "type",
        Rule::Value { .. } => "value",
        Rule::RequiredKey { .. } => "required_key",
        Rule::Format { .. } => "format",
        Rule::TooSmall { .. } => "too_small",
        Rule::TooBig { .. } => "too_big",
        Rule::Blob(_) => "blob",
        Rule::Empty => "empty",
        Rule::TooDeep => "too_deep",
        Rule::UnknownLexicon { .. } => "unknown_lexicon",
        Rule::Schema(_) => "schema",
    }
}

/// The path and rule of a rejection, or `accepted`.
fn outcome(result: &Result<(), ValidationError>) -> String {
    match result {
        Ok(()) => "accepted".to_owned(),
        Err(e) => format!("{}: {}", e.path, code(&e.rule)),
    }
}

#[test]
fn record_data_valid_vectors_validate() {
    let corpus = catalog();
    let mut problems = Vec::new();
    for case in cases("interop/lexicon/record-data-valid.json") {
        let name = case["name"].as_str().expect("name");
        let rkey = case["rkey"].as_str().expect("rkey");
        let result = parse(&case["data"])
            .map_err(|e| format!("JSON parsing: {e}"))
            .and_then(|data| {
                corpus
                    .validate_record(&data)
                    .map_err(|e| format!("validate_record: {e}"))?;
                let nsid = data.type_name().expect("$type");
                let key = &corpus.record(nsid).expect("record lexicon").key;
                validate_record_key(key, rkey).map_err(|e| format!("validate_record_key: {e}"))
            });
        check(
            &format!("record-data-valid: {name}"),
            result.is_ok(),
            &result.err().unwrap_or_default(),
            &mut problems,
        );
    }
    assert!(problems.is_empty(), "{problems:#?}");
}

/// What rejects each `record-data-invalid` case, in file order: the path
/// and the rule from `validate_record`, or `json` for JSON parsing into
/// `Data`.
///
/// The three `unknown wrong type` cases omit the required `integer`, so
/// `validate_record` fails on that key and does not reach `unknown`;
/// `unknown_is_an_object` covers the type.
const INVALID_RECORD_DATA: &[(&str, &str)] = &[
    ("missing required field", "record: required_key"),
    ("invalid boolean field", "record/boolean: type"),
    ("invalid integer field", "record/integer: type"),
    ("invalid non-nullable string field", "record/string: type"),
    ("invalid string field", "record/string: type"),
    ("invalid bytes field", "record/bytes: type"),
    ("invalid bytes: empty object", "record/bytes: type"),
    ("invalid bytes: wrong type", "record/bytes: type"),
    ("invalid cid-link field", "record/cid-link: type"),
    ("invalid blob field", "record/blob: type"),
    ("invalid blob: wrong type", "record/blob: blob"),
    ("invalid array", "record/array: type"),
    ("invalid array element", "record/array/0: type"),
    ("object wrong data type", "record/object: type"),
    ("object nested wrong data type", "record/object/a: type"),
    ("invalid token ref type", "record/ref: type"),
    ("invalid ref value", "record/ref: type"),
    (
        "invalid string format handle",
        "record/formats/handle: format",
    ),
    ("invalid string format did", "record/formats/did: format"),
    (
        "invalid string format atidentifier",
        "record/formats/atidentifier: format",
    ),
    ("invalid string format nsid", "record/formats/nsid: format"),
    (
        "invalid string format aturi",
        "record/formats/aturi: format",
    ),
    ("invalid string format cid", "record/formats/cid: format"),
    (
        "invalid string format datetime",
        "record/formats/datetime: format",
    ),
    (
        "invalid string format language",
        "record/formats/language: format",
    ),
    ("invalid string format uri", "record/formats/uri: format"),
    ("invalid string format tid", "record/formats/tid: format"),
    (
        "invalid string format recordkey",
        "record/formats/recordkey: format",
    ),
    ("wrong const value", "record/constInteger: value"),
    ("integer not in enum", "record/enumInteger: value"),
    ("out of integer range", "record/rangeInteger: too_big"),
    ("string too short", "record/lenString: too_small"),
    ("string too long", "record/lenString: too_big"),
    (
        "string too short (graphemes)",
        "record/graphemeString: too_small",
    ),
    (
        "string too long (graphemes)",
        "record/graphemeString: too_big",
    ),
    ("out of enum string", "record/enumString: value"),
    ("bytes too short", "record/sizeBytes: too_small"),
    ("bytes too long", "record/sizeBytes: too_big"),
    ("array too short", "record/lenArray: too_small"),
    ("array too long", "record/lenArray: too_big"),
    ("blob too large", "record/sizeBlob: too_big"),
    ("blob wrong type", "record/acceptBlob/mimeType: value"),
    ("open union wrong data type", "record/union: type"),
    ("open union missing $type", "record/union: type"),
    ("out of closed union", "record/closedUnion/$type: value"),
    ("union inner invalid", "record/closedUnion/$type: value"),
    ("union inner invalid", "record/union/a: type"),
    ("unknown wrong type (bool)", "record: required_key"),
    ("unknown wrong type (bytes)", "record: required_key"),
    ("unknown wrong type (blob)", "record: required_key"),
];

#[test]
fn record_data_invalid_vectors_are_rejected() {
    let corpus = catalog();
    let cases = cases("interop/lexicon/record-data-invalid.json");
    assert_eq!(cases.len(), INVALID_RECORD_DATA.len());
    let mut problems = Vec::new();
    let mut also_data_model = Vec::new();
    for (case, (expected_name, expected)) in cases.iter().zip(INVALID_RECORD_DATA) {
        let name = case["name"].as_str().expect("name");
        assert_eq!(name, *expected_name, "the table follows the file order");
        let found = match parse(&case["data"]) {
            Ok(data) => {
                if validate_data_model(&data).is_err() {
                    also_data_model.push(name);
                }
                outcome(&corpus.validate_record(&data))
            }
            Err(_) => "json".to_owned(),
        };
        check(
            &format!("record-data-invalid: {name}"),
            found == *expected,
            &format!("validate_record gave {found:?}, expected {expected:?}"),
            &mut problems,
        );
    }
    assert!(problems.is_empty(), "{problems:#?}");
    // These blob references carry `dag-cbor` CIDs, so the data-model
    // check rejects them as well.
    assert_eq!(
        also_data_model,
        [
            "blob too large",
            "blob wrong type",
            "unknown wrong type (blob)"
        ]
    );
}

#[test]
fn the_full_vector_fails_the_data_model_check_for_its_blob_cids() {
    let corpus = catalog();
    let cases = cases("interop/lexicon/record-data-valid.json");
    let full = &cases
        .iter()
        .find(|case| case["name"] == "full")
        .expect("full")["data"];
    // `123` is not canonical base64: its last character has unused bits
    // set. The Reference reads two bytes, and so does `pds-types`.
    assert_eq!(full["bytes"], json!({ "$bytes": "123" }));
    let record = data(full);
    assert_eq!(outcome(&corpus.validate_record(&record)), "accepted");
    // The blob references of the vector carry `dag-cbor` CIDs, which
    // SPEC §6.3 and the Reference's strict blob check reject.
    assert_eq!(
        outcome(&validate_data_model(&record)),
        "record/acceptBlob/ref: blob"
    );
    let minimal = data(&cases[0]["data"]);
    assert_eq!(outcome(&validate_data_model(&minimal)), "accepted");
}

#[test]
fn data_model_valid_vectors_pass() {
    let mut problems = Vec::new();
    for case in cases("interop/data-model/data-model-valid.json") {
        let note = case["note"].as_str().expect("note");
        let result = parse(&case["json"])
            .map_err(|e| format!("JSON parsing: {e}"))
            .and_then(|data| {
                validate_data_model(&data).map_err(|e| format!("validate_data_model: {e}"))
            });
        check(
            &format!("data-model-valid: {note}"),
            result.is_ok(),
            &result.err().unwrap_or_default(),
            &mut problems,
        );
    }
    assert!(problems.is_empty(), "{problems:#?}");
}

/// What rejects each `data-model-invalid` case, in file order: `json`
/// for JSON parsing into `Data`, else the path and rule from
/// `validate_data_model`.
const INVALID_DATA_MODEL: &[(&str, &str)] = &[
    ("top-level not an object", "record: type"),
    ("float", "json"),
    ("record with $type null", "record/rcrd/$type: type"),
    ("record with $type wrong type", "record/rcrd/$type: type"),
    ("record with empty $type string", "record/rcrd/$type: empty"),
    ("blob with string size", "record/blb/size: type"),
    ("blob with missing key", "record/blb: required_key"),
    ("bytes with wrong field type", "json"),
    ("bytes with extra fields", "json"),
    ("link with wrong field type", "json"),
    ("link with bogus CID", "json"),
    ("link with extra fields", "json"),
];

#[test]
fn data_model_invalid_vectors_are_rejected() {
    let cases = cases("interop/data-model/data-model-invalid.json");
    assert_eq!(cases.len(), INVALID_DATA_MODEL.len());
    let mut problems = Vec::new();
    for (case, (expected_note, expected)) in cases.iter().zip(INVALID_DATA_MODEL) {
        let note = case["note"].as_str().expect("note");
        assert_eq!(note, *expected_note, "the table follows the file order");
        let found = parse(&case["json"]).map_or_else(
            |_| "json".to_owned(),
            |data| outcome(&validate_data_model(&data)),
        );
        check(
            &format!("data-model-invalid: {note}"),
            found == *expected,
            &format!("got {found:?}, expected {expected:?}"),
            &mut problems,
        );
    }
    assert!(problems.is_empty(), "{problems:#?}");
}

const RAW_CID: &str = "bafkreiccldh766hwcnuxnf2wh6jgzepf2nlu2lvcllt63eww5p6chi4ity";
const CBOR_CID: &str = "bafyreiclp443lavogvhj3d2ob2cxbfuscni2k5jk7bebjzg7khl3esabwq";

fn blob(mime_type: &str, size: i64) -> Value {
    json!({ "$type": "blob", "ref": { "$link": RAW_CID }, "mimeType": mime_type, "size": size })
}

fn post() -> Value {
    let parent = json!({
        "uri": "at://did:plc:ewvi7nxzyoun6zhxrhs64oiz/app.bsky.feed.post/3kznmn7xqxl22",
        "cid": CBOR_CID
    });
    json!({
        "$type": "app.bsky.feed.post",
        "text": "Read the specification at atproto.com @alice.example #atproto",
        "createdAt": "2026-10-12T09:30:00.000Z",
        "langs": ["en", "pt-BR"],
        "facets": [
            {
                "index": { "byteStart": 26, "byteEnd": 37 },
                "features": [
                    { "$type": "app.bsky.richtext.facet#link", "uri": "https://atproto.com" }
                ]
            },
            {
                "index": { "byteStart": 38, "byteEnd": 52 },
                "features": [{
                    "$type": "app.bsky.richtext.facet#mention",
                    "did": "did:plc:ewvi7nxzyoun6zhxrhs64oiz"
                }]
            },
            {
                "index": { "byteStart": 53, "byteEnd": 61 },
                "features": [{ "$type": "app.bsky.richtext.facet#tag", "tag": "atproto" }]
            }
        ],
        "reply": { "root": parent, "parent": parent },
        "embed": {
            "$type": "app.bsky.embed.images",
            "images": [{
                "alt": "A diagram",
                "image": blob("image/png", 51_234),
                "aspectRatio": { "width": 16, "height": 9 }
            }]
        },
        "labels": {
            "$type": "com.atproto.label.defs#selfLabels",
            "values": [{ "val": "graphic-media" }]
        },
        "tags": ["atproto"]
    })
}

/// The valid post with the value at a JSON pointer replaced.
fn post_with(pointer: &str, value: Value) -> Value {
    let mut record = post();
    *record.pointer_mut(pointer).expect("pointer") = value;
    record
}

/// The valid post without one key of the object at a JSON pointer.
fn post_without(pointer: &str, key: &str) -> Value {
    let mut record = post();
    let object = record.pointer_mut(pointer).expect("pointer");
    object.as_object_mut().expect("object").remove(key);
    record
}

#[test]
fn a_post_validates_against_the_vendored_corpus() {
    let corpus = lexicons();
    let record = data(&post());
    assert_eq!(outcome(&validate_data_model(&record)), "accepted");
    assert_eq!(outcome(&corpus.validate_record(&record)), "accepted");
    let key = &corpus.record("app.bsky.feed.post").expect("post").key;
    assert_eq!(
        outcome(&validate_record_key(key, "3kznmn7xqxl22")),
        "accepted"
    );
    assert_eq!(outcome(&validate_record_key(key, "self")), "rkey: format");
}

#[test]
fn invalid_posts_are_rejected() {
    let long = "a".repeat(129);
    let image = "/embed/images/0/image";
    let cases = [
        (post_without("", "createdAt"), "record: required_key"),
        (
            post_with("/createdAt", json!("2026-10-12 09:30:00")),
            "record/createdAt: format",
        ),
        (post_with("/text", json!(5)), "record/text: type"),
        (
            post_with("/langs", json!(["en", "pt", "de", "fr"])),
            "record/langs: too_big",
        ),
        (
            post_with("/langs/1", json!("english!")),
            "record/langs/1: format",
        ),
        (
            post_with("/facets/0/features/0/uri", json!("no scheme")),
            "record/facets/0/features/0/uri: format",
        ),
        (
            post_with("/facets/1/features/0/did", json!("alice.example")),
            "record/facets/1/features/0/did: format",
        ),
        (
            post_with("/facets/2/index/byteStart", json!(-1)),
            "record/facets/2/index/byteStart: too_small",
        ),
        (
            post_without("/reply", "parent"),
            "record/reply: required_key",
        ),
        (
            post_with("/reply/root/cid", json!("nope")),
            "record/reply/root/cid: format",
        ),
        (post_without("/embed", "$type"), "record/embed: type"),
        (
            post_with(image, blob("application/pdf", 51_234)),
            "record/embed/images/0/image/mimeType: value",
        ),
        (
            post_with(image, blob("image/png", 2_000_001)),
            "record/embed/images/0/image: too_big",
        ),
        (
            post_with(image, json!({ "cid": RAW_CID, "mimeType": "image/png" })),
            "record/embed/images/0/image: blob",
        ),
        (
            post_with("/embed/images/0/aspectRatio/width", json!(0)),
            "record/embed/images/0/aspectRatio/width: too_small",
        ),
        (
            post_with("/labels/values/0/val", json!(long)),
            "record/labels/values/0/val: too_big",
        ),
    ];
    let corpus = lexicons();
    for (record, expected) in cases {
        assert_eq!(outcome(&corpus.validate_record(&data(&record))), expected);
    }
}

#[test]
fn post_text_is_limited_in_bytes_and_in_graphemes() {
    let corpus = lexicons();
    let rule = |text: String| {
        corpus
            .validate_record(&data(&post_with("/text", json!(text))))
            .map_err(|e| e.rule)
    };
    // 300 graphemes of two bytes each.
    assert_eq!(rule("é".repeat(300)), Ok(()));
    assert_eq!(
        rule("é".repeat(301)),
        Err(Rule::TooBig {
            measure: Measure::Grapheme,
            maximum: 300,
            actual: 301
        })
    );
    // One grapheme of 25 bytes: four people joined by zero-width joiners.
    let family = "👩‍👩‍👦‍👦";
    assert_eq!(rule(family.repeat(120)), Ok(()));
    assert_eq!(
        rule(family.repeat(121)),
        Err(Rule::TooBig {
            measure: Measure::String,
            maximum: 3000,
            actual: 3025
        })
    );
}

#[test]
fn an_open_union_accepts_a_type_outside_its_refs() {
    let corpus = lexicons();
    let record = post_with("/embed", json!({ "$type": "com.example.embed", "x": 1 }));
    assert_eq!(outcome(&corpus.validate_record(&data(&record))), "accepted");
}

#[test]
fn a_lexicon_schema_record_validates_against_the_vendored_corpus() {
    let corpus = lexicons();
    let record = json!({
        "$type": "com.atproto.lexicon.schema",
        "lexicon": 1,
        "id": "com.example.thing",
        "defs": { "main": { "type": "token" } }
    });
    assert_eq!(outcome(&corpus.validate_record(&data(&record))), "accepted");
    let key = &corpus
        .record("com.atproto.lexicon.schema")
        .expect("schema")
        .key;
    assert_eq!(
        outcome(&validate_record_key(key, "com.example.thing")),
        "accepted"
    );
    assert_eq!(outcome(&validate_record_key(key, "self")), "rkey: format");

    let mut without = record.clone();
    without.as_object_mut().expect("object").remove("lexicon");
    assert_eq!(
        outcome(&corpus.validate_record(&data(&without))),
        "record: required_key"
    );
    let mut text = record;
    text["lexicon"] = json!("1");
    assert_eq!(
        outcome(&corpus.validate_record(&data(&text))),
        "record/lexicon: type"
    );
}

#[test]
fn a_type_without_a_record_lexicon_is_reported_as_unknown() {
    let corpus = lexicons();
    for nsid in [
        "com.example.unknown",
        // An object, not a record.
        "com.atproto.repo.strongRef",
        // The Reference looks the `$type` up as written.
        "app.bsky.feed.post#main",
    ] {
        let result = corpus.validate_record(&data(&json!({ "$type": nsid })));
        assert_eq!(outcome(&result), "record/$type: unknown_lexicon");
    }
    let untyped = corpus.validate_record(&data(&json!({ "text": "x" })));
    assert_eq!(outcome(&untyped), "record: required_key");
    let number = corpus.validate_record(&data(&json!({ "$type": 5 })));
    assert_eq!(outcome(&number), "record/$type: type");
    assert_eq!(
        outcome(&corpus.validate_record(&Data::Null)),
        "record: type"
    );
}

#[test]
fn record_key_types_are_enforced() {
    let cases = [
        ("tid", "3kznmn7xqxl22", "accepted"),
        ("tid", "self", "rkey: format"),
        ("nsid", "com.example.thing", "accepted"),
        ("nsid", "self", "rkey: format"),
        ("any", "self", "accepted"),
        ("any", "a/b", "rkey: format"),
        ("any", "..", "rkey: format"),
        ("literal:self", "self", "accepted"),
        ("literal:self", "other", "rkey: value"),
        ("literal:self", "", "rkey: format"),
        ("uuid", "self", "rkey: schema"),
    ];
    for (key_type, rkey, expected) in cases {
        assert_eq!(
            outcome(&validate_record_key(key_type, rkey)),
            expected,
            "{key_type} {rkey:?}"
        );
    }
}

#[test]
fn every_vendored_string_format_and_record_key_type_is_known() {
    fn formats(value: &Value, out: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                if map.get("type") == Some(&json!("string"))
                    && let Some(Value::String(format)) = map.get("format")
                {
                    out.push(format.clone());
                }
                map.values().for_each(|v| formats(v, out));
            }
            Value::Array(items) => items.iter().for_each(|v| formats(v, out)),
            _ => {}
        }
    }
    let corpus = lexicons();
    let mut found = Vec::new();
    for doc in corpus.docs() {
        formats(&serde_json::to_value(doc).expect("doc"), &mut found);
        if let Some(record) = corpus.record(&doc.id) {
            let result = validate_record_key(&record.key, "self");
            assert_ne!(outcome(&result), "rkey: schema", "{}", doc.id);
        }
    }
    found.sort();
    found.dedup();
    assert!(found.len() > 5, "formats found: {found:?}");
    for format in found {
        let result = string_format(&format, "");
        assert!(
            !matches!(
                result,
                Err(ValidationError {
                    rule: Rule::Schema(_),
                    ..
                })
            ),
            "unknown string format {format}"
        );
    }
}

/// Two documents that exercise the rules the vectors do not reach.
fn rules() -> Corpus {
    let docs = [
        json!({
            "lexicon": 1,
            "id": "com.example.rules",
            "defs": {
                "main": {
                    "type": "record",
                    "key": "any",
                    "record": {
                        "type": "object",
                        "nullable": ["maybe"],
                        "properties": {
                            "constString": { "type": "string", "const": "abc", "maxLength": 1 },
                            "enumString": { "type": "string", "enum": ["long"], "maxLength": 1 },
                            "impossible": { "type": "string", "const": "x", "enum": ["y"] },
                            "flag": { "type": "boolean", "const": false },
                            "maybe": { "type": "string" },
                            "number": { "type": "integer" },
                            "defaulted": { "type": "ref", "ref": "#defaulted" },
                            "token": { "type": "ref", "ref": "#tok" },
                            "item": { "type": "ref", "ref": "#item" },
                            "open": { "type": "union", "refs": ["#item", "com.example.other"] },
                            "closed": {
                                "type": "union",
                                "closed": true,
                                "refs": ["#item", "com.example.other"]
                            },
                            "image": {
                                "type": "blob",
                                "accept": ["image/*", "application/pdf"],
                                "maxSize": 100
                            },
                            "anyBlob": { "type": "blob", "accept": ["*/*"] },
                            "anything": { "type": "unknown" },
                            "cycle": { "type": "ref", "ref": "#a" },
                            "dangling": { "type": "ref", "ref": "#nowhere" },
                            "method": { "type": "ref", "ref": "com.example.query" },
                            "odd": { "type": "string", "format": "made-up" },
                            "tree": { "type": "ref", "ref": "#node" }
                        }
                    }
                },
                "defaulted": {
                    "type": "object",
                    "required": ["n"],
                    "properties": { "n": { "type": "integer", "default": 7 } }
                },
                "tok": { "type": "token" },
                "item": { "type": "object", "properties": { "a": { "type": "integer" } } },
                "a": { "type": "ref", "ref": "#b" },
                "b": { "type": "ref", "ref": "#a" },
                "node": {
                    "type": "object",
                    "properties": {
                        "children": { "type": "array", "items": { "type": "ref", "ref": "#node" } }
                    }
                }
            }
        }),
        json!({
            "lexicon": 1,
            "id": "com.example.other",
            "defs": {
                "main": {
                    "type": "object",
                    "required": ["v"],
                    "properties": { "v": { "type": "string" } }
                }
            }
        }),
        json!({
            "lexicon": 1,
            "id": "com.example.query",
            "defs": { "main": { "type": "query" } }
        }),
    ];
    let mut corpus = Corpus::new();
    for doc in docs {
        let doc: LexiconDoc = serde_json::from_value(doc).expect("lexicon");
        corpus.insert(doc).expect("unique id");
    }
    corpus
}

/// The outcome of validating a `com.example.rules` record with one
/// property.
fn rule_for(key: &str, value: Value) -> String {
    rule_in(BlobRules::Write, key, value)
}

fn rule_in(blobs: BlobRules, key: &str, value: Value) -> String {
    let mut record = json!({ "$type": "com.example.rules" });
    record[key] = value;
    outcome(&rules().validate_record_with(&data(&record), blobs))
}

#[test]
fn const_and_enum_replace_the_other_constraints() {
    assert_eq!(rule_for("constString", json!("abc")), "accepted");
    assert_eq!(
        rule_for("constString", json!("a")),
        "record/constString: value"
    );
    assert_eq!(
        rule_for("constString", json!(1)),
        "record/constString: value"
    );
    assert_eq!(rule_for("enumString", json!("long")), "accepted");
    assert_eq!(
        rule_for("enumString", json!("l")),
        "record/enumString: value"
    );
    assert_eq!(
        rule_for("impossible", json!("x")),
        "record/impossible: value"
    );
    assert_eq!(
        rule_for("impossible", json!("y")),
        "record/impossible: value"
    );
    assert_eq!(rule_for("flag", json!(false)), "accepted");
    assert_eq!(rule_for("flag", json!(true)), "record/flag: value");
}

#[test]
fn nullable_and_required_follow_the_object_schema() {
    assert_eq!(rule_for("maybe", json!(null)), "accepted");
    assert_eq!(rule_for("maybe", json!("x")), "accepted");
    assert_eq!(rule_for("number", json!(null)), "record/number: type");
    // A default does not stand in for a required property.
    assert_eq!(
        rule_for("defaulted", json!({})),
        "record/defaulted: required_key"
    );
    assert_eq!(rule_for("defaulted", json!({ "n": 1 })), "accepted");
    // Properties outside the schema are allowed.
    assert_eq!(rule_for("undeclared", json!([1, "two"])), "accepted");
}

#[test]
fn an_integer_stays_within_the_safe_range() {
    let record = |number: i64| {
        Data::Object(BTreeMap::from([
            (
                TYPE_KEY.to_owned(),
                Data::String("com.example.rules".to_owned()),
            ),
            ("number".to_owned(), Data::Integer(number)),
        ]))
    };
    let corpus = rules();
    assert_eq!(
        outcome(&corpus.validate_record(&record(MAX_SAFE_INTEGER))),
        "accepted"
    );
    assert_eq!(
        outcome(&corpus.validate_record(&record(-MAX_SAFE_INTEGER - 1))),
        "record/number: type"
    );
}

#[test]
fn a_reference_checks_the_type_it_leads_to() {
    assert_eq!(
        rule_for("token", json!("com.example.rules#tok")),
        "accepted"
    );
    assert_eq!(
        rule_for("token", json!("com.example.rules#other")),
        "record/token: value"
    );
    assert_eq!(rule_for("token", json!(5)), "record/token: type");
    assert_eq!(rule_for("item", json!({ "a": 1 })), "accepted");
    let typed = json!({ "$type": "com.example.rules#item", "a": 1 });
    assert_eq!(rule_for("item", typed), "accepted");
    let other = json!({ "$type": "com.example.other", "a": 1 });
    assert_eq!(rule_for("item", other), "record/item/$type: value");
    assert_eq!(
        rule_for("item", json!({ "$type": 5 })),
        "record/item/$type: value"
    );
    assert_eq!(rule_for("item", json!({ "a": "x" })), "record/item/a: type");
}

#[test]
fn a_union_selects_its_member_by_the_type_as_written() {
    let item = json!({ "$type": "com.example.rules#item", "a": 1 });
    let other = json!({ "$type": "com.example.other", "v": "x" });
    for key in ["open", "closed"] {
        assert_eq!(rule_for(key, item.clone()), "accepted");
        assert_eq!(rule_for(key, other.clone()), "accepted");
        let bad = json!({ "$type": "com.example.other" });
        assert_eq!(rule_for(key, bad), format!("record/{key}: required_key"));
        assert_eq!(
            rule_for(key, json!({ "v": "x" })),
            format!("record/{key}: type")
        );
        assert_eq!(rule_for(key, json!("x")), format!("record/{key}: type"));
    }
    // `<nsid>#main` is not the member `<nsid>`: an open union does not
    // validate it and a closed union rejects it.
    let main = json!({ "$type": "com.example.other#main" });
    assert_eq!(rule_for("open", main.clone()), "accepted");
    assert_eq!(rule_for("closed", main), "record/closed/$type: value");
    assert_eq!(
        rule_for("open", json!({ "$type": "com.example.new" })),
        "accepted"
    );
    assert_eq!(
        rule_for("open", json!({ "$type": 5 })),
        "record/open/$type: type"
    );
    assert_eq!(
        rule_for("closed", json!({ "$type": 5 })),
        "record/closed/$type: value"
    );
}

#[test]
fn a_blob_meets_accept_and_max_size_on_a_write() {
    assert_eq!(rule_for("image", blob("image/png", 100)), "accepted");
    assert_eq!(rule_for("image", blob("application/pdf", 0)), "accepted");
    assert_eq!(
        rule_for("image", blob("image/png", 101)),
        "record/image: too_big"
    );
    assert_eq!(
        rule_for("image", blob("text/plain", 1)),
        "record/image/mimeType: value"
    );
    assert_eq!(
        rule_for("image", blob("imagex/png", 1)),
        "record/image/mimeType: value"
    );
    assert_eq!(rule_for("anyBlob", blob("text/plain", 1)), "accepted");
    assert_eq!(
        rule_for("anyBlob", blob("plain", 1)),
        "record/anyBlob/mimeType: blob"
    );
    assert_eq!(
        rule_for("anyBlob", blob("text/plain", -1)),
        "record/anyBlob/size: too_small"
    );
    let mut extra = blob("text/plain", 1);
    extra["name"] = json!("a.txt");
    assert_eq!(rule_for("anyBlob", extra), "record/anyBlob/name: blob");
    let mut untyped = blob("text/plain", 1);
    untyped["$type"] = json!("com.example.blob");
    assert_eq!(rule_for("anyBlob", untyped), "record/anyBlob/$type: value");
    let mut unlinked = blob("text/plain", 1);
    unlinked["ref"] = json!(RAW_CID);
    assert_eq!(rule_for("anyBlob", unlinked), "record/anyBlob/ref: type");
}

#[test]
fn the_legacy_blob_form_is_accepted_on_read_only() {
    let legacy = json!({ "cid": RAW_CID, "mimeType": "image/png" });
    assert_eq!(
        rule_in(BlobRules::Write, "image", legacy.clone()),
        "record/image: blob"
    );
    assert_eq!(rule_in(BlobRules::Read, "image", legacy), "accepted");
    let bad_cid = json!({ "cid": "nope", "mimeType": "image/png" });
    assert_eq!(
        rule_in(BlobRules::Read, "image", bad_cid),
        "record/image: blob"
    );
    // On read the declared type and size are not held to the lexicon.
    assert_eq!(
        rule_in(BlobRules::Read, "image", blob("text/plain", 101)),
        "accepted"
    );
    assert_eq!(
        rule_in(BlobRules::Read, "image", blob("text/plain", -1)),
        "accepted"
    );
    assert_eq!(
        rule_in(BlobRules::Read, "image", blob("text/plain", -2)),
        "record/image/size: too_small"
    );
}

#[test]
fn unknown_is_an_object() {
    assert_eq!(rule_for("anything", json!({ "a": [1, null] })), "accepted");
    assert_eq!(rule_for("anything", json!({})), "accepted");
    for value in [
        json!(false),
        json!(null),
        json!("text"),
        json!([1]),
        json!({ "$bytes": "AQID" }),
        json!({ "$link": RAW_CID }),
    ] {
        assert_eq!(rule_for("anything", value), "record/anything: type");
    }
    // The Reference accepts an object shaped like a blob reference; the
    // specification forbids it.
    assert_eq!(rule_for("anything", blob("text/plain", 1)), "accepted");
}

#[test]
fn a_lexicon_that_cannot_be_evaluated_is_a_schema_error() {
    assert_eq!(rule_for("cycle", json!(1)), "record/cycle: schema");
    assert_eq!(rule_for("dangling", json!(1)), "record/dangling: schema");
    assert_eq!(rule_for("method", json!(1)), "record/method: schema");
    assert_eq!(rule_for("odd", json!("x")), "record/odd: schema");
}

/// `containers` nested objects and arrays: a node, its `children`, a
/// node, and so on.
fn tree(containers: usize) -> Data {
    let mut value: Option<Data> = None;
    for level in (0..containers).rev() {
        value = Some(if level % 2 == 0 {
            Data::Object(
                value
                    .map(|inner| ("children".to_owned(), inner))
                    .into_iter()
                    .collect(),
            )
        } else {
            Data::Array(value.into_iter().collect())
        });
    }
    value.expect("at least one container")
}

#[test]
fn nesting_stops_at_32_levels() {
    // The record is level 1, so `containers` more reach level
    // `containers + 1`.
    let record = |containers: usize| {
        Data::Object(BTreeMap::from([
            (
                TYPE_KEY.to_owned(),
                Data::String("com.example.rules".to_owned()),
            ),
            ("tree".to_owned(), tree(containers)),
        ]))
    };
    let corpus = rules();
    let at_limit = record(MAX_DEPTH - 1);
    assert_eq!(outcome(&validate_data_model(&at_limit)), "accepted");
    assert_eq!(outcome(&corpus.validate_record(&at_limit)), "accepted");
    let beyond = record(MAX_DEPTH);
    let path = format!("record/tree{}", "/children/0".repeat((MAX_DEPTH - 1) / 2));
    let expected = format!("{path}/children: too_deep");
    assert_eq!(outcome(&validate_data_model(&beyond)), expected);
    assert_eq!(outcome(&corpus.validate_record(&beyond)), expected);
}

#[test]
fn the_data_model_rules_hold_at_any_depth() {
    let check = |value: Value| outcome(&validate_data_model(&data(&value)));
    assert_eq!(
        check(json!({ "a": [{ "b": blob("image/png", 1) }] })),
        "accepted"
    );
    // Unknown `$`-prefixed keys are ignored, and the legacy blob form
    // has no `$type`.
    assert_eq!(
        check(json!({ "$unknown": 1, "$type": "com.example.x" })),
        "accepted"
    );
    assert_eq!(
        check(json!({ "b": { "cid": RAW_CID, "mimeType": "image/png" } })),
        "accepted"
    );
    assert_eq!(
        check(json!({ "a": [{ "$type": "" }] })),
        "record/a/0/$type: empty"
    );
    assert_eq!(
        check(json!({ "a": [{ "$type": ["x"] }] })),
        "record/a/0/$type: type"
    );
    assert_eq!(
        check(json!({ "b": blob("image/png", 0) })),
        "record/b/size: too_small"
    );
    assert_eq!(
        check(json!({ "b": blob("", 1) })),
        "record/b/mimeType: empty"
    );
    let mut cbor = blob("image/png", 1);
    cbor["ref"] = json!({ "$link": CBOR_CID });
    assert_eq!(check(json!({ "b": cbor })), "record/b/ref: blob");
    let mut text = blob("image/png", 1);
    text["ref"] = json!(RAW_CID);
    assert_eq!(check(json!({ "b": text })), "record/b/ref: type");
    assert_eq!(check(json!([])), "record: type");
}

#[test]
fn an_error_names_the_path_and_the_rule() {
    let corpus = catalog();
    let record = json!({
        "$type": "example.lexicon.record",
        "integer": 1,
        "formats": { "handle": "123" }
    });
    let error = corpus
        .validate_record(&data(&record))
        .expect_err("invalid handle");
    assert_eq!(error.path, "record/formats/handle");
    assert!(matches!(&error.rule, Rule::Format { format, .. } if format == "handle"));
    assert!(
        error
            .to_string()
            .starts_with("record/formats/handle: invalid handle: ")
    );
    let _: &dyn std::error::Error = &error;
}
