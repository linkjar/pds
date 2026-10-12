#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        if let Ok(v) = pds_types::Data::from_json_str(s) {
            // Whatever the JSON limits admit fits the frame limits in CBOR.
            let cbor = v.to_dag_cbor();
            let back = pds_types::Data::from_dag_cbor_with(&cbor, pds_types::Limits::FRAME)
                .expect("decodable");
            assert_eq!(back, v);
        }
    }
});
