// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Every cic-render test that needs a GPU device, in one target.
//!
//! Deliberately one target rather than several. Cargo runs separate test binaries in parallel, and
//! two processes creating a headless wgpu adapter at the same time crashes the driver on at least
//! one supported configuration (`STATUS_ACCESS_VIOLATION` on Windows). Kept sequentially they are
//! clean. Within this binary a single shared device is created once behind a `OnceLock`, so however
//! many threads the harness uses, adapter creation happens exactly once — and every test below
//! reuses it rather than paying for another 180 MiB of shadow maps.
//!
//! A new GPU test belongs here, not in a target of its own.

use std::sync::OnceLock;

use cic_formats::{MapLimits, decode_map_blend, decode_map_height, parse_map};
use cic_render::{
    Capture, HeadlessRenderer, MapPresentationFrame, MapScene, MapViewCamera, MapViewPasses, Pose,
    RenderError, StagedTerrain, StagedWater, TerrainLighting, TerrainStagingOptions,
    TextureResourceManager, WaterAppearance,
};

/// One device for the whole binary. `None` when the machine has no usable adapter, which is a skip
/// rather than a failure so the suite still runs on headless CI without a software rasterizer.
fn renderer() -> Option<&'static HeadlessRenderer> {
    static RENDERER: OnceLock<Option<HeadlessRenderer>> = OnceLock::new();
    RENDERER
        .get_or_init(|| match pollster::block_on(HeadlessRenderer::new()) {
            Ok(renderer) => Some(renderer),
            Err(RenderError::RequestAdapter(error)) => {
                eprintln!("skipping GPU tests without a headless adapter: {error}");
                None
            }
            Err(error) => panic!("initializing headless renderer: {error}"),
        })
        .as_ref()
}

#[test]
fn synthetic_pose_capture_matches_completion_hash() {
    let Some(renderer) = renderer() else {
        return;
    };
    let capture = renderer
        .capture_triangle(64, 64, Pose::translation(0.25, 0.0).expect("finite pose"))
        .expect("headless capture");
    assert!(matches!(
        renderer.capture_triangle(4_097, 1, Pose::IDENTITY),
        Err(RenderError::CaptureTooLarge)
    ));
    let expected = include_str!("fixtures/synthetic-pose.rgba.sha256").trim();
    assert_eq!(capture.sha256(), expected);
}

#[test]
fn synthetic_layered_terrain_capture_matches_completion_hash() {
    let mut bytes = blend_fixture();
    let sentinel = bytes
        .windows(4)
        .position(|window| window == [0x00, 0x00, 0xDA, 0x7A])
        .expect("blend sentinel");
    bytes[sentinel - 4..sentinel].copy_from_slice(&(-1_i32).to_le_bytes());
    let terrain = staged_terrain(&bytes, false);
    let Some(renderer) = renderer() else {
        return;
    };
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
    let terrain = staged_terrain(&blend_fixture(), true);
    let Some(renderer) = renderer() else {
        return;
    };
    let capture = renderer
        .capture_terrain(128, 128, &terrain)
        .expect("terrain capture");

    assert_eq!(
        capture.sha256(),
        "5f5761f44446d8784b7c0910adee7ede440c9e428a3d4b25be26ce470bfabd27"
    );
}

