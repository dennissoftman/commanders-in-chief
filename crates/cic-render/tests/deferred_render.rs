//! The deferred chain, verified against a real GPU device.
//!
//! Captures land in `CARGO_TARGET_TMPDIR` for inspection. The assertions are a tripwire; the images
//! are the verification. Shadows in particular are something a test can confirm the *presence* of but
//! not the correctness of — acne, peter-panning, and a cascade seam all pass a "darker pixels exist"
//! check.

// The fixture generator converts small bounded integers to `f32` and clamped `f32` back to `u16`.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

mod support;

use std::path::PathBuf;
use std::sync::OnceLock;

use cic_assets::{Terrain, TerrainLayer};
use cic_camera::CameraPose;
use cic_render::terrain::LayerColour;
use cic_render::{
    Capture, CaptureTarget, Clouds, DeferredFrame, DeferredRenderer, DeferredTargets, Environment,
    Fog, GpuContext, TerrainRenderer, WaterBody, WaterMaterial, WaterSurface,
};

const WIDTH: u32 = 720;
const HEIGHT: u32 = 480;
const SAMPLES: u32 = 193;
const SPACING: f32 = 8.0;
const VERTICAL: f32 = 0.5;

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

/// A terrain with a steep ridge and a tall spire.
///
/// Shaped specifically to make shadows falsifiable. A gentle heightfield casts almost nothing, so a
/// broken shadow pass and a correct one look identical on it. The ridge runs across the light and the
/// spire stands alone, so each must throw a long shadow onto ground that is otherwise open — and if
/// they do not, the image says so immediately.
fn shadowing_terrain() -> Terrain {
    let count = (SAMPLES * SAMPLES) as usize;
    let mut elevations = Vec::with_capacity(count);
    let last = (SAMPLES - 1) as f32;
    for y in 0..SAMPLES {
        for x in 0..SAMPLES {
            let fx = x as f32 / last;
            let fy = y as f32 / last;
            // A steep, *narrow* ridge across the map. Narrow matters: at a 44-degree sun a 210-unit
            // ridge throws a 219-unit shadow, so a ridge wider than that casts entirely onto its own
            // unlit back slope and its shadow is invisible however correct the pass is.
            let ridge = 620.0 * (-((fy - 0.62).powi(2)) / 0.00035).exp();
            // A narrow spire, tall enough to cast well clear of itself.
            let spire = 900.0 * (-((fx - 0.30).powi(2) + (fy - 0.28).powi(2)) / 0.0009).exp();
            // A shallow bowl, so occlusion has a concave surface to find.
            let bowl = -140.0 * (-((fx - 0.72).powi(2) + (fy - 0.30).powi(2)) / 0.010).exp();
            let undulation = 22.0 * ((fx * 6.1).sin() * (fy * 4.7).cos());
            let elevation = 200.0 + undulation + ridge + spire + bowl;
            elevations.push(elevation.round().clamp(0.0, 65_535.0) as u16);
        }
    }

    let mut ground = Vec::with_capacity(count);
    let mut rock = Vec::with_capacity(count);
    for elevation in &elevations {
        let height = f32::from(*elevation);
        let into_rock = ramp(height, 420.0, 700.0);
        ground.push(((1.0 - into_rock) * 255.0).round() as u8);
        rock.push((into_rock * 255.0).round() as u8);
    }

    Terrain::new(
        SAMPLES,
        SAMPLES,
        SPACING,
        VERTICAL,
        elevations,
        vec![
            TerrainLayer {
                name: "ground".to_owned(),
                weights: ground,
            },
            TerrainLayer {
                name: "rock".to_owned(),
                weights: rock,
            },
        ],
    )
    .expect("valid shadowing terrain")
}

