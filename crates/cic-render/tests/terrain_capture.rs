// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only

use cic_formats::{MapLimits, decode_map_blend, decode_map_height, parse_map};
use cic_render::{HeadlessRenderer, StagedTerrain, TerrainStagingOptions, TextureResourceManager};

#[test]
fn synthetic_layered_terrain_capture_matches_completion_hash() {
    let mut bytes = blend_fixture();
    let sentinel = bytes
        .windows(4)
        .position(|window| window == [0x00, 0x00, 0xDA, 0x7A])
        .expect("blend sentinel");
    bytes[sentinel - 4..sentinel].copy_from_slice(&(-1_i32).to_le_bytes());
    let limits = MapLimits::default();
    let map = parse_map(&bytes, "blend.map", limits).expect("MAP fixture");
    let height = decode_map_height(&map, limits).expect("height fixture");
    let blend = decode_map_blend(&map, &height, limits).expect("blend fixture");
    let mut textures = TextureResourceManager::default();
    textures
        .insert(b"Base", 128, 128, texture_sheet())
        .expect("texture sheet");
    let terrain = StagedTerrain::from_map(
        &height,
        &blend,
        &textures,
        TerrainStagingOptions::SOURCE_BACKGROUND,
    )
    .expect("staged terrain");
    let renderer = pollster::block_on(HeadlessRenderer::new()).expect("headless renderer");
    let capture = renderer
        .capture_terrain(128, 128, &terrain)
        .expect("terrain capture");

    assert_eq!(
        capture.sha256(),
        "d19dee6e96471515ab0b4902e99aa9bed44650b10f975e35a91c427e95f96cad"
    );
}

#[test]
fn synthetic_custom_edge_capture_matches_completion_hash() {
    let bytes = blend_fixture();
    let limits = MapLimits::default();
    let map = parse_map(&bytes, "blend.map", limits).expect("MAP fixture");
    let height = decode_map_height(&map, limits).expect("height fixture");
    let blend = decode_map_blend(&map, &height, limits).expect("blend fixture");
    let mut textures = TextureResourceManager::default();
    textures
        .insert(b"Base", 128, 128, texture_sheet())
        .expect("texture sheet");
    textures
        .insert(b"Shore", 64, 64, edge_sheet())
        .expect("edge sheet");
    let terrain = StagedTerrain::from_map(
        &height,
        &blend,
        &textures,
        TerrainStagingOptions::SOURCE_BACKGROUND,
    )
    .expect("staged terrain");
    let renderer = pollster::block_on(HeadlessRenderer::new()).expect("headless renderer");
    let capture = renderer
        .capture_terrain(128, 128, &terrain)
        .expect("terrain capture");

    assert_eq!(
        capture.sha256(),
        "5f5761f44446d8784b7c0910adee7ede440c9e428a3d4b25be26ce470bfabd27"
    );
}

fn blend_fixture() -> Vec<u8> {
    let hex = include_str!("../../cic-formats/tests/fixtures/blend.map.hex");
    let digits = hex
        .bytes()
        .filter(u8::is_ascii_hexdigit)
        .collect::<Vec<_>>();
    digits
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).expect("ASCII fixture");
            u8::from_str_radix(pair, 16).expect("valid fixture")
        })
        .collect()
}

fn texture_sheet() -> Vec<u8> {
    let mut rgba = vec![0_u8; 128 * 128 * 4];
    fill(&mut rgba, 0, 96, [255, 0, 0, 255]);
    fill(&mut rgba, 32, 96, [0, 255, 0, 255]);
    fill(&mut rgba, 0, 64, [0, 0, 255, 255]);
    fill(&mut rgba, 32, 64, [255, 255, 0, 255]);
    rgba
}

fn edge_sheet() -> Vec<u8> {
    let mut rgba = vec![0_u8; 64 * 64 * 4];
    for y in 0..64 {
        for x in 0..64 {
            let color = match x % 16 {
                0..=3 => [255, 255, 255, 255],
                4..=11 => [240, 48, 192, 255],
                _ => [0, 0, 0, 255],
            };
            let offset = (y * 64 + x) * 4;
            rgba[offset..offset + 4].copy_from_slice(&color);
        }
    }
    rgba
}

fn fill(rgba: &mut [u8], origin_x: usize, origin_y: usize, color: [u8; 4]) {
    for y in origin_y..origin_y + 32 {
        for x in origin_x..origin_x + 32 {
            let offset = (y * 128 + x) * 4;
            rgba[offset..offset + 4].copy_from_slice(&color);
        }
    }
}