/// The load-bearing test for the deferred path: it drives the *whole* real resource construction
/// (every bind group layout, the caster cascade bind groups, the five shadow cascade layers, the
/// multisampled G-buffer, the occlusion targets) and then the whole real pass sequence.
///
/// wgpu validation failures in that constructor previously reached the user as runtime panics from
/// `map-view`, because nothing headless could reach it. Any such failure now fails here.
#[test]
fn deferred_map_capture_renders_the_whole_real_path() {
    let scene = capture_scene();
    let Some(renderer) = renderer() else {
        return;
    };
    let capture = renderer
        .capture_map_view(
            [96, 64],
            &scene,
            MapViewCamera::CENTERED,
            MapPresentationFrame::ZERO,
            MapViewPasses::ALL,
        )
        .expect("deferred MAP scene capture");

    assert_eq!((capture.width(), capture.height()), (96, 64));
    assert_eq!(capture.rgba().len(), 96 * 64 * 4);
    assert!(
        capture.rgba().chunks_exact(4).all(|pixel| pixel[3] == 255),
        "the composite must write an opaque frame"
    );
    // A path that failed to draw anything would still return a valid buffer, so the frame has to be
    // shown to carry more than the clear colour.
    let distinct = capture
        .rgba()
        .chunks_exact(4)
        .map(|pixel| u32::from_be_bytes([pixel[0], pixel[1], pixel[2], pixel[3]]))
        .collect::<std::collections::BTreeSet<_>>();
    assert!(
        distinct.len() > 16,
        "expected shaded terrain, got {} distinct colours",
        distinct.len()
    );
}

/// Determinism is the property the whole capture exists for: if identical explicit inputs did not
/// reproduce an identical image, no hash comparison across a tuning change would mean anything.
#[test]
fn deferred_map_capture_is_reproducible_and_pose_dependent() {
    let scene = capture_scene();
    let Some(renderer) = renderer() else {
        return;
    };
    let capture = |camera, seconds: f32| -> Capture {
        renderer
            .capture_map_view(
                [80, 48],
                &scene,
                camera,
                MapPresentationFrame::new(seconds).expect("explicit presentation time"),
                MapViewPasses::ALL,
            )
            .expect("deferred MAP scene capture")
    };
    let baseline = capture(MapViewCamera::CENTERED, 0.0);
    assert_eq!(
        baseline.sha256(),
        capture(MapViewCamera::CENTERED, 0.0).sha256(),
        "identical explicit inputs must reproduce the frame exactly"
    );

    let rotated = MapViewCamera::new(std::f32::consts::FRAC_PI_2, 240.0).expect("finite placement");
    assert_ne!(
        baseline.sha256(),
        capture(rotated, 0.0).sha256(),
        "a different pose must reach the shadow fit and the view matrix"
    );
}

/// Isolating a term may only ever brighten the frame: both flags work by leaving a target at its
/// neutral clear value, so a capture with one off is a per-pixel upper bound on the capture with it
/// on. A flag wired to the wrong pass, or a shadow cascade left uncleared when its draws are
/// skipped, would show up as a pixel that got *darker*.
///
/// Only the occlusion flag is asserted to reach the frame. This fixture is eight by two nearly flat
/// cells with no scenery, so it has nothing to cast a shadow, and requiring the shadow flag to move
/// the image here would be asserting a property of the fixture rather than of the renderer. Caster
/// coverage over real relief is what `map-view --shadows off` against an installed map is for.
#[test]
fn pass_isolation_only_ever_brightens_the_frame() {
    let scene = capture_scene();
    let Some(renderer) = renderer() else {
        return;
    };
    let capture = |passes| -> Capture {
        renderer
            .capture_map_view(
                [80, 48],
                &scene,
                MapViewCamera::CENTERED,
                MapPresentationFrame::ZERO,
                passes,
            )
            .expect("deferred MAP scene capture")
    };
    let full = capture(MapViewPasses::ALL);
    for passes in [
        MapViewPasses {
            shadows: false,
            ..MapViewPasses::ALL
        },
        MapViewPasses {
            occlusion: false,
            ..MapViewPasses::ALL
        },
        MapViewPasses {
            shadows: false,
            occlusion: false,
        },
    ] {
        let isolated = capture(passes);
        if !passes.occlusion {
            assert_ne!(
                full.sha256(),
                isolated.sha256(),
                "{passes:?} must reach the frame"
            );
        }
        let darker = full
            .rgba()
            .chunks_exact(4)
            .zip(isolated.rgba().chunks_exact(4))
            .filter(|(lit, unlit)| {
                let sum =
                    |pixel: &[u8]| u32::from(pixel[0]) + u32::from(pixel[1]) + u32::from(pixel[2]);
                // A tolerance, not equality: dropping the caster draws changes nothing about the
                // shadow comparison itself, but resolve and filtering still round per pixel.
                sum(unlit) + 6 < sum(lit)
            })
            .count();
        assert_eq!(darker, 0, "{passes:?} left {darker} pixels darker");
    }
}