fn ramp(value: f32, edge_a: f32, edge_b: f32) -> f32 {
    if (edge_b - edge_a).abs() < f32::EPSILON {
        return f32::from(value >= edge_a);
    }
    let t = ((value - edge_a) / (edge_b - edge_a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn palette() -> Vec<LayerColour> {
    vec![
        LayerColour([0.42, 0.46, 0.28]),
        LayerColour([0.52, 0.49, 0.45]),
    ]
}

/// A low, oblique camera. Shadows are most legible when the view is not aligned with the light.
fn pose(terrain: &Terrain) -> CameraPose {
    let [extent_x, extent_y] = terrain.world_extent();
    let focus = [extent_x * 0.45, extent_y * 0.45, 0.0];
    let eye = [
        focus[0] + extent_x * 0.42,
        focus[1] - extent_y * 0.72,
        extent_x * 0.40,
    ];
    CameraPose {
        eye,
        focus,
        forward: [-0.42, 0.72, -0.40],
    }
}

fn capture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
}

fn write_capture(name: &str, capture: &Capture) {
    let path = capture_dir().join(name);
    std::fs::write(&path, capture.png().expect("encode png")).expect("write png");
    eprintln!("wrote {}", path.display());
}

struct Harness {
    terrain: Terrain,
    renderer: TerrainRenderer,
    deferred: DeferredRenderer,
    targets: DeferredTargets,
    output: CaptureTarget,
}

fn harness(context: &GpuContext) -> Harness {
    harness_for(context, shadowing_terrain())
}

fn harness_for(context: &GpuContext, terrain: Terrain) -> Harness {
    let renderer =
        TerrainRenderer::new(context, &terrain, &palette()).expect("build terrain renderer");
    let targets = DeferredTargets::new(context, WIDTH, HEIGHT).expect("allocate targets");
    let deferred = DeferredRenderer::new(
        context,
        &renderer,
        &targets,
        cic_render::gpu::CAPTURE_FORMAT,
    )
    .expect("build deferred renderer");
    let output = CaptureTarget::new(context, WIDTH, HEIGHT).expect("output target");
    Harness {
        terrain,
        renderer,
        deferred,
        targets,
        output,
    }
}

fn encoder(context: &GpuContext) -> wgpu::CommandEncoder {
    context
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("test resolve"),
        })
}

/// Renders the whole chain and resolves the composite.
fn render(context: &GpuContext, harness: &Harness, frame: DeferredFrame) -> Capture {
    harness
        .deferred
        .set_frame(context, &harness.renderer, &[], &[], frame)
        .expect("upload frame uniforms");
    harness.deferred.render(
        context,
        &harness.renderer,
        &[],
        &[],
        &harness.targets,
        harness.output.colour_view(),
    );
    harness
        .output
        .resolve(context, encoder(context))
        .expect("resolve composite")
}

#[test]
fn the_deferred_chain_produces_a_lit_scene() {
    let Some(context) = context() else { return };
    let harness = harness(context);
    let frame = DeferredFrame::new(pose(&harness.terrain), WIDTH, HEIGHT);
    let capture = render(context, &harness, frame);
    write_capture("deferred-composite.png", &capture);
    // The whole chain in one image: cascades, occlusion, deferred lighting, and the tone curve. The
    // assertions below cannot tell a correct frame from several wrong ones that share its statistics.
    support::check_reference(context, "deferred-composite.png", &capture);

    assert_eq!(capture.width(), WIDTH);
    assert_eq!(capture.height(), HEIGHT);

    // The lighting pass paints its own sky gradient where coverage is zero, so an all-one-colour
    // frame means either nothing drew or the composite never ran.
    let (lowest, highest) = capture.luminance_range();
    assert!(
        highest - lowest > 0.35,
        "the scene should span a wide luminance range, got {lowest}..{highest}"
    );
    let deviation = capture.luminance_deviation();
    assert!(
        deviation > 0.08,
        "shading and shadowing should vary widely, deviation {deviation}"
    );
}

#[test]
fn shadows_darken_the_scene_against_an_unshadowed_control() {
    // A shadow test that cannot pass by accident, and whose control differs in *only* the shadowing.
    //
    // An earlier version compared an oblique sun against an overhead one, which is not a control at
    // all: moving the sun also changes every surface's incidence, so the two frames would differ even
    // with the shadow pass deleted. Instead the light, the camera, the geometry, and the occlusion are
    // held identical and only the shadow distance is collapsed, which puts every receiver outside all
    // four cascades and makes `shadow_visibility` return fully lit.
    let Some(context) = context() else { return };
    let harness = harness(context);

    let mut shadowed = DeferredFrame::new(pose(&harness.terrain), WIDTH, HEIGHT);
    // The sun is placed so shadows fall *toward* the camera. Shadows travel opposite the direction
    // toward the light, so a sun in the same quadrant as the camera throws every shadow behind its
    // own caster and out of frame -- which makes a working shadow pass and a broken one produce
    // near-identical images, and is exactly what an earlier version of this test measured.
    shadowed.light.direction = [-0.50, 0.52, 0.69];
    let with_shadows = render(context, &harness, shadowed);
    write_capture("deferred-oblique-sun.png", &with_shadows);

    let mut control = shadowed;
    control.shadow_distance = 0.5;
    let without_shadows = render(context, &harness, control);
    write_capture("deferred-no-shadows.png", &without_shadows);

    let mean = |capture: &Capture| {
        let total = capture.rgba().len() / 4;
        let sum: f32 = capture
            .rgba()
            .chunks_exact(4)
            .map(|pixel| {
                0.2126 * f32::from(pixel[0]) / 255.0
                    + 0.7152 * f32::from(pixel[1]) / 255.0
                    + 0.0722 * f32::from(pixel[2]) / 255.0
            })
            .sum();
        sum / total as f32
    };

    let shadowed_mean = mean(&with_shadows);
    let control_mean = mean(&without_shadows);
    eprintln!("mean luminance: shadowed {shadowed_mean}, control {control_mean}");
    assert!(
        control_mean - shadowed_mean > 0.01,
        "shadowing must measurably darken the scene: {shadowed_mean} against {control_mean}"
    );

    // And the darkening must be *localised*, not a uniform dimming. A shadow occupies part of the
    // frame; if every pixel dropped by the same amount, something is scaling the whole image instead.
    let mut darkened = 0usize;
    let mut unchanged = 0usize;
    for (lit, shade) in without_shadows
        .rgba()
        .chunks_exact(4)
        .zip(with_shadows.rgba().chunks_exact(4))
    {
        let difference = i32::from(lit[1]) - i32::from(shade[1]);
        if difference > 12 {
            darkened += 1;
        } else if difference.abs() <= 1 {
            unchanged += 1;
        }
    }
    eprintln!("pixels darkened {darkened}, unchanged {unchanged}");
    assert!(
        darkened > 4_000,
        "a cast shadow should cover real area, got {darkened} pixels"
    );
    assert!(
        unchanged > darkened,
        "most of the frame should be untouched by shadowing: {unchanged} unchanged against          {darkened} darkened"
    );
}

