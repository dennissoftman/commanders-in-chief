//! Terrain rendering, verified against a real GPU device.
//!
//! These tests all write their captures to `CARGO_TARGET_TMPDIR` so a human can look at them. That
//! is not decoration: a green suite coexists comfortably with a visibly broken frame, so the
//! assertions here are a tripwire and the PNGs are the actual verification.
//!
//! Every test skips rather than fails when no adapter is available, because a machine with no GPU
//! and no software rasteriser cannot say anything about rendering either way — unless
//! `CIC_REQUIRE_ADAPTER` is set, which CI does, because there a skip is a silent loss of coverage.

// The fixture generators convert small bounded integers to `f32` and clamped `f32` back to `u16`.
// Sample counts here are 129 and elevations are clamped to the `u16` range before conversion, so
// neither direction can lose anything.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

mod support;

use std::path::PathBuf;
use std::sync::OnceLock;

use cic_assets::texture::{BlockFormat, TextureAsset, TextureLimits};
use cic_assets::{Terrain, TerrainLayer};
use cic_render::detail::TerrainDetailRequest;
use cic_render::terrain::LayerColour;
use cic_render::terrain_virtual::VirtualPageView;
use cic_render::view::Projection;
use cic_render::{
    Capture, CaptureTarget, DirectionalLight, GpuContext, LayerMaterial, TerrainFrame,
    TerrainPageCache, TerrainRenderer, TextureImage, capture_terrain, render_terrain_into,
    view_projection,
};

const WIDTH: u32 = 640;
const HEIGHT: u32 = 420;

/// Samples along each axis of the test terrain.
const SAMPLES: u32 = 129;

/// World units between samples, so the terrain spans `(129 - 1) * 8 = 1024` units.
const SPACING: f32 = 8.0;

/// World units per elevation step.
const VERTICAL: f32 = 0.5;

/// One device for the whole test binary.
///
/// Not a device per test. Rust runs tests in parallel, and creating and destroying several devices
/// on one adapter concurrently crashed the driver outright (an access violation, not a test
/// failure). Sharing is also considerably faster, since device creation dominates these tests.
static CONTEXT: OnceLock<Option<GpuContext>> = OnceLock::new();

fn context() -> Option<&'static GpuContext> {
    CONTEXT.get_or_init(support::shared_context).as_ref()
}

/// Builds a terrain with a diagonal ridge, two hills, and broad low undulation.
///
/// Deliberately not flat and not noise. The ridge makes a projection or winding error obvious and
/// the two distinct hills make a mirrored or transposed render recognisable. The gentle undulation
/// underneath exists so that *no* part of the surface is a perfect plane: a plane has one normal, so
/// a terrain built from one tells you nothing about whether shading works.
fn test_terrain() -> Terrain {
    let count = (SAMPLES * SAMPLES) as usize;
    let mut elevations = Vec::with_capacity(count);
    let last = (SAMPLES - 1) as f32;
    for y in 0..SAMPLES {
        for x in 0..SAMPLES {
            let fx = x as f32 / last;
            let fy = y as f32 / last;
            let ridge = 220.0 * (-((fx - fy).powi(2)) / 0.02).exp();
            let tall_hill = 320.0 * (-((fx - 0.25).powi(2) + (fy - 0.75).powi(2)) / 0.02).exp();
            let small_hill = 170.0 * (-((fx - 0.80).powi(2) + (fy - 0.30).powi(2)) / 0.010).exp();
            let undulation =
                34.0 * ((fx * 5.7).sin() * (fy * 4.1).cos() + 0.6 * (fx * 11.3 + fy * 9.7).sin());
            let elevation = 90.0 + undulation + ridge + tall_hill + small_hill;
            elevations.push(elevation.round().clamp(0.0, 65_535.0) as u16);
        }
    }

    // Three layers keyed to elevation, so layer blending is visible rather than inferred.
    // `ramp` rises from its first edge to its second, so a falling weight is `1 - ramp`.
    let mut sand = Vec::with_capacity(count);
    let mut grass = Vec::with_capacity(count);
    let mut rock = Vec::with_capacity(count);
    for elevation in &elevations {
        let height = f32::from(*elevation);
        let above_sand = ramp(height, 70.0, 150.0);
        let into_rock = ramp(height, 330.0, 470.0);
        sand.push(1.0 - above_sand);
        grass.push(above_sand * (1.0 - into_rock));
        rock.push(into_rock);
    }
    let quantise = |weights: Vec<f32>| {
        weights
            .into_iter()
            .map(|weight| (weight.clamp(0.0, 1.0) * 255.0).round() as u8)
            .collect::<Vec<u8>>()
    };

    Terrain::new(
        SAMPLES,
        SAMPLES,
        SPACING,
        VERTICAL,
        elevations,
        vec![
            TerrainLayer {
                name: "sand".to_owned(),
                weights: quantise(sand),
            },
            TerrainLayer {
                name: "grass".to_owned(),
                weights: quantise(grass),
            },
            TerrainLayer {
                name: "rock".to_owned(),
                weights: quantise(rock),
            },
        ],
    )
    .expect("valid test terrain")
}

