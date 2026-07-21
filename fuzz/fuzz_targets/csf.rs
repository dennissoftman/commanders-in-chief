#![no_main]

use cic_formats::{CsfLimits, parse_csf};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let limits = CsfLimits {
        maximum_file_bytes: 1024 * 1024,
        maximum_labels: 4_096,
        maximum_strings: 8_192,
        maximum_variants_per_label: 256,
        maximum_label_bytes: 1_024,
        maximum_text_units: 65_536,
        maximum_wave_bytes: 1_024,
    };
    let _ = parse_csf(bytes, "fuzz.csf", limits);
});
