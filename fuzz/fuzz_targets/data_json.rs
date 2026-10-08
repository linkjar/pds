#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        if let Ok(v) = pds_types::Data::from_json_str(s) {
            let cbor = v.to_dag_cbor().expect("encodable");
            let back = pds_types::Data::from_dag_cbor(&cbor).expect("decodable");
            assert_eq!(back, v);
        }
    }
});