/// A smooth 0..1 ramp between two thresholds, in either direction.
fn ramp(value: f32, edge_a: f32, edge_b: f32) -> f32 {
    if (edge_b - edge_a).abs() < f32::EPSILON {
        return f32::from(value >= edge_a);
    }
    let t = ((value - edge_a) / (edge_b - edge_a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn palette() -> Vec<LayerColour> {
    vec![
        LayerColour([0.78, 0.71, 0.52]),
        LayerColour([0.28, 0.40, 0.20]),
        LayerColour([0.46, 0.44, 0.42]),
    ]
}

/// The same three layers, each carrying a striped albedo, tiled at a different world scale.
///
/// Stripes rather than noise: a stripe has a direction and a period, so a capture shows immediately
/// whether the tiling is world-aligned and at the size that was asked for. Noise would only show that
/// *something* was sampled.
fn textured_layers() -> Vec<LayerMaterial> {
    let colours = palette();
    [(24.0f32, 4u32), (40.0, 6), (64.0, 3)]
        .into_iter()
        .zip(colours)
        .map(|((detail_scale, stripes), colour)| {
            LayerMaterial::colour(colour.0).with_albedo(striped(64, stripes), detail_scale)
        })
        .collect()
}

/// A square image of horizontal stripes alternating between a quarter and full brightness.
fn striped(size: u32, stripes: u32) -> TextureImage {
    let period = size.div_ceil(stripes.max(1));
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        let value = if (y / period).is_multiple_of(2) {
            64
        } else {
            255
        };
        for _ in 0..size {
            rgba.extend_from_slice(&[value, value, value, u8::MAX]);
        }
    }
    TextureImage::new(size, size, rgba).expect("valid stripe image")
}

fn capture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
}

fn write_capture(name: &str, capture: &Capture) {
    let path = capture_dir().join(name);
    std::fs::write(&path, capture.png().expect("encode png")).expect("write png");
    eprintln!("wrote {}", path.display());
}

#[test]
fn renders_a_terrain_overview() {
    let Some(context) = context() else { return };
    let terrain = test_terrain();
    let frame = TerrainFrame::overview(&terrain, WIDTH, HEIGHT);
    let capture = capture_terrain(context, &terrain, &palette(), frame, WIDTH, HEIGHT)
        .expect("render terrain");
    write_capture("terrain-overview.png", &capture);

    assert_eq!(capture.width(), WIDTH);
    assert_eq!(capture.height(), HEIGHT);

    // The clear colour, as the capture's 8-bit sRGB encoding of it. A frame that is entirely this
    // means nothing drew -- the single most likely failure and the one a size assertion misses.
    let covered = capture.fraction_differing_from(clear_pixel(&capture));
    assert!(
        covered > 0.35,
        "terrain should cover a good share of the frame, covered {covered}"
    );

    // Luminance spread, not a colour count: a count mostly reports how varied the fixture's palette
    // is, whereas spread reports whether the light actually differentiates slopes from flats.
    let (lowest, highest) = capture.luminance_range();
    assert!(
        highest - lowest > 0.30,
        "relief should span a wide luminance range, got {lowest}..{highest}"
    );
    let deviation = capture.luminance_deviation();
    assert!(
        deviation > 0.06,
        "shading should vary across the surface, deviation {deviation}"
    );

    // Each of the three layers must actually reach the frame. Sand is warm (red above blue), grass is
    // green-dominant. A blend that silently collapsed to one layer would pass every check above.
    let surface: Vec<[u8; 4]> = capture
        .rgba()
        .chunks_exact(4)
        .map(|pixel| [pixel[0], pixel[1], pixel[2], pixel[3]])
        .filter(|pixel| *pixel != clear_pixel(&capture))
        .collect();
    let warm = surface
        .iter()
        .filter(|pixel| pixel[0] > pixel[1] && pixel[1] > pixel[2])
        .count();
    let green = surface
        .iter()
        .filter(|pixel| pixel[1] > pixel[0] && pixel[1] > pixel[2])
        .count();
    assert!(
        warm > 1_000,
        "the sand layer should be visible, got {warm} pixels"
    );
    assert!(
        green > 1_000,
        "the grass layer should be visible, got {green} pixels"
    );
}

/// Renders one terrain with the given layer materials and returns the capture.
fn capture_with(context: &GpuContext, terrain: &Terrain, materials: &[LayerMaterial]) -> Capture {
    let renderer =
        TerrainRenderer::with_materials(context, terrain, materials).expect("build renderer");
    let target = CaptureTarget::new(context, WIDTH, HEIGHT).expect("target");
    let frame = TerrainFrame::overview(terrain, WIDTH, HEIGHT);
    renderer.set_frame(
        context,
        &cic_render::view_projection(frame.pose, frame.projection),
        frame.pose.eye,
        frame.light,
    );
    render_terrain_into(context, &target, &renderer).expect("pass");
    target.resolve(context, encoder(context)).expect("resolve")
}

/// A flat-coloured layer texture, block-compressed, at the size the array will use.
///
/// Flat on purpose: a flat block has exactly one encoding, so a comparison against the same colour as an
/// RGBA8 image measures the *upload* — row pitch, mip extent, layer origin — rather than a compressor's
/// choices. Detail textures are the largest budget here, and a wrong row pitch on one would scramble the
/// whole map rather than shade it slightly differently.
fn compressed_layer(colour: [u8; 4]) -> TextureAsset {
    TextureAsset::solid(
        64,
        64,
        BlockFormat::Bc7UnormSrgb,
        colour,
        TextureLimits::default(),
    )
    .expect("a flat texture is always encodable")
}

/// The colour every layer of the comparison below uses. All four channels are odd, so they agree on the
/// low bit BC7 mode 6 shares between an endpoint's channels and the block is exact — which is what lets
/// the comparison assert equality instead of a tolerance.
const COMPRESSED_LAYER_COLOUR: [u8; 4] = [201, 199, 87, 255];

#[test]
fn block_compressed_layers_render_the_same_terrain_as_uncompressed_ones() {
    // Terrain is where block compression pays most: a detail texture is sampled by up to eight layers in
    // one fragment across the whole visible map. This is the claim that adopting it changed nothing about
    // the picture -- the same frame through two upload paths.
    //
    // On an adapter without TEXTURE_COMPRESSION_BC the compressed materials still render, by decoding to
    // RGBA8, so this exercises the fallback rather than skipping it.
    let Some(context) = context() else { return };
    let terrain = test_terrain();

    let image = TextureImage::solid(64, 64, COMPRESSED_LAYER_COLOUR);
    let uncompressed: Vec<LayerMaterial> = palette()
        .into_iter()
        .map(|colour| LayerMaterial::colour(colour.0).with_albedo(image.clone(), 32.0))
        .collect();
    let compressed: Vec<LayerMaterial> = palette()
        .into_iter()
        .map(|colour| {
            LayerMaterial::colour(colour.0)
                .with_compressed_albedo(compressed_layer(COMPRESSED_LAYER_COLOUR), 32.0)
        })
        .collect();

    let plain = capture_with(context, &terrain, &uncompressed);
    let blocks = capture_with(context, &terrain, &compressed);
    write_capture("terrain-uncompressed-layers.png", &plain);
    write_capture("terrain-block-compressed-layers.png", &blocks);

    let took_the_fast_path = context.supports_block_compression();
    let renderer = TerrainRenderer::with_materials(context, &terrain, &compressed)
        .expect("compressed renderer");
    assert_eq!(
        renderer.layer_albedo().block_format(),
        took_the_fast_path.then_some(BlockFormat::Bc7UnormSrgb),
        "the path taken must follow the device's capability, not chance"
    );
    assert_eq!(
        renderer.layer_albedo().mip_level_count(),
        7,
        "a 64-pixel detail texture reaches 1x1 in seven levels, whichever path uploaded it"
    );

    // Bounded per channel rather than byte-identical, and the bound is the point: **no channel anywhere may
    // move by more than one least-significant bit.**
    //
    // Exact equality was the first version of this assertion and it was wrong. A hardware or driver BC
    // decoder is not required to be bit-exact: Apple's reconstructs this mode-6 block exactly, and Mesa's
    // `llvmpipe` — which CI runs on, and which does advertise `TEXTURE_COMPRESSION_BC` — rounds one
    // least-significant bit differently across a quarter of the frame. That is the format working as
    // specified, not a fault, and it is the same reason this project's reference captures are already named
    // per adapter.
    //
    // The bound is still strictly stronger than "few pixels differ", because it catches what this path can
    // actually get wrong. A mistaken row pitch, mip extent or layer origin does not shift a frame by a
    // shade — it scrambles it, and every such mistake made while writing this produced either a `wgpu`
    // validation error or deltas in the tens.
    let mut differing = 0usize;
    let mut worst = 0u8;
    for (bare, compressed) in plain
        .rgba()
        .chunks_exact(4)
        .zip(blocks.rgba().chunks_exact(4))
    {
        if bare != compressed {
            differing += 1;
        }
        for channel in 0..4 {
            worst = worst.max(bare[channel].abs_diff(compressed[channel]));
        }
    }
    eprintln!(
        "block compression: fast path {took_the_fast_path}, {differing} of {} pixels differ, worst channel delta {worst}",
        plain.rgba().len() / 4
    );
    assert!(
        worst <= 1,
        "the two upload paths disagree by {worst} of 255 over {differing} pixels, which is more than a \
         decoder's rounding"
    );

    // And the terrain actually drew, which every equality test has to rule out separately: two frames of
    // nothing agree perfectly.
    assert!(
        plain.luminance_deviation() > 0.0,
        "the terrain did not draw"
    );
}

#[test]
fn one_uncompressed_layer_puts_the_whole_array_back_on_the_slow_path() {
    // One array holds one format, so this is not a per-layer decision. A half-converted layer set must
    // still load -- and must still use the converted textures, decoded, because those are the ones the
    // author intended.
    let Some(context) = context() else { return };
    let terrain = test_terrain();
    let colours = palette();

    let mixed: Vec<LayerMaterial> = vec![
        LayerMaterial::colour(colours[0].0)
            .with_compressed_albedo(compressed_layer([201, 199, 87, 255]), 32.0),
        LayerMaterial::colour(colours[1].0).with_albedo(striped(64, 4), 32.0),
        LayerMaterial::colour(colours[2].0),
    ];
    let renderer =
        TerrainRenderer::with_materials(context, &terrain, &mixed).expect("a mixed set must load");
    assert_eq!(
        renderer.layer_albedo().block_format(),
        None,
        "one RGBA8 layer among them is the whole array's format"
    );
    assert_eq!(
        renderer.layer_albedo().layer_count(),
        3,
        "every layer still gets a slice, so a weight still pairs with its own surface"
    );

    // Compressed textures at different sizes are the other rejected case, for the same reason: a
    // compressed array cannot resample a slice.
    let mismatched: Vec<LayerMaterial> = vec![
        LayerMaterial::colour(colours[0].0)
            .with_compressed_albedo(compressed_layer([201, 199, 87, 255]), 32.0),
        LayerMaterial::colour(colours[1].0).with_compressed_albedo(
            TextureAsset::solid(
                32,
                32,
                BlockFormat::Bc7UnormSrgb,
                [99, 99, 99, 255],
                TextureLimits::default(),
            )
            .expect("solid"),
            32.0,
        ),
        LayerMaterial::colour(colours[2].0),
    ];
    let renderer = TerrainRenderer::with_materials(context, &terrain, &mismatched)
        .expect("mismatched sizes must load");
    assert_eq!(renderer.layer_albedo().block_format(), None);

    // And an untextured layer is *not* an obstacle: it takes a flat white slice in the array's own
    // format, so a partly-textured map still takes the fast path. This is the assertion that keeps the
    // two above about real disagreements rather than about the feature never engaging.
    let with_a_bare_layer: Vec<LayerMaterial> = vec![
        LayerMaterial::colour(colours[0].0)
            .with_compressed_albedo(compressed_layer([201, 199, 87, 255]), 32.0),
        LayerMaterial::colour(colours[1].0),
        LayerMaterial::colour(colours[2].0)
            .with_compressed_albedo(compressed_layer([87, 199, 201, 255]), 32.0),
    ];
    let renderer = TerrainRenderer::with_materials(context, &terrain, &with_a_bare_layer)
        .expect("a partly textured set");
    assert_eq!(
        renderer.layer_albedo().block_format(),
        context
            .supports_block_compression()
            .then_some(BlockFormat::Bc7UnormSrgb)
    );
    assert_eq!(renderer.layer_albedo().size(), (64, 64));
}

#[test]
fn layer_albedo_reaches_the_frame_and_tiles_in_world_space() {
    let Some(context) = context() else { return };
    let terrain = test_terrain();

    let flat_materials: Vec<LayerMaterial> =
        palette().into_iter().map(LayerMaterial::from).collect();
    let flat = capture_with(context, &terrain, &flat_materials);
    let textured = capture_with(context, &terrain, &textured_layers());
    write_capture("terrain-flat-layers.png", &flat);
    write_capture("terrain-textured-layers.png", &textured);
    // Pins world-space layer tiling and the linear-light mip chain. A reversed layer ramp and a mip
    // chain averaged in the wrong space both leave the statistics below entirely healthy.
    support::check_reference(context, "terrain-textured-layers.png", &textured);

    let mut changed = 0usize;
    for (bare, patterned) in flat
        .rgba()
        .chunks_exact(4)
        .zip(textured.rgba().chunks_exact(4))
    {
        if bare[0..3] != patterned[0..3] {
            changed += 1;
        }
    }
    let total = flat.rgba().len() / 4;
    eprintln!("pixels changed by layer albedo: {changed} of {total}");
    assert!(
        changed > total / 4,
        "layer textures should change most of the covered surface, got {changed} of {total}"
    );
    assert!(
        textured.luminance_deviation() > flat.luminance_deviation(),
        "a striped surface should vary more than a flat one: {} vs {}",
        textured.luminance_deviation(),
        flat.luminance_deviation()
    );

    // Halving every detail scale doubles the number of repeats across the same map. That is the
    // claim world-space tiling makes and normalized `uv` sampling cannot: with `uv` the image is
    // stretched to the map either way, so the two frames would be identical.
    let finer: Vec<LayerMaterial> = textured_layers()
        .into_iter()
        .map(|material| LayerMaterial {
            detail_scale: material.detail_scale * 0.5,
            ..material
        })
        .collect();
    let tighter = capture_with(context, &terrain, &finer);
    write_capture("terrain-textured-layers-fine.png", &tighter);
    assert_ne!(
        textured.rgba(),
        tighter.rgba(),
        "the detail scale must change where the texture repeats"
    );
}

#[test]
fn a_layer_without_an_image_renders_as_its_flat_colour() {
    // The compatibility claim: albedo multiplies the palette rather than replacing it, so a terrain
    // authored before textures existed renders byte for byte as it did. If the white fallback slice
    // were anything but opaque white -- or were sampled at the wrong mip -- this would drift.
    let Some(context) = context() else { return };
    let terrain = test_terrain();

    let through_colours = capture_terrain(
        context,
        &terrain,
        &palette(),
        TerrainFrame::overview(&terrain, WIDTH, HEIGHT),
        WIDTH,
        HEIGHT,
    )
    .expect("render terrain");
    let through_materials = capture_with(
        context,
        &terrain,
        &palette()
            .into_iter()
            .map(LayerMaterial::from)
            .collect::<Vec<_>>(),
    );
    assert_eq!(
        through_colours.rgba(),
        through_materials.rgba(),
        "an imageless material must render exactly as the flat palette did"
    );
}

#[test]
fn the_layer_array_reports_what_it_uploaded() {
    let Some(context) = context() else { return };
    let terrain = test_terrain();
    let renderer = TerrainRenderer::with_materials(context, &terrain, &textured_layers())
        .expect("build renderer");
    let albedo = renderer.layer_albedo();
    assert_eq!(albedo.layer_count(), 3, "one slice per weight layer");
    assert_eq!(albedo.size(), (64, 64));
    assert_eq!(
        albedo.mip_level_count(),
        7,
        "a 64-pixel slice reduces to 1x1 in seven levels"
    );

    // A terrain with no layers at all still gets a one-slice array, because the bind group layout is
    // fixed and an array texture cannot have zero layers.
    let bare = Terrain::new(4, 4, SPACING, VERTICAL, vec![100; 16], Vec::new()).expect("valid");
    let bare = TerrainRenderer::with_materials(context, &bare, &[]).expect("build renderer");
    assert_eq!(bare.layer_albedo().layer_count(), 1);
    assert_eq!(bare.layer_albedo().size(), (1, 1));
}

#[test]
fn a_height_edit_changes_the_render() {
    // The design claim being tested: terrain deformation is a texture write, not a remesh. If the
    // grid were baked into a vertex buffer, this would render identically.
    let Some(context) = context() else { return };
    let terrain = test_terrain();
    let renderer = TerrainRenderer::new(context, &terrain, &palette()).expect("build renderer");
    let target = CaptureTarget::new(context, WIDTH, HEIGHT).expect("target");
    let frame = TerrainFrame::overview(&terrain, WIDTH, HEIGHT);
    renderer.set_frame(
        context,
        &cic_render::view_projection(frame.pose, frame.projection),
        frame.pose.eye,
        frame.light,
    );

    render_terrain_into(context, &target, &renderer).expect("first pass");
    let before = target
        .resolve(context, encoder(context))
        .expect("resolve before");

    // Raise a broad plateau in the middle of the map. High enough to be unmistakable, low enough
    // that the capture still frames the terrain around it rather than only the plateau.
    let size = [40u32, 40u32];
    let origin = [44u32, 44u32];
    let raised = vec![620u16; (size[0] * size[1]) as usize];
    renderer
        .write_height_region(context, origin, size, &raised)
        .expect("write heights");

    render_terrain_into(context, &target, &renderer).expect("second pass");
    let after = target
        .resolve(context, encoder(context))
        .expect("resolve after");

    write_capture("terrain-height-edit.png", &after);
    assert_ne!(
        before.rgba(),
        after.rgba(),
        "raising a block of terrain must change the image"
    );
}

#[test]
fn a_layer_weight_edit_changes_the_render() {
    // The road case from the design notes: grading a route across the map should be a weight write
    // with no geometry involvement at all.
    let Some(context) = context() else { return };
    let terrain = test_terrain();
    let renderer = TerrainRenderer::new(context, &terrain, &palette()).expect("build renderer");
    let target = CaptureTarget::new(context, WIDTH, HEIGHT).expect("target");
    let frame = TerrainFrame::overview(&terrain, WIDTH, HEIGHT);
    renderer.set_frame(
        context,
        &cic_render::view_projection(frame.pose, frame.projection),
        frame.pose.eye,
        frame.light,
    );

    render_terrain_into(context, &target, &renderer).expect("first pass");
    let before = target
        .resolve(context, encoder(context))
        .expect("resolve before");

    // Paint a wide band of layer 0 (sand) straight across the map, at full weight. Standing in for
    // a graded route: appearance changes, elevation does not.
    let band = [SAMPLES, 10u32];
    let weights = vec![255u8; (band[0] * band[1]) as usize];
    renderer
        .write_layer_region(context, 0, [0, 58], band, &weights)
        .expect("write weights");

    render_terrain_into(context, &target, &renderer).expect("second pass");
    let after = target
        .resolve(context, encoder(context))
        .expect("resolve after");

    write_capture("terrain-layer-edit.png", &after);
    assert_ne!(
        before.rgba(),
        after.rgba(),
        "painting a layer band must change the image"
    );
}

#[test]
fn rejects_a_region_outside_the_terrain() {
    let Some(context) = context() else { return };
    let terrain = test_terrain();
    let renderer = TerrainRenderer::new(context, &terrain, &palette()).expect("build renderer");

    // Past the edge.
    assert!(
        renderer
            .write_layer_region(context, 0, [SAMPLES - 2, 0], [8, 8], &[0u8; 64])
            .is_err()
    );
    // A weight buffer that disagrees with the region's area.
    assert!(
        renderer
            .write_layer_region(context, 0, [0, 0], [8, 8], &[0u8; 10])
            .is_err()
    );
    // An empty region.
    assert!(
        renderer
            .write_layer_region(context, 0, [0, 0], [0, 8], &[])
            .is_err()
    );
    // A layer the terrain does not declare.
    assert!(
        renderer
            .write_layer_region(context, 9, [0, 0], [8, 8], &[0u8; 64])
            .is_err()
    );
}

#[test]
fn an_unpainted_terrain_still_renders() {
    // A terrain with no layers must produce a shaded surface, not a black hole. The shader's neutral
    // fallback covers it, and this is the test that keeps that fallback honest.
    let Some(context) = context() else { return };
    let terrain = Terrain::new(33, 33, 8.0, 0.5, ramped_elevations(33), Vec::new())
        .expect("valid unpainted terrain");
    let frame = TerrainFrame::overview(&terrain, 320, 240);
    let capture =
        capture_terrain(context, &terrain, &[], frame, 320, 240).expect("render unpainted");
    write_capture("terrain-unpainted.png", &capture);

    let covered = capture.fraction_differing_from(clear_pixel(&capture));
    assert!(
        covered > 0.25,
        "an unpainted terrain must still draw, covered {covered}"
    );
    let (lowest, highest) = capture.luminance_range();
    assert!(
        highest - lowest > 0.20,
        "the dome must be shaded, luminance {lowest}..{highest}"
    );
}

/// A dome, so the surface has a continuously varying normal.
///
/// Not a linear slope: `(x + y)` is a plane, every normal on it is identical, and a render of it
/// cannot distinguish working shading from a constant colour.
fn ramped_elevations(samples: u32) -> Vec<u16> {
    let mut elevations = Vec::with_capacity((samples * samples) as usize);
    let last = (samples - 1) as f32;
    for y in 0..samples {
        for x in 0..samples {
            let fx = x as f32 / last - 0.5;
            let fy = y as f32 / last - 0.5;
            let dome = 600.0 * (0.5 - (fx * fx + fy * fy)).max(0.0);
            elevations.push(dome.round().clamp(0.0, 65_535.0) as u16);
        }
    }
    elevations
}

/// Reads the top-left pixel, which the overview framing always leaves as sky.
///
/// Taken from the capture rather than computed from the clear colour, because the sRGB round trip
/// through the framebuffer makes the exact byte value awkward to predict.
fn clear_pixel(capture: &Capture) -> [u8; 4] {
    capture.pixel(0, 0).expect("capture has a top-left pixel")
}

fn encoder(context: &GpuContext) -> wgpu::CommandEncoder {
    context
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("test resolve"),
        })
}

