//! Terrain rendering, verified against a real GPU device.
//!
//! These tests all write their captures to `CARGO_TARGET_TMPDIR` so a human can look at them. That
//! is not decoration: a green suite coexists comfortably with a visibly broken frame, so the
//! assertions here are a tripwire and the PNGs are the actual verification.
//!
//! Every test skips rather than fails when no adapter is available, because a machine with no GPU
//! and no software rasteriser cannot say anything about rendering either way.

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

use cic_assets::{Terrain, TerrainLayer};
use cic_render::terrain::LayerColour;
use cic_render::{
    Capture, CaptureTarget, GpuContext, LayerMaterial, TerrainFrame, TerrainRenderer, TextureImage,
    capture_terrain, render_terrain_into,
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
    CONTEXT
        .get_or_init(|| match pollster::block_on(GpuContext::new()) {
            Ok(context) => {
                eprintln!("adapter: {}", context.adapter_info().name);
                Some(context)
            }
            Err(error) => {
                eprintln!("skipping: no usable adapter ({error})");
                None
            }
        })
        .as_ref()
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
