#![no_main]

use cic_formats::{WndLimits, parse_wnd};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let limits = WndLimits {
        maximum_file_bytes: 1024 * 1024,
        maximum_tokens: 65_536,
        maximum_lines: 16_384,
        ..WndLimits::default()
    };
    let _ = parse_wnd(bytes, limits);
});