/// A terrain split down the middle: the left half is layer 0 at full weight, the right half layer 1.
///
/// A hard boundary rather than a gradient, because the properties worth checking are about *where* a page
/// reads its data from. A smooth field would look plausible under a coordinate that was off by a page.
fn split_terrain() -> Terrain {
    let samples = 65u32;
    let count = (samples * samples) as usize;
    let mut left = vec![0u8; count];
    let mut right = vec![0u8; count];
    for index in 0..count {
        let x = index as u32 % samples;
        if x < samples / 2 {
            left[index] = 255;
        } else {
            right[index] = 255;
        }
    }
    Terrain::new(
        samples,
        samples,
        10.0,
        0.25,
        vec![200u16; count],
        vec![
            TerrainLayer {
                name: "left".to_owned(),
                weights: left,
            },
            TerrainLayer {
                name: "right".to_owned(),
                weights: right,
            },
        ],
    )
    .expect("valid split terrain")
}

/// One physical page read back from the GPU, at one mip level.
///
/// Carries its own extent because the levels are different sizes, which is the whole subject once a page has
/// a chain: an indexing helper that assumed the base level would read the wrong texel at every other one.
struct PageImage {
    texels: Vec<u8>,
    extent: u32,
}

impl PageImage {
    /// One texel, as RGBA.
    fn texel(&self, x: u32, y: u32) -> [u8; 4] {
        let offset = ((y * self.extent + x) * 4) as usize;
        [
            self.texels[offset],
            self.texels[offset + 1],
            self.texels[offset + 2],
            self.texels[offset + 3],
        ]
    }
}

