#![no_main]

use cic_vfs::{BigLimits, parse_big_archive};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let limits = BigLimits {
        maximum_archive_bytes: 1024 * 1024,
        maximum_directory_bytes: 256 * 1024,
        maximum_entries: 4_096,
        maximum_name_bytes: 1_024,
        maximum_directory_trailer_bytes: 4_096,
    };
    let _ = parse_big_archive(bytes, limits);
});
