#![no_main]

use cic_formats::{WaterIniLimits, parse_water_transparency_ini};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let limits = WaterIniLimits {
        max_file_bytes: 1024 * 1024,
        max_lines: 16_384,
        max_line_bytes: 4_096,
    };
    let _ = parse_water_transparency_ini(bytes, limits);
});