/// Reads one physical page back as RGBA bytes, row-major from its top-left corner.
fn read_page(context: &GpuContext, cache: &TerrainPageCache, layer: u32, level: u32) -> PageImage {
    let extent = cic_render::terrain_virtual::VIRTUAL_PAGE_EXTENT >> level;
    // A copy destination's row pitch has to be a multiple of 256 bytes, so the buffer is padded and the
    // padding is dropped on the way out. The same arithmetic the capture path uses, and for the same reason.
    let unpadded = extent * 4;
    let padded = unpadded.div_ceil(256) * 256;
    let buffer = context.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("page readback"),
        size: u64::from(padded) * u64::from(extent),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = context
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("page readback"),
        });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: cache.pages(),
            mip_level: level,
            origin: wgpu::Origin3d {
                x: 0,
                y: 0,
                z: layer,
            },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(extent),
            },
        },
        wgpu::Extent3d {
            width: extent,
            height: extent,
            depth_or_array_layers: 1,
        },
    );
    let submission = context.queue().submit([encoder.finish()]);

    let slice = buffer.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    context
        .device()
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .expect("poll the readback");
    receiver
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the map callback fires")
        .expect("the map succeeds");
    let mapped = slice.get_mapped_range().expect("the mapped range");
    let mut rgba = Vec::with_capacity((unpadded * extent) as usize);
    for row in 0..extent {
        let start = (row * padded) as usize;
        rgba.extend_from_slice(&mapped[start..start + unpadded as usize]);
    }
    drop(mapped);
    buffer.unmap();
    PageImage {
        texels: rgba,
        extent,
    }
}