#[test]
fn a_low_sun_casts_long_shadows_across_open_ground() {
    // A low sun is the demanding case and the diagnostic one. Shadow length scales with the cotangent
    // of the sun's elevation, so at 20 degrees a 210-unit ridge throws a 580-unit shadow that must
    // clear its own base and run across open ground. It is also where acne and peter-panning show
    // worst, because the light grazes the surface and every depth comparison is near its own bias.
    let Some(context) = context() else { return };
    let harness = harness(context);
    let mut frame = DeferredFrame::new(pose(&harness.terrain), WIDTH, HEIGHT);
    frame.light.direction = [-0.30, 0.90, 0.32];
    let capture = render(context, &harness, frame);
    // The resulting capture has dead-straight shadow boundaries and an L-shaped lit strip along two
    // map edges. Both are correct: a straight ridge casts a straight-edged shadow, and at this sun
    // elevation the shadow's lateral offset leaves the map's own edge uncovered. A flat-terrain
    // capture from the same camera shows none of it, which is the control that rules out a cascade
    // seam -- the thing this pattern looks like at first glance.
    write_capture("deferred-low-sun.png", &capture);

    let mut control = frame;
    control.shadow_distance = 0.5;
    let unshadowed = render(context, &harness, control);

    let mut darkened = 0usize;
    for (lit, shade) in unshadowed
        .rgba()
        .chunks_exact(4)
        .zip(capture.rgba().chunks_exact(4))
    {
        if i32::from(lit[1]) - i32::from(shade[1]) > 12 {
            darkened += 1;
        }
    }
    eprintln!("low sun darkened {darkened} pixels");
    assert!(
        darkened > 15_000,
        "a low sun should shadow a large area, got {darkened} pixels"
    );
}

#[test]
fn ambient_occlusion_is_computed_and_varies() {
    // Read the AO target directly, so a broken occlusion pass cannot hide behind the lighting.
    let Some(context) = context() else { return };
    let harness = harness(context);
    let frame = DeferredFrame::new(pose(&harness.terrain), WIDTH, HEIGHT);
    harness
        .deferred
        .set_frame(context, &harness.renderer, &[], &[], frame)
        .expect("upload frame uniforms");
    harness.deferred.render(
        context,
        &harness.renderer,
        &[],
        &[],
        &harness.targets,
        harness.output.colour_view(),
    );

    // The AO target is single-channel, so copy it through a colour capture by rendering the composite
    // and reading the occlusion's effect is indirect. Instead assert on the composite twice: once as
    // rendered, and once with the terrain flattened, where there is nothing to occlude.
    let rendered = harness
        .output
        .resolve(context, encoder(context))
        .expect("resolve");

    let flat = Terrain::new(
        SAMPLES,
        SAMPLES,
        SPACING,
        VERTICAL,
        vec![400u16; (SAMPLES * SAMPLES) as usize],
        Vec::new(),
    )
    .expect("valid flat terrain");
    let flat_renderer = TerrainRenderer::new(context, &flat, &[]).expect("flat renderer");
    let flat_targets = DeferredTargets::new(context, WIDTH, HEIGHT).expect("flat targets");
    let flat_deferred = DeferredRenderer::new(
        context,
        &flat_renderer,
        &flat_targets,
        cic_render::gpu::CAPTURE_FORMAT,
    )
    .expect("flat deferred");
    let flat_output = CaptureTarget::new(context, WIDTH, HEIGHT).expect("flat output");
    let flat_frame = DeferredFrame::new(pose(&harness.terrain), WIDTH, HEIGHT);
    flat_deferred
        .set_frame(context, &flat_renderer, &[], &[], flat_frame)
        .expect("upload flat uniforms");
    flat_deferred.render(
        context,
        &flat_renderer,
        &[],
        &[],
        &flat_targets,
        flat_output.colour_view(),
    );
    let flat_capture = flat_output
        .resolve(context, encoder(context))
        .expect("resolve flat");
    write_capture("deferred-flat.png", &flat_capture);

    // A plane has one normal, no self-shadowing, and nothing to occlude, so its luminance spread is
    // necessarily narrower than the shaped terrain's.
    let (shaped_low, shaped_high) = rendered.luminance_range();
    let (flat_low, flat_high) = flat_capture.luminance_range();
    assert!(
        (shaped_high - shaped_low) > (flat_high - flat_low),
        "shaped terrain should span more luminance than a plane: \
         {shaped_low}..{shaped_high} vs {flat_low}..{flat_high}"
    );
}

