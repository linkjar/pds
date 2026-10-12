#![no_main]
//! A DID document from the PLC directory or a web host, and the two
//! answers of handle verification.
use libfuzzer_sys::fuzz_target;
use pds_identity::DidDocument;
use pds_identity::handle::{did_from_txt_records, did_from_well_known};

fuzz_target!(|data: &[u8]| {
    if let Ok(doc) = DidDocument::from_json(data, 64 * 1024) {
        let _ = doc.handle();
        let _ = doc.signing_key();
        let _ = doc.pds_endpoint();
        let _ = doc.service_endpoint("#bsky_appview", None);
    }
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = did_from_well_known(text);
        let records: Vec<&str> = text.split('\n').collect();
        let _ = did_from_txt_records(&records);
    }
});