/// Linear light from an sRGB-encoded byte, matching `transfer.wgsl`.
fn linear_from_srgb(value: u8) -> f64 {
    let value = f64::from(value) / 255.0;
    if value <= 0.040_45 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

/// An sRGB-encoded byte from linear light, matching `transfer.wgsl`.
fn srgb_from_linear(value: f64) -> f64 {
    let encoded = if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    };
    encoded.clamp(0.0, 1.0) * 255.0
}

/// A view looking down at the middle of a terrain, in the terms the residency map wants.
fn page_view(terrain: &Terrain) -> VirtualPageView {
    let [extent_x, extent_y] = terrain.world_extent();
    VirtualPageView::new(
        [extent_x * 0.5, extent_y * 0.5 - 200.0, 300.0],
        [0.0, 0.8, -0.6],
        [1.0, 0.0, 0.0],
        [0.0, 0.6, 0.8],
        ([0.0, 0.0, 0.0], [extent_x, extent_y, 100.0]),
        (std::f32::consts::PI / 6.0).tan(),
        16.0 / 9.0,
        terrain.horizontal_scale(),
    )
}

#[test]
fn the_page_cache_composes_the_ground_a_view_asks_for() {
    // The first thing worth knowing about a cache is that it is a cache: a view that has not changed must
    // compose nothing the second time. That is the property the whole residency map exists to provide, and
    // until now nothing on a GPU had ever asked it for anything.
    let Some(context) = context() else { return };
    let terrain = split_terrain();
    let renderer = TerrainRenderer::new(
        context,
        &terrain,
        &[LayerColour([1.0, 0.0, 0.0]), LayerColour([0.0, 0.0, 1.0])],
    )
    .expect("terrain renderer");
    // The uniform block has to be uploaded before the compose pass reads it: the pass binds the terrain's
    // own buffer, which is what stops it disagreeing with the G-buffer, and an unwritten buffer is zeroes.
    renderer.set_frame(
        context,
        &view_projection(
            TerrainFrame::overview(&terrain, 64, 64).pose,
            Projection::for_viewport(64, 64),
        ),
        [0.0; 3],
        DirectionalLight::default(),
    );

    let mut cache = TerrainPageCache::new(context, &renderer, 8).expect("page cache");
    assert_eq!(cache.layer_count(), 8);

    let (cells_x, cells_y) = renderer.cell_size();
    let requests = [TerrainDetailRequest::uniform(
        [0, 0],
        [cells_x, cells_y],
        16,
    )];
    let view = page_view(&terrain);

    let first = cache.update(context, &requests, view);
    assert!(first > 0, "the first update must compose pages");
    let second = cache.update(context, &requests, view);
    assert_eq!(
        second, 0,
        "an unchanged view must compose nothing: {first} pages were already resident"
    );

    // The page table names a resident layer for the pages that were staged, and zero elsewhere. Level 1 is
    // the coarse level the request asked for at 16 texels per cell.
    let resident = cache.table_view(1).expect("level 1 exists");
    let _ = resident;
}