#[test]
fn a_singular_camera_is_reported_rather_than_rendered() {
    // A collapsed projection cannot be inverted, so no world position can be reconstructed. Reporting
    // it beats rendering a frame where every pixel reconstructs to the origin.
    let Some(context) = context() else { return };
    let harness = harness(context);
    let mut frame = DeferredFrame::new(pose(&harness.terrain), WIDTH, HEIGHT);
    frame.projection.near = 1.0;
    frame.projection.far = 1.0;
    frame.projection.vertical_fov = 0.0;
    let error = harness
        .deferred
        .set_frame(context, &harness.renderer, &[], &[], frame)
        .expect_err("a singular camera must be refused");
    assert!(
        matches!(error, cic_render::RenderError::SingularCamera),
        "got {error:?}"
    );
}

#[test]
fn the_chain_renders_into_a_bgra_target() {
    // What presentation actually hands the composite. Surfaces commonly offer `Bgra8UnormSrgb` rather
    // than the RGBA the capture path uses, and a pipeline built for the wrong one fails at creation.
    // This is the cheapest way to verify the output-format plumbing without opening a window.
    let Some(context) = context() else { return };
    let terrain = shadowing_terrain();
    let renderer =
        TerrainRenderer::new(context, &terrain, &palette()).expect("build terrain renderer");
    let targets = DeferredTargets::new(context, WIDTH, HEIGHT).expect("allocate targets");

    let format = wgpu::TextureFormat::Bgra8UnormSrgb;
    let deferred = DeferredRenderer::new(context, &renderer, &targets, format)
        .expect("a BGRA composite pipeline must build");

    let output = context.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("bgra output"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = output.create_view(&wgpu::TextureViewDescriptor::default());

    let frame = DeferredFrame::new(pose(&terrain), WIDTH, HEIGHT);
    deferred
        .set_frame(context, &renderer, &[], &[], frame)
        .expect("upload frame uniforms");
    deferred.render(context, &renderer, &[], &[], &targets, &view);

    // Validation errors surface asynchronously, so drain the queue before declaring success.
    context
        .device()
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(10)),
        })
        .expect("the BGRA chain must submit and complete cleanly");
}

#[test]
fn moving_the_camera_changes_the_frame() {
    let Some(context) = context() else { return };
    let harness = harness(context);
    let first = render(
        context,
        &harness,
        DeferredFrame::new(pose(&harness.terrain), WIDTH, HEIGHT),
    );

    let mut moved = pose(&harness.terrain);
    moved.eye[0] += 300.0;
    moved.eye[2] += 120.0;
    let second = render(context, &harness, DeferredFrame::new(moved, WIDTH, HEIGHT));

    assert_ne!(
        first.rgba(),
        second.rgba(),
        "the camera moved, so the frame must differ"
    );
}

// -------------------------------------------------------------------------------------------------
// Water
//
// Every assertion below is differential: the same scene with and without water, or at two scene
// times. That is deliberate. An absolute threshold on a water frame mostly reports how the fixture
// was lit, whereas "these two frames differ over this fraction of the image, and only over that
// fraction" states a property the pass actually has to hold.
// -------------------------------------------------------------------------------------------------

