#![no_main]
//! A record from a client, checked against the data model rules and
//! against the lexicons of the upstream interop catalog.
use std::path::Path;
use std::sync::LazyLock;

use libfuzzer_sys::fuzz_target;
use pds_lexicon::Corpus;
use pds_lexicon::validate::{validate_data_model, validate_record_key};

static CATALOG: LazyLock<Corpus> = LazyLock::new(|| {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../interop/lexicon/catalog");
    Corpus::load_dir(&dir).expect("the interop catalog loads")
});

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else { return };
    let _ = validate_record_key("tid", text);
    let _ = validate_record_key("nsid", text);
    let _ = validate_record_key("literal:self", text);
    if let Ok(record) = pds_types::Data::from_json_str(text) {
        let _ = validate_data_model(&record);
        let _ = CATALOG.validate_record(&record);
    }
});