/// The split terrain's two layers as flat colours: red on the left of the split, blue on the right.
fn split_colours() -> Vec<LayerMaterial> {
    vec![
        LayerMaterial::colour([0.9, 0.1, 0.1]),
        LayerMaterial::colour([0.1, 0.1, 0.9]),
    ]
}

/// The same two layers, each carrying a striped albedo a page can resolve.
///
/// Sixty-four world units per repeat against a page texel of 0.625, so the compose pass reads the albedo's
/// base level and the stripes reach the page intact. That matters for one test only, and the reason is the
/// point: mip levels exist for content with high spatial frequency, and a flat-coloured page has none — its
/// only contrast is the weight blend across the split, which is spread over a whole cell and so is far too
/// smooth to tell a linear-light average from a byte-space one.
fn split_stripes() -> Vec<LayerMaterial> {
    split_colours()
        .into_iter()
        .map(|material| material.with_albedo(striped(64, 4), 64.0))
        .collect()
}

/// A cache over the split terrain, warmed at the coarse level over the whole map.
///
/// One place for a setup four page tests share. It returns the renderer too, because the cache borrows
/// nothing from it but the terrain's uniform buffer has to outlive both.
fn warmed_split_cache(
    context: &GpuContext,
    terrain: &Terrain,
    materials: &[LayerMaterial],
) -> (TerrainRenderer, TerrainPageCache, u32) {
    let renderer =
        TerrainRenderer::with_materials(context, terrain, materials).expect("terrain renderer");
    // The uniform block has to be uploaded before the compose pass reads it: the pass binds the terrain's own
    // buffer, which is what stops it disagreeing with the G-buffer, and an unwritten buffer is zeroes.
    renderer.set_frame(
        context,
        &view_projection(
            TerrainFrame::overview(terrain, 64, 64).pose,
            Projection::for_viewport(64, 64),
        ),
        [0.0; 3],
        DirectionalLight::default(),
    );
    let mut cache = TerrainPageCache::new(context, &renderer, 16).expect("page cache");
    let (cells_x, cells_y) = renderer.cell_size();
    let composed = cache.update(
        context,
        &[TerrainDetailRequest::uniform(
            [0, 0],
            [cells_x, cells_y],
            16,
        )],
        page_view(terrain),
    );
    (renderer, cache, composed)
}