/// A clear basin with a definite rim, and open ground around it.
///
/// A separate fixture from `shadowing_terrain` for the same reason the shadow tests needed their own
/// shape. That terrain's spire puts its elevation range in the hundreds, so any water level high
/// enough to fill its bowl also drowns the plain, and a capture of the result says nothing about the
/// shoreline. This one has a single basin, a single rim, and a level that separates them.
fn basin_terrain() -> Terrain {
    let count = (SAMPLES * SAMPLES) as usize;
    let mut elevations = Vec::with_capacity(count);
    let last = (SAMPLES - 1) as f32;
    for y in 0..SAMPLES {
        for x in 0..SAMPLES {
            let fx = x as f32 / last;
            let fy = y as f32 / last;
            // A basin wide enough to hold a lake that covers a real share of the frame. A narrow one
            // is a worse fixture than it looks: at this camera's standoff a 180-unit lake occupies
            // under two percent of the image, which leaves no room between "drew nothing" and "failed
            // to clip" for an assertion to sit in.
            let basin = 300.0 * (-((fx - 0.5).powi(2) + (fy - 0.5).powi(2)) / 0.14).exp();
            // Relief on the rim, so the shoreline is an irregular contour rather than a circle. A
            // circle would be satisfied by a broken clip that simply drew the whole rectangle.
            let undulation = 20.0 * ((fx * 5.3).sin() * (fy * 4.1).cos());
            let elevation = 420.0 - basin + undulation;
            elevations.push(elevation.round().clamp(0.0, 65_535.0) as u16);
        }
    }

    let mut ground = Vec::with_capacity(count);
    let mut rock = Vec::with_capacity(count);
    for elevation in &elevations {
        let into_rock = ramp(f32::from(*elevation), 300.0, 420.0);
        ground.push(((1.0 - into_rock) * 255.0).round() as u8);
        rock.push((into_rock * 255.0).round() as u8);
    }

    Terrain::new(
        SAMPLES,
        SAMPLES,
        SPACING,
        VERTICAL,
        elevations,
        vec![
            TerrainLayer {
                name: "ground".to_owned(),
                weights: ground,
            },
            TerrainLayer {
                name: "rock".to_owned(),
                weights: rock,
            },
        ],
    )
    .expect("valid basin terrain")
}

/// The terrain's lowest and highest elevation, in world units.
fn world_elevation_range(terrain: &Terrain) -> (f32, f32) {
    let (low, high) = terrain
        .elevations()
        .iter()
        .fold((u16::MAX, u16::MIN), |(low, high), sample| {
            (low.min(*sample), high.max(*sample))
        });
    let scale = terrain.vertical_scale();
    (f32::from(low) * scale, f32::from(high) * scale)
}

/// Waves scaled to be legible in a 720x480 capture of a 1,536-unit map.
///
/// The defaults in `water.rs` are sized for a camera at playing distance. At this fixture's standoff a
/// 0.35-unit swell is well under one pixel, so a capture could not show whether the surface animates
/// at all — which would make the animation test vacuous rather than failing.
fn visible_waves() -> WaterMaterial {
    WaterMaterial {
        wave_height: 1.6,
        wave_length: 64.0,
        ..WaterMaterial::default()
    }
}

/// A water table over the whole map at `elevation`.
fn lake(terrain: &Terrain, elevation: f32) -> WaterSurface {
    let [extent_x, extent_y] = terrain.world_extent();
    WaterSurface::new([0.0, 0.0, extent_x, extent_y], elevation).with_material(visible_waves())
}

/// Renders the chain with water and resolves the composite.
fn render_water(
    context: &GpuContext,
    harness: &Harness,
    water: &[WaterBody],
    frame: DeferredFrame,
) -> Capture {
    harness
        .deferred
        .set_frame(context, &harness.renderer, &[], water, frame)
        .expect("upload frame uniforms");
    harness.deferred.render(
        context,
        &harness.renderer,
        &[],
        water,
        &harness.targets,
        harness.output.colour_view(),
    );
    harness
        .output
        .resolve(context, encoder(context))
        .expect("resolve composite")
}

