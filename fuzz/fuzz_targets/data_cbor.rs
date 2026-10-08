#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(v) = pds_types::Data::from_dag_cbor(data) {
        let _ = serde_json::to_string(&v);
    }
});