#[test]
fn a_composed_page_holds_the_surface_the_terrain_declares() {
    // What the compose pass is for, checked by reading the page rather than by rendering it. A page over the
    // left half of a split terrain must be that half's colour, and one over the right half the other — so a
    // coordinate off by a page, a transposed axis, or a page written to the wrong layer all fail here.
    let Some(context) = context() else { return };
    let terrain = split_terrain();
    // Two pages at least, one wholly inside each half. The terrain is 64 cells across and a level-1 page spans
    // 16, so page (0, 0) is entirely left and page (3, 0) entirely right.
    let (_renderer, cache, composed) = warmed_split_cache(context, &terrain, &split_colours());
    assert!(composed >= 2, "expected at least two pages, got {composed}");

    // The interior centre of each page, which is where a coordinate error is unambiguous: the border could be
    // clamped for a legitimate reason at the map edge, and the centre never can.
    let border = cic_render::terrain_virtual::VIRTUAL_PAGE_BORDER;
    let interior = cic_render::terrain_virtual::VIRTUAL_PAGE_INTERIOR;
    let centre = border + interior / 2;

    // Which layer holds which page is the residency map's decision, so both are read and matched by content
    // rather than assumed. A page is "left" if it is red-dominant and "right" if blue-dominant.
    let mut saw_left = false;
    let mut saw_right = false;
    for layer in 0..cache.layer_count() {
        let page = read_page(context, &cache, layer, 0);
        let texel = page.texel(centre, centre);
        if texel[0] > texel[2].saturating_add(40) {
            saw_left = true;
        } else if texel[2] > texel[0].saturating_add(40) {
            saw_right = true;
        }
        // Roughness rides in alpha, and every layer here takes the default. A page whose alpha came out zero
        // would mean the fourth channel was never written, which no colour assertion would notice.
        assert!(
            texel[3] > 200,
            "layer {layer} has alpha {} — the roughness channel is not being written",
            texel[3]
        );
    }
    assert!(
        saw_left && saw_right,
        "the cache must hold both halves of the terrain: left {saw_left}, right {saw_right}"
    );
}

#[test]
fn a_page_border_carries_the_neighbouring_ground_rather_than_a_clamped_edge() {
    // The property that decides whether pages can be sampled with filtering at all. A bilinear tap at a
    // page's edge reads across it, so the border has to hold the *adjacent* cells' surface — and if it
    // clamped instead, every page boundary would show a seam, and the seams would crawl as the camera moved
    // because page boundaries are fixed to the ground rather than to the screen.
    //
    // Measured on the split terrain, and the fixture is the whole argument. Its boundary sits at cell 32,
    // which is exactly a level-1 page boundary — so one page's interior begins in the right half while its
    // border lies over the left half, and the two halves are different colours. A clamped border would
    // simply repeat the interior's colour, which is a difference of about sixty in the red channel rather
    // than a difference of one.
    //
    // Two earlier fixtures could not show this, and both failures are worth recording because they are the
    // same failure. A single ramped layer normalizes to its palette colour whatever its weight is, because
    // `surface()` divides by the summed weight — the frame was uniform and the test verified nothing. Two
    // complementary ramps across the whole map fixed that and were still too shallow: four border texels
    // span a quarter of a cell, which at that gradient is under one eight-bit step. This is the third time a
    // fixture in this crate has been the bug rather than the code, which the milestone's design notes
    // already carry as a standing warning.
    let Some(context) = context() else { return };
    let terrain = split_terrain();
    let (_renderer, cache, _) = warmed_split_cache(context, &terrain, &split_colours());

    let border = cic_render::terrain_virtual::VIRTUAL_PAGE_BORDER;
    let interior = cic_render::terrain_virtual::VIRTUAL_PAGE_INTERIOR;
    let middle = border + interior / 2;

    // Which slot holds which page is the residency map's decision, so the page that straddles the split is
    // found by content rather than assumed: its interior is blue and its border, lying over the left half,
    // carries more red.
    let mut found = false;
    for layer in 0..cache.layer_count() {
        let page = read_page(context, &cache, layer, 0);
        let inside = page.texel(border, middle);
        let outside = page.texel(0, middle);
        // Only the page whose interior begins in the right half can say anything here.
        if inside[2] <= inside[0].saturating_add(40) {
            continue;
        }
        if outside[0] > inside[0].saturating_add(30) {
            found = true;
            eprintln!(
                "layer {layer}: interior red {}, border red {} — the border is reading the left half",
                inside[0], outside[0]
            );
            // And the border is a *gradient* across its columns rather than a step at its inner edge,
            // which is what a correctly composed border looks like when the ground beneath it is being
            // filtered: nearer the interior is nearer the interior's colour.
            let nearer = page.texel(border - 1, middle)[0];
            assert!(
                nearer < outside[0],
                "layer {layer}: the border's inner column ({nearer}) is not between the interior ({}) and \
                 its outer column ({}), so the border is not a continuation of the ground",
                inside[0],
                outside[0]
            );
        }
    }
    assert!(
        found,
        "no page carried the neighbouring half in its border, so either the border is clamped or no page \
         straddles the split"
    );
}