/// Pixels that differ between two captures, as a fraction of the frame.
fn fraction_differing(first: &Capture, second: &Capture) -> f32 {
    assert_eq!(first.rgba().len(), second.rgba().len(), "size mismatch");
    let differing = first
        .rgba()
        .chunks_exact(4)
        .zip(second.rgba().chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();
    differing as f32 / (first.width() * first.height()) as f32
}

/// Luminance spread over only those pixels where two captures differ, and how many there were.
///
/// The difference between a dry frame and a flooded one *is* a mask of the water, which is what makes
/// this measurable: a spread taken over the whole frame would mostly report the terrain's own range,
/// and the question here is whether the water surface itself varies.
fn masked_luminance(dry: &Capture, wet: &Capture) -> (f32, f32, usize) {
    let luminance = |pixel: &[u8]| {
        (0.2126 * f32::from(pixel[0]) + 0.7152 * f32::from(pixel[1]) + 0.0722 * f32::from(pixel[2]))
            / 255.0
    };
    let mut lowest = f32::MAX;
    let mut highest = f32::MIN;
    let mut counted = 0usize;
    for (a, b) in dry.rgba().chunks_exact(4).zip(wet.rgba().chunks_exact(4)) {
        if a == b {
            continue;
        }
        let value = luminance(b);
        lowest = lowest.min(value);
        highest = highest.max(value);
        counted += 1;
    }
    if counted == 0 {
        return (0.0, 0.0, 0);
    }
    (lowest, highest, counted)
}

/// How much luminance each changed pixel lost, as `(smallest, largest, mean)`.
///
/// Compared position by position rather than as two whole-frame ranges, which is what a first attempt at
/// the cloud test did and why it measured nothing: the darkest pixel in either frame is a corner of the
/// sky, which a cloud deck correctly does not touch, so a frame-wide minimum is identical with and
/// without clouds however much ground went into shade.
fn luminance_drop(before: &Capture, after: &Capture) -> (f32, f32, f32) {
    let luminance = |pixel: &[u8]| {
        (0.2126 * f32::from(pixel[0]) + 0.7152 * f32::from(pixel[1]) + 0.0722 * f32::from(pixel[2]))
            / 255.0
    };
    let mut smallest = f32::MAX;
    let mut largest = f32::MIN;
    let mut total = 0.0;
    let mut counted = 0usize;
    for (a, b) in before
        .rgba()
        .chunks_exact(4)
        .zip(after.rgba().chunks_exact(4))
    {
        if a == b {
            continue;
        }
        let drop = luminance(a) - luminance(b);
        smallest = smallest.min(drop);
        largest = largest.max(drop);
        total += drop;
        counted += 1;
    }
    if counted == 0 {
        return (0.0, 0.0, 0.0);
    }
    (smallest, largest, total / counted as f32)
}

/// The basin fixture, its dry capture, and a water level between floor and rim.
struct WaterScene {
    harness: Harness,
    dry: Capture,
    level: f32,
    frame: DeferredFrame,
}

fn water_scene(context: &GpuContext) -> WaterScene {
    let harness = harness_for(context, basin_terrain());
    let frame = DeferredFrame::new(pose(&harness.terrain), WIDTH, HEIGHT);
    let dry = render_water(context, &harness, &[], frame);
    let (floor, rim) = world_elevation_range(&harness.terrain);
    // Just under halfway from floor to rim. Deep enough that the tint ramp reaches its far end near the
    // middle and the lake covers a usable share of the frame, shallow enough to leave a broad margin of
    // dry ground for the shoreline clip to be visible against.
    let level = floor + (rim - floor) * 0.45;
    WaterScene {
        harness,
        dry,
        level,
        frame,
    }
}

fn water_body(context: &GpuContext, scene: &WaterScene, elevation: f32) -> Vec<WaterBody> {
    vec![
        WaterBody::new(
            context,
            lake(&scene.harness.terrain, elevation),
            scene.harness.deferred.water_layout(),
        )
        .expect("build water body"),
    ]
}

#[test]
fn water_fills_the_basin_and_leaves_the_high_ground_dry() {
    let Some(context) = context() else { return };
    let scene = water_scene(context);
    let water = water_body(context, &scene, scene.level);
    let flooded = render_water(context, &scene.harness, &water, scene.frame);

    write_capture("water-dry.png", &scene.dry);
    write_capture("water-basin.png", &flooded);
    // Pins the shoreline, the depth ramp, and the wave detail at a known scene time. The bounds below
    // would accept a lake of the right size with entirely the wrong surface on it.
    support::check_reference(context, "water-basin.png", &flooded);

    let changed = fraction_differing(&scene.dry, &flooded);
    // Both bounds matter and each catches a different fault. Too little means the surface never drew or
    // was clipped away entirely; too much means the depth comparison is not clipping it at the shore
    // and the whole rectangle was painted over the terrain.
    assert!(
        changed > 0.08,
        "water covers only {:.1}% of the frame, so it barely drew",
        changed * 100.0
    );
    assert!(
        changed < 0.75,
        "water covers {:.1}% of the frame, so the shore is not clipping it",
        changed * 100.0
    );
}

#[test]
fn a_water_table_below_the_terrain_draws_nothing() {
    // The clip test at its limit. Every fragment's bed sits above the surface, so every one has to be
    // discarded, and the frame must come back byte-identical to the dry one rather than merely similar.
    let Some(context) = context() else { return };
    let scene = water_scene(context);
    let (floor, _) = world_elevation_range(&scene.harness.terrain);
    let water = water_body(context, &scene, floor - 25.0);
    let flooded = render_water(context, &scene.harness, &water, scene.frame);
    write_capture("water-below-terrain.png", &flooded);

    assert_eq!(
        scene.dry.rgba(),
        flooded.rgba(),
        "a table below the terrain must leave the frame untouched"
    );
}

#[test]
fn water_animates_with_scene_time_and_only_with_it() {
    // What makes a reference capture of water safe. The surface has to move with the frame's time and
    // with nothing else; if it read a clock instead, the first and third captures here would differ
    // despite being taken at the same time, which is exactly what the last assertion rules out.
    let Some(context) = context() else { return };
    let scene = water_scene(context);
    let water = water_body(context, &scene, scene.level);

    let first = render_water(context, &scene.harness, &water, scene.frame.at_time(0.0));
    let later = render_water(context, &scene.harness, &water, scene.frame.at_time(3.0));
    let repeated = render_water(context, &scene.harness, &water, scene.frame.at_time(0.0));

    write_capture("water-time-0.png", &first);
    write_capture("water-time-3.png", &later);

    let moved = fraction_differing(&first, &later);
    assert!(
        moved > 0.02,
        "the surface changed over only {:.2}% of the frame between t=0 and t=3",
        moved * 100.0
    );
    assert!(
        moved < 0.75,
        "advancing time changed {:.1}% of the frame, more than the water covers",
        moved * 100.0
    );
    assert_eq!(
        first.rgba(),
        repeated.rgba(),
        "one scene time must render one frame, or no capture can serve as a reference"
    );
}

#[test]
fn the_water_surface_varies_rather_than_reading_as_a_flat_sheet() {
    // The fault this exists for is a shader that compiles, blends, and clips correctly while emitting
    // one constant colour. Every assertion above still passes on that, and the image looks like a sheet
    // of plastic laid over the basin.
    let Some(context) = context() else { return };
    let scene = water_scene(context);
    let water = water_body(context, &scene, scene.level);
    let flooded = render_water(context, &scene.harness, &water, scene.frame);

    let (lowest, highest, counted) = masked_luminance(&scene.dry, &flooded);
    assert!(counted > 1_000, "only {counted} water pixels to measure");
    let spread = highest - lowest;
    assert!(
        spread > 0.10,
        "the water spans only {spread:.3} in luminance across {counted} pixels, \
         which is a flat sheet rather than a lit surface"
    );
}

#[test]
fn cloud_shadows_dapple_the_ground_rather_than_dimming_it() {
    // The distinction this test exists for. Raising coverage must grow *patches* of shade, so the frame
    // gains contrast between lit and shaded ground; scaling a density instead would darken every pixel by
    // the same factor, which is a brightness change wearing a cloud's name and would pass any assertion
    // that only asked whether the image got darker.
    let Some(context) = context() else { return };
    let scene = water_scene(context);
    let clouded = scene.frame.in_environment(Environment {
        clouds: Clouds {
            coverage: 0.55,
            // Small enough that several patches fall inside a 1,536-unit map, or the capture shows one
            // uniform state and the test measures nothing.
            scale: 320.0,
            ..Clouds::default()
        },
        ..Environment::default()
    });
    let shaded = render_water(context, &scene.harness, &[], clouded);
    write_capture("atmosphere-clouds.png", &shaded);
    support::check_reference(context, "atmosphere-clouds.png", &shaded);

    let changed = fraction_differing(&scene.dry, &shaded);
    assert!(
        changed > 0.20,
        "clouds changed only {:.1}% of the frame",
        changed * 100.0
    );
    // Light was removed on the whole.
    let (smallest, largest, mean) = luminance_drop(&scene.dry, &shaded);
    assert!(
        mean > 0.0,
        "the deck brightened the scene: mean drop {mean:.4}"
    );
    // And it was removed *unevenly*, which is the whole claim. A deck that scaled the light instead would
    // take about the same amount from every pixel, leaving almost no spread between the least and most
    // affected — so this range is what separates a cloud shadow from a brightness slider.
    assert!(
        largest - smallest > 0.10,
        "every changed pixel lost about the same amount ({smallest:.4} to {largest:.4}), \
         so the deck dimmed the scene rather than dappling it"
    );
}

#[test]
fn fog_fills_the_basin_more_deeply_than_it_veils_the_rim() {
    // The property that separates height fog from a distance haze, and the reason the density is
    // integrated along the view ray rather than sampled at the fragment: a low basin seen from above must
    // fog *more* than the high ground beside it, even where the two are the same distance away.
    let Some(context) = context() else { return };
    let scene = water_scene(context);
    let (floor, rim) = world_elevation_range(&scene.harness.terrain);
    let foggy = scene.frame.in_environment(Environment {
        fog: Fog {
            // Deliberately modest. Fog opacity is `1 - exp(-optical)`, which saturates: at a density
            // thick enough to veil the frame, the exponential is already near its ceiling and a large
            // swing in density barely moves the result — so the banks below become invisible for a
            // reason that has nothing to do with the banks. A density leaving the factor mid-range is
            // what makes any variation in it legible.
            density: 0.0022,
            // A falloff well under the basin's depth, so the rim stands clear of what fills the floor.
            height_falloff: (rim - floor) * 0.35,
            base: floor,
            // Banked rather than uniform. The scale has to be small: the patchiness is sampled at each
            // ray's midpoint, and this camera's midpoints span only about 770 world units, so anything
            // near the map's own extent gives under one cell of variation across the frame.
            patchiness: 0.85,
            // *Large*, not small, and this inverts what a midpoint tap wanted. Marching integrates the
            // density along the ray, so a scale much smaller than the ray is long makes every ray cross
            // several banks and average them into the same value -- adjacent pixels then agree and the
            // result is the uniform wash the patchiness was added to avoid. A scale comparable to the ray
            // keeps each ray largely inside one bank, so neighbouring rays genuinely differ.
            patch_scale: 900.0,
        },
        ..Environment::default()
    });
    let veiled = render_water(context, &scene.harness, &[], foggy);
    write_capture("atmosphere-fog.png", &veiled);
    support::check_reference(context, "atmosphere-fog.png", &veiled);

    let changed = fraction_differing(&scene.dry, &veiled);
    assert!(
        changed > 0.30,
        "fog changed only {:.1}% of the frame",
        changed * 100.0
    );
    // Fog flattens: it pulls everything toward one colour, so the spread has to shrink.
    let (low_before, high_before) = scene.dry.luminance_range();
    let (low_after, high_after) = veiled.luminance_range();
    assert!(
        (high_after - low_after) < (high_before - low_before),
        "fog did not reduce the luminance spread: {:.3} against {:.3}",
        high_after - low_after,
        high_before - low_before
    );
}

#[test]
fn a_water_body_survives_the_chain_being_rebuilt() {
    // What a window resize does. `SurfaceRenderer::resize` reallocates every target and builds a fresh
    // `DeferredRenderer`, which creates a *new* water bind group layout — while the bodies the caller
    // is holding were bound against the old one. That is only safe if layouts are compatible
    // structurally rather than by identity, and the same exposure applies to every model batch, so it
    // is worth pinning rather than assuming. No window is needed to find out.
    let Some(context) = context() else { return };
    let terrain = basin_terrain();
    let renderer =
        TerrainRenderer::new(context, &terrain, &palette()).expect("build terrain renderer");
    let first_targets = DeferredTargets::new(context, WIDTH, HEIGHT).expect("first targets");
    let first = DeferredRenderer::new(
        context,
        &renderer,
        &first_targets,
        cic_render::gpu::CAPTURE_FORMAT,
    )
    .expect("first deferred renderer");

    let (floor, rim) = world_elevation_range(&terrain);
    let water = vec![
        WaterBody::new(
            context,
            lake(&terrain, floor + (rim - floor) * 0.45),
            first.water_layout(),
        )
        .expect("build water body"),
    ];

    // The resize: new targets at a new size, and a renderer rebuilt against them.
    let (wide, tall) = (WIDTH + 96, HEIGHT + 64);
    let second_targets = DeferredTargets::new(context, wide, tall).expect("second targets");
    let second = DeferredRenderer::new(
        context,
        &renderer,
        &second_targets,
        cic_render::gpu::CAPTURE_FORMAT,
    )
    .expect("second deferred renderer");
    let output = CaptureTarget::new(context, wide, tall).expect("resized output");

    let frame = DeferredFrame::new(pose(&terrain), wide, tall);
    second
        .set_frame(context, &renderer, &[], &water, frame)
        .expect("upload frame uniforms");
    second.render(
        context,
        &renderer,
        &[],
        &water,
        &second_targets,
        output.colour_view(),
    );

    // Validation errors surface asynchronously, so drain the queue before declaring success.
    context
        .device()
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(10)),
        })
        .expect("a body bound against the old layout must still draw after a rebuild");
}

