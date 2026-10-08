#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        if let Ok(v) = pds_types::Datetime::parse(s) {
            assert_eq!(v.as_str(), s);
        }
    }
});