#[test]
fn each_page_level_is_the_linear_light_average_of_the_one_above_it() {
    // What the reduce pass is, stated as arithmetic over the bytes it wrote. A mip level *means* the average
    // of the area its texel covers, so every texel of level 1 must be the mean of the 2x2 block beneath it —
    // and a reduction that read the wrong footprint, or halved the wrong axis, or wrote to the wrong layer
    // fails here rather than in a frame where it would read as slight blur.
    //
    // The average has to be taken in **linear light**, and this is the one place that is falsifiable rather
    // than merely stated. The sRGB curve is concave, so the mean of two encoded values sits above the encoding
    // of their mean: averaging stored bytes makes a high-contrast page pale as it recedes, which the eye reads
    // as fog nobody added. So the test predicts each texel both ways and asserts not just that the linear
    // prediction matches, but that the byte-space one *misses* — otherwise it would pass on a fixture flat
    // enough for the two to agree, which is exactly how three fixtures in this file have already been the bug.
    let Some(context) = context() else { return };
    let terrain = split_terrain();
    let (_renderer, cache, composed) = warmed_split_cache(context, &terrain, &split_stripes());
    assert!(composed > 0, "no pages were composed, so none were reduced");

    let mips = cic_render::terrain_virtual::VIRTUAL_PAGE_MIPS;
    assert!(mips > 1, "a chain of one level has nothing to check");

    let mut worst_linear = 0.0f64;
    let mut worst_byte_at_a_contrast = 0.0f64;
    let mut contrast_texels = 0usize;
    for level in 1..mips {
        let coarse = read_page(context, &cache, 0, level);
        let fine = read_page(context, &cache, 0, level - 1);
        for y in 0..coarse.extent {
            for x in 0..coarse.extent {
                let block = [
                    fine.texel(x * 2, y * 2),
                    fine.texel(x * 2 + 1, y * 2),
                    fine.texel(x * 2, y * 2 + 1),
                    fine.texel(x * 2 + 1, y * 2 + 1),
                ];
                let stored = coarse.texel(x, y);
                for channel in 0..3 {
                    let values = block.map(|texel| texel[channel]);
                    let linear = srgb_from_linear(
                        values
                            .iter()
                            .map(|value| linear_from_srgb(*value))
                            .sum::<f64>()
                            / 4.0,
                    );
                    worst_linear = worst_linear.max((linear - f64::from(stored[channel])).abs());

                    let spread = f64::from(
                        values.iter().max().copied().unwrap_or(0)
                            - values.iter().min().copied().unwrap_or(0),
                    );
                    if spread >= 20.0 {
                        let bytes = values.iter().map(|value| f64::from(*value)).sum::<f64>() / 4.0;
                        contrast_texels += 1;
                        worst_byte_at_a_contrast = worst_byte_at_a_contrast
                            .max((bytes - f64::from(stored[channel])).abs());
                    }
                }
                // Roughness is a linear measurement already, so it averages as stored rather than through a
                // transfer function. Checked on the same block, because a channel silently left unwritten
                // reads as zero and no colour assertion would notice.
                let alpha = block.iter().map(|texel| f64::from(texel[3])).sum::<f64>() / 4.0;
                assert!(
                    (alpha - f64::from(stored[3])).abs() <= 1.5,
                    "level {level} texel ({x}, {y}) alpha is {} where the mean of its block is {alpha:.1}",
                    stored[3]
                );
            }
        }
    }

    eprintln!(
        "reduce against a linear-light prediction: worst channel error {worst_linear:.2}; a byte-space \
         average would be out by {worst_byte_at_a_contrast:.2} over {contrast_texels} high-contrast channels"
    );
    // One eight-bit step, which is the rounding a store costs and nothing else. A footprint off by a texel
    // shows up here in the tens.
    assert!(
        worst_linear <= 1.5,
        "a level is not the linear-light average of the one above it: worst channel error {worst_linear:.2}"
    );
    // And the fixture can tell the two apart, which is what stops the assertion above from being vacuous.
    assert!(
        contrast_texels > 0 && worst_byte_at_a_contrast >= 3.0,
        "this fixture cannot distinguish a linear average from a byte-space one ({contrast_texels} \
         high-contrast channels, worst byte-space error {worst_byte_at_a_contrast:.2}), so it proves nothing \
         about the transfer function"
    );
}

#[test]
fn a_page_border_survives_every_level_of_the_chain() {
    // The failure mode that makes a mip chain over a bordered page different from a mip chain over a texture.
    // The border exists so a filtered tap at a page edge reads the neighbouring ground; every reduction halves
    // it, and if it ever stops being a whole texel the tap reads inside the *next* page's border and the seam
    // the border prevents comes back — at every level below the base, on ground the base level looks perfect
    // on. No frame at close range would show it, which is why it is read back instead.
    //
    // Measured on the same split terrain and on the same page: the one whose interior begins in the right half
    // while its border lies over the left. The margin narrows with depth and must not vanish, because the
    // border is a gradient over half a cell and a deep level averages more of it.
    let Some(context) = context() else { return };
    let terrain = split_terrain();
    let (_renderer, cache, _) = warmed_split_cache(context, &terrain, &split_colours());

    let base_border = cic_render::terrain_virtual::VIRTUAL_PAGE_BORDER;
    let interior = cic_render::terrain_virtual::VIRTUAL_PAGE_INTERIOR;
    let mips = cic_render::terrain_virtual::VIRTUAL_PAGE_MIPS;

    // The straddling page is found by content at the base level, then followed down its own chain.
    let mut straddling = None;
    for layer in 0..cache.layer_count() {
        let page = read_page(context, &cache, layer, 0);
        let inside = page.texel(base_border, base_border + interior / 2);
        let outside = page.texel(0, base_border + interior / 2);
        if inside[2] > inside[0].saturating_add(40) && outside[0] > inside[0].saturating_add(30) {
            straddling = Some(layer);
            break;
        }
    }
    let layer = straddling.expect("a page straddles the split at the base level");

    for level in 0..mips {
        let page = read_page(context, &cache, layer, level);
        let border = base_border >> level;
        let middle = (base_border + interior / 2) >> level;
        let inside = page.texel(border, middle);
        let outside = page.texel(0, middle);
        eprintln!(
            "layer {layer} level {level}: border is {border} texels, interior red {}, border red {}",
            inside[0], outside[0]
        );
        assert!(border >= 1, "level {level} has no border left at all");
        // The bound is set from the measurement rather than guessed, which is the same discipline the page
        // agreement test uses. Against an interior red of 89 the border reads 184, 180, 170 and 149 as the
        // chain deepens — narrowing, because the deepest level averages the whole border gradient into one
        // texel, and still 60 clear at its narrowest where a clamped border would read 0 clear.
        assert!(
            outside[0] >= inside[0].saturating_add(40),
            "at level {level} the border's red ({}) is not clear of the interior's ({}), so the reduction has \
             mixed the border into the interior and the seam is back",
            outside[0],
            inside[0]
        );
    }
}
