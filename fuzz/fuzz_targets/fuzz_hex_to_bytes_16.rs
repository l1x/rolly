#![no_main]

use libfuzzer_sys::fuzz_target;
use rolly::bench::hex_to_bytes_16;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = hex_to_bytes_16(s);
    }
});