/// The composite tonemaps into `0..=1` and returns the result unencoded, because the colour target
/// applies the sRGB transfer function in hardware. Capturing into a linear target instead stored
/// linear values in a file that declares itself sRGB, which displayed several times too dark — the
/// midtones are where that shows, so this pins them.
///
/// A frame whose lighting genuinely averaged this low would be night, not the neutral preview
/// lighting this fixture uses; storing the same frame linearly would put the median near 30.
#[test]
fn deferred_map_capture_is_encoded_for_display_not_stored_linear() {
    let scene = capture_scene();
    let Some(renderer) = renderer() else {
        return;
    };
    let capture = renderer
        .capture_map_view(
            [96, 64],
            &scene,
            MapViewCamera::CENTERED,
            MapPresentationFrame::ZERO,
            MapViewPasses::ALL,
        )
        .expect("deferred MAP scene capture");
    let mut luminance = capture
        .rgba()
        .chunks_exact(4)
        .map(|pixel| {
            // Integer weights out of 10000, so the comparison needs no float cast at all.
            (2126 * u32::from(pixel[0]) + 7152 * u32::from(pixel[1]) + 722 * u32::from(pixel[2]))
                / 10_000
        })
        .collect::<Vec<_>>();
    luminance.sort_unstable();
    let median = luminance[luminance.len() / 2];
    assert!(
        median > 60,
        "median luminance {median} suggests the frame was stored linearly rather than encoded"
    );
}

#[test]
fn deferred_map_capture_rejects_invalid_dimensions_and_placements() {
    assert!(matches!(
        MapViewCamera::new(f32::NAN, 232.0),
        Err(RenderError::InvalidCameraPlacement)
    ));
    assert!(matches!(
        MapViewCamera::new(0.0, f32::INFINITY),
        Err(RenderError::InvalidCameraPlacement)
    ));
    assert!(matches!(
        MapViewCamera::CENTERED.with_focus([0.0, f32::NAN]),
        Err(RenderError::InvalidCameraPlacement)
    ));
    let scene = capture_scene();
    let Some(renderer) = renderer() else {
        return;
    };
    assert!(matches!(
        renderer.capture_map_view(
            [0, 64],
            &scene,
            MapViewCamera::CENTERED,
            MapPresentationFrame::ZERO,
            MapViewPasses::ALL
        ),
        Err(error) if error.to_string().contains("capture dimensions")
    ));
}

fn capture_scene() -> MapScene {
    MapScene::terrain_only(
        staged_terrain(&blend_fixture(), true),
        StagedWater::empty(),
        WaterAppearance::without_caustics(),
        TerrainLighting::preview(),
    )
}

fn staged_terrain(bytes: &[u8], with_edge_sheet: bool) -> StagedTerrain {
    let limits = MapLimits::default();
    let map = parse_map(bytes, "blend.map", limits).expect("MAP fixture");
    let height = decode_map_height(&map, limits).expect("height fixture");
    let blend = decode_map_blend(&map, &height, limits).expect("blend fixture");
    let mut textures = TextureResourceManager::default();
    textures
        .insert(b"Base", 128, 128, texture_sheet())
        .expect("texture sheet");
    if with_edge_sheet {
        textures
            .insert(b"Shore", 64, 64, edge_sheet())
            .expect("edge sheet");
    }
    StagedTerrain::from_map(
        &height,
        &blend,
        &textures,
        TerrainStagingOptions::SOURCE_BACKGROUND,
    )
    .expect("staged terrain")
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
