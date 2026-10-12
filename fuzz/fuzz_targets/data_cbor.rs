#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let checked = pds_types::cbor::check(data, pds_types::Limits::RECORD);
    match pds_types::Data::from_dag_cbor(data) {
        Ok(v) => {
            // The decoder accepts canonical input only, so encoding gives
            // the input back, and the check agrees with the decoder.
            assert!(checked.is_ok());
            assert_eq!(v.to_dag_cbor(), data);
            assert!(v.check_limits(pds_types::Limits::RECORD).is_ok());
            let _ = serde_json::to_string(&v);
        }
        Err(e) => assert_eq!(checked, Err(e)),
    }
});