#[test]
fn a_sun_in_the_mirror_direction_puts_glitter_on_the_water() {
    // Verifies the specular term reaches the *frame*, not merely that it exists in the shader. The
    // gloss exponent was deliberately lowered from a mirror-like value so glitter would be visible at
    // realistic wave slopes, and without an assertion that looks for the highlight there is no
    // difference between that having worked and the term being dead code.
    let Some(context) = context() else { return };
    let scene = water_scene(context);
    let water = water_body(context, &scene, scene.level);

    // The camera sits south-east and above, so a surface normal of +Z bisects view and light exactly
    // when the sun is placed north-west at the mirrored elevation. `light.direction` points from the
    // surface *toward* the light, so this is the view direction with its horizontal part negated.
    let mut glinting_frame = scene.frame;
    glinting_frame.light.direction = [-0.45, 0.77, 0.43];

    let dry_glinting = render_water(context, &scene.harness, &[], glinting_frame);
    let glinting = render_water(context, &scene.harness, &water, glinting_frame);
    write_capture("water-glitter.png", &glinting);
    support::check_reference(context, "water-glitter.png", &glinting);

    let (_, default_peak, _) = masked_luminance(&scene.dry, &{
        render_water(context, &scene.harness, &water, scene.frame)
    });
    let (_, glinting_peak, counted) = masked_luminance(&dry_glinting, &glinting);

    assert!(counted > 1_000, "only {counted} water pixels to measure");
    assert!(
        glinting_peak > default_peak + 0.15,
        "the mirrored sun peaked at {glinting_peak:.3} against {default_peak:.3} under the default \
         one, so the highlight is not reaching the frame"
    );
}
