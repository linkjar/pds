#![no_main]
//! What a client controls in a write: the JSON body of a procedure and the
//! record in it. The body checks, the record validation and the wording of
//! each refusal must hold for any input.
use libfuzzer_sys::fuzz_target;
use pds_lexicon::Corpus;
use pds_lexicon::validate::validate_data_model;
use pds_types::{Data, Limits};
use pds_xrpc::body;
use pds_xrpc::params::Format;
use serde_json::Value;

fuzz_target!(|data: &[u8]| {
    let Ok(Value::Object(input)) = serde_json::from_slice::<Value>(data) else {
        return;
    };
    // The properties of `createRecord`, in the order its lexicon declares.
    let refusals = [
        body::string(&input, "repo", true, Format::AtIdentifier, None, "$").err(),
        body::string(&input, "collection", true, Format::Nsid, None, "$").err(),
        body::string(&input, "rkey", false, Format::RecordKey, Some(512), "$").err(),
        body::boolean(&input, "validate", "$").err(),
        body::cid(&input, "swapCommit", "$").err(),
        body::array(&input, "writes", "$").err(),
    ];
    for refusal in refusals.into_iter().flatten() {
        assert_eq!(refusal.status.as_u16(), 400);
        assert!(refusal.message.contains(" at $"), "{}", refusal.message);
    }
    let Ok(record) = body::record(&input, "record", "$") else {
        return;
    };
    let Ok(json) = serde_json::to_vec(&Value::Object(record.clone())) else {
        return;
    };
    let Ok(record) = Data::from_json_slice(&json, Limits::JSON_RECORD) else {
        return;
    };
    // A record that passes re-encodes to the block the server stores.
    let block = record.to_dag_cbor();
    assert_eq!(Data::from_dag_cbor(&block).ok().as_ref(), Some(&record));
    for refusal in [
        Corpus::bundled().validate_record(&record).err(),
        validate_data_model(&record).err(),
    ]
    .into_iter()
    .flatten()
    {
        assert!(refusal.reference_message(&record, "$.record").contains(" at $.record"));
    }
});
