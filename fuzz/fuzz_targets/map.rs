#![no_main]

use cic_formats::{MapLimits, decode_map_blend, decode_map_height, parse_map};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let limits = MapLimits {
        maximum_file_bytes: 1024 * 1024,
        maximum_decompressed_bytes: 1024 * 1024,
        maximum_symbols: 1_024,
        maximum_symbol_bytes: 255,
        maximum_chunks: 4_096,
        maximum_chunk_bytes: 1024 * 1024,
        maximum_height_dimension: 1_024,
        maximum_height_samples: 1024 * 1024,
        maximum_boundaries: 1_024,
        maximum_bitmap_tiles: 2_047,
        maximum_edge_tiles: 2_047,
        maximum_blended_tiles: 4_096,
        maximum_cliff_records: 4_096,
        maximum_texture_classes: 256,
        maximum_texture_name_bytes: 1_024,
        maximum_polygon_triggers: 1_024,
        maximum_polygon_points: 1_024,
        maximum_polygon_total_points: 16_384,
        maximum_water_points: 16_384,
        maximum_trigger_name_bytes: 1_024,
    };
    if let Ok(map) = parse_map(bytes, "fuzz.map", limits) {
        if let Ok(height) = decode_map_height(&map, limits) {
            let _ = decode_map_blend(&map, &height, limits);
        }
    }
});
