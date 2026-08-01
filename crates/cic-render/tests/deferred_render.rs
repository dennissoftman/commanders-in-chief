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
use std::time::Duration;

use cic_assets::sky::{SkyAsset, SkyLimits, decode_radiance};
use cic_assets::{Terrain, TerrainLayer};
use cic_camera::CameraPose;
use cic_render::detail::TerrainDetailRequest;
use cic_render::display::JITTER_PHASES;
use cic_render::sky::{Sky, SkySettings};
use cic_render::terrain::LayerColour;
use cic_render::terrain_virtual::VirtualPageView;
use cic_render::{
    Antialiasing, Capture, CaptureTarget, Clouds, DeferredFrame, DeferredRenderer, DeferredTargets,
    DisplaySettings, Environment, Fog, FrameTimings, GpuContext, LayerMaterial, ReflectionProvider,
    TerrainRenderer, TextureImage, TimedPass, WaterBody, WaterKind, WaterMaterial, WaterSurface,
    Weather,
};

const WIDTH: u32 = 720;
const HEIGHT: u32 = 480;
const SAMPLES: u32 = 193;
const SPACING: f32 = 8.0;
const VERTICAL: f32 = 0.5;

static CONTEXT: OnceLock<Option<GpuContext>> = OnceLock::new();

fn context() -> Option<&'static GpuContext> {
    CONTEXT.get_or_init(support::shared_context).as_ref()
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

/// A camera with the horizon in frame, for the tests about what is behind the scene.
///
/// [`pose`] is pitched about twenty-six degrees down and the projection's half angle is a little under
/// that, so its topmost row sits within a degree of horizontal — every background pixel it captures is
/// *below* the horizon. That is a perfectly good camera for shadows and a useless one for a sky, and
/// the distinction took a capture to see: the first run of these tests rendered the environment's lower
/// hemisphere across the whole background and read as a flat grey lid, which looks exactly like a
/// texture that failed to bind.
fn sky_pose(terrain: &Terrain) -> CameraPose {
    let [extent_x, extent_y] = terrain.world_extent();
    let focus = [extent_x * 0.45, extent_y * 0.55, 240.0];
    let eye = [
        focus[0] + extent_x * 0.30,
        focus[1] - extent_y * 0.95,
        extent_x * 0.22,
    ];
    CameraPose {
        eye,
        focus,
        // A shallow pitch, so the upper third of the frame is genuinely above the horizon.
        forward: [-0.30, 0.95, -0.14],
    }
}

/// A camera close enough to the ground to run *out* of scene before it runs out of shadow distance.
///
/// The other half of the cascade fit from [`pose`], which is bounded at its near end. This one stands a
/// little way above the low ground at the near corner — roughly 229 units there, from the base 200 plus the
/// undulation and the tail of the spire — and looks steeply down, so its frustum drops through the terrain's
/// floor around 430 units out and the whole 1,600-unit shadow distance past that has nothing left to shade.
/// It existed first as the control for a near cascade with casters in it, back when [`pose`] had none.
fn ground_level_pose(terrain: &Terrain) -> CameraPose {
    let [extent_x, extent_y] = terrain.world_extent();
    let eye = [extent_x * 0.25, extent_y * 0.25, 300.0];
    let focus = [eye[0] + 25.0, eye[1] + 25.0, 229.0];
    CameraPose {
        eye,
        focus,
        // The eye-to-focus delta itself. `pose` states its forward the same way, unnormalised: what the
        // camera uses is the direction, and dividing by the length here would only hide the arithmetic.
        forward: [focus[0] - eye[0], focus[1] - eye[1], focus[2] - eye[2]],
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
    harness_with(context, terrain, DisplaySettings::NATIVE)
}

fn harness_with(context: &GpuContext, terrain: Terrain, display: DisplaySettings) -> Harness {
    let materials: Vec<LayerMaterial> =
        palette().iter().copied().map(LayerMaterial::from).collect();
    harness_from(context, terrain, display, &materials)
}

fn harness_from(
    context: &GpuContext,
    terrain: Terrain,
    display: DisplaySettings,
    materials: &[LayerMaterial],
) -> Harness {
    let renderer = TerrainRenderer::with_materials(context, &terrain, materials)
        .expect("build terrain renderer");
    let targets = DeferredTargets::new(
        context,
        WIDTH,
        HEIGHT,
        cic_render::gpu::CAPTURE_FORMAT,
        display,
    )
    .expect("allocate targets");
    let deferred =
        DeferredRenderer::new(context, &renderer, &targets).expect("build deferred renderer");
    // The capture is always the *output* size. That is the whole point of a resolution scale: what
    // changes is the sampling rate, not the size of the image the caller receives.
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
        None,
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

    // The estimate is half resolution and its resolve is full. Pinned structurally as well as by the
    // reference image, because reverting it costs 42% of the frame and would otherwise show up only as a
    // handful of pixels differing on a capture — a change nobody would read as a performance regression.
    assert_eq!(harness.targets.render_size(), (WIDTH, HEIGHT));
    assert_eq!(harness.targets.occlusion_size(), (WIDTH / 2, HEIGHT / 2));
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
        None,
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
    let flat_targets = DeferredTargets::new(
        context,
        WIDTH,
        HEIGHT,
        cic_render::gpu::CAPTURE_FORMAT,
        DisplaySettings::NATIVE,
    )
    .expect("flat targets");
    let flat_deferred =
        DeferredRenderer::new(context, &flat_renderer, &flat_targets).expect("flat deferred");
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
        None,
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
    let format = wgpu::TextureFormat::Bgra8UnormSrgb;
    // Antialiasing on as well, so the BGRA plumbing is exercised on both passes that write the output
    // format *and* on the intermediate the composite writes when a pass follows it. That intermediate is
    // allocated in the output format precisely so one composite pipeline serves both cases, which makes
    // it the thing most likely to be wrong here.
    let targets = DeferredTargets::new(
        context,
        WIDTH,
        HEIGHT,
        format,
        DisplaySettings::NATIVE.with_antialiasing(Antialiasing::Fxaa),
    )
    .expect("allocate targets");
    let deferred = DeferredRenderer::new(context, &renderer, &targets)
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
    deferred.render(context, &renderer, &[], &[], None, &targets, &view);

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
        None,
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

/// A camera over the near shore, pitched down at the water the way a playing one is.
///
/// [`pose`] stands far enough back that the whole lake is a few hundred pixels of near-uniform
/// surface, which is the right camera for asking whether water drew at all and the wrong one for
/// asking what kind of water it is. The reason is Fresnel and it took a capture to see: at ten degrees
/// above the surface the reflected share is over two thirds everywhere, so all three kinds render as
/// the same sky and their tints — the thing that most separates them — contribute almost nothing. This
/// one looks down at about forty-five degrees at the near waterline, where the reflected share is
/// two percent and the body colour is the whole of what is seen, and flattens out to a grazing view by
/// the far shore. Both regimes are in one frame, which is also what makes it the right capture for
/// judging the wave train: the near water resolves individual waves and the far water must not.
fn shore_pose(terrain: &Terrain, level: f32) -> CameraPose {
    let [extent_x, extent_y] = terrain.world_extent();
    let focus = [extent_x * 0.5, extent_y * 0.40, level];
    let eye = [extent_x * 0.5, extent_y * 0.195, level + 120.0];
    CameraPose {
        eye,
        focus,
        forward: [focus[0] - eye[0], focus[1] - eye[1], focus[2] - eye[2]],
    }
}

/// The basin fixture seen from its own shore, and a level low enough to leave a broad shallow margin.
///
/// Shallower than [`water_scene`]'s. The tint ramp and the foam band are both keyed on depth, so a
/// level that puts the shoreline against a steep part of the bowl compresses both into a few pixels
/// and the three kinds then differ only in their deep-water colour.
struct ShoreScene {
    harness: Harness,
    level: f32,
    frame: DeferredFrame,
}

fn shore_scene(context: &GpuContext) -> ShoreScene {
    let harness = harness_for(context, basin_terrain());
    let (floor, rim) = world_elevation_range(&harness.terrain);
    let level = floor + (rim - floor) * 0.30;
    let frame = DeferredFrame::new(shore_pose(&harness.terrain, level), WIDTH, HEIGHT);
    ShoreScene {
        harness,
        level,
        frame,
    }
}

/// One body of `material` over the whole map at the scene's level.
fn shore_body(context: &GpuContext, scene: &ShoreScene, material: WaterMaterial) -> Vec<WaterBody> {
    let [extent_x, extent_y] = scene.harness.terrain.world_extent();
    vec![
        WaterBody::new(
            context,
            WaterSurface::new([0.0, 0.0, extent_x, extent_y], scene.level).with_material(material),
            scene.harness.deferred.water_layout(),
        )
        .expect("build water body"),
    ]
}

/// Mean per-channel difference between two captures over the pixels a predicate accepts, in `0..=255`.
///
/// The predicate takes a pixel index rather than a coordinate pair, because every caller here is
/// intersecting two masks that are already in that form.
fn mean_difference_where(first: &Capture, second: &Capture, accept: impl Fn(usize) -> bool) -> f32 {
    assert_eq!(first.rgba().len(), second.rgba().len(), "size mismatch");
    let mut total = 0.0f64;
    let mut counted = 0usize;
    for pixel in 0..first.rgba().len() / 4 {
        if !accept(pixel) {
            continue;
        }
        for channel in 0..3 {
            let a = f64::from(first.rgba()[pixel * 4 + channel]);
            let b = f64::from(second.rgba()[pixel * 4 + channel]);
            total += (a - b).abs();
        }
        counted += 3;
    }
    if counted == 0 {
        return 0.0;
    }
    (total / counted as f64) as f32
}

/// A mask of where a body of water drew, taken as the pixels a flood changed.
fn wet_mask(dry: &Capture, wet: &Capture) -> Vec<bool> {
    dry.rgba()
        .chunks_exact(4)
        .zip(wet.rgba().chunks_exact(4))
        .map(|(a, b)| a != b)
        .collect()
}

#[test]
fn the_three_kinds_of_water_are_told_apart_by_their_own_captures() {
    // The images are the verification here more than anywhere else in this file. "A river looks like a
    // river" is not a number, and the assertions below only bound the three properties that a careless
    // retune could erase silently — the tint, the surf, and the current. Everything else about whether
    // these read correctly is in the three captures.
    //
    // All three bodies span the whole map, which is what a lake and an ocean are and what a river is
    // not: a real channel is a chain of narrow reaches, each carrying its own tangent as its flow. The
    // fixture is showing the *material* rather than the channel, because a strip laid across this basin
    // would be cut off by its own rectangle along both banks and the hard edge would be the most
    // visible thing in the frame.
    let Some(context) = context() else { return };
    let scene = shore_scene(context);
    let dry = render_water(context, &scene.harness, &[], scene.frame);
    write_capture("water-kind-dry.png", &dry);

    let kinds = [
        ("lake", WaterKind::Lake),
        ("river", WaterKind::River { flow: [7.0, 2.0] }),
        ("ocean", WaterKind::Ocean { wind: [1.0, 0.35] }),
    ];
    let mut captures = Vec::new();
    for (name, kind) in kinds {
        let body = shore_body(context, &scene, kind.material());
        let capture = render_water(context, &scene.harness, &body, scene.frame);
        let file = format!("water-kind-{name}.png");
        write_capture(&file, &capture);
        captures.push((name, file, capture));
    }
    // All three at once rather than one call each, so a run against an adapter with no reference set
    // writes all three and a change that moved all three reports all three. See `check_references`.
    support::check_references(
        context,
        &captures
            .iter()
            .map(|(_, file, capture)| (file.as_str(), capture))
            .collect::<Vec<_>>(),
    );

    // Compared pixel by pixel rather than as three mean colours, which is what a first attempt did and
    // why it measured almost nothing: an ocean runs from turquoise over the bank to near-black in the
    // middle and averages to very nearly the flat mid-blue of a lake, so the two came out 1.9 apart in
    // 255 while looking nothing whatever alike. What separates the kinds is *where* the colour is, and
    // a per-pixel difference is the statistic that says so.
    let masks: Vec<Vec<bool>> = captures
        .iter()
        .map(|(_, _, capture)| wet_mask(&dry, capture))
        .collect();
    for (index, (name, _, capture)) in captures.iter().enumerate() {
        let other = (index + 1) % captures.len();
        let difference = mean_difference_where(capture, &captures[other].2, |pixel| {
            masks[index][pixel] && masks[other][pixel]
        });
        assert!(
            difference > 12.0,
            "{name} and {} differ by {difference:.1} of 255 per channel over the water they share, \
             which is too little to be two kinds of water rather than one",
            captures[other].0
        );
    }

    // The depth ramp, which is the other half of why the means were equal. An ocean's own water has to
    // span a wide range — bright shallows against a near-black middle — where a lake's is nearly one
    // colour throughout. This is the assertion the mean could not make.
    let range = |capture: &Capture, mask: &[bool]| {
        let (mut lowest, mut highest) = (f32::MAX, f32::MIN);
        for (index, wet) in mask.iter().enumerate() {
            if !*wet {
                continue;
            }
            let pixel = &capture.rgba()[index * 4..index * 4 + 3];
            let value = 0.2126 * f32::from(pixel[0])
                + 0.7152 * f32::from(pixel[1])
                + 0.0722 * f32::from(pixel[2]);
            lowest = lowest.min(value);
            highest = highest.max(value);
        }
        highest - lowest
    };
    let ocean_range = range(&captures[2].2, &masks[2]);
    let lake_range = range(&captures[0].2, &masks[0]);
    assert!(
        ocean_range > lake_range * 1.5,
        "the ocean spans {ocean_range:.1} in luminance against the lake's {lake_range:.1}, \
         so its depth ramp is not carrying shallows against deep water"
    );

    // Surf. Counted as water pixels with every channel above 190, which is a level none of these
    // tints reaches on its own however it is lit — the lake, whose foam is a tenth strength, has no
    // pixel at all there. That is what foam is for: a band reading as white against a body that never
    // does.
    let frothy = |capture: &Capture, mask: &[bool]| {
        mask.iter()
            .enumerate()
            .filter(|(_, wet)| **wet)
            .filter(|(index, _)| {
                capture.rgba()[index * 4..index * 4 + 3]
                    .iter()
                    .all(|channel| *channel > 190)
            })
            .count()
    };
    let (lake_foam, ocean_foam) = (
        frothy(&captures[0].2, &masks[0]),
        frothy(&captures[2].2, &masks[2]),
    );
    assert!(
        ocean_foam > 400 && ocean_foam > lake_foam * 4,
        "the ocean shows {ocean_foam} foam pixels against the lake's {lake_foam}, \
         so the surf that is most of what reads as an ocean is not there"
    );
}

/// A lake with a steep ridge on its far shore and a banded bed under it.
///
/// Built for the two things `basin_terrain` structurally cannot show, and both absences were found by
/// measuring rather than by looking.
///
/// **Something above the waterline to reflect.** A screen-space reflection can only return what is on
/// screen, and a bowl has nothing standing above its own water: every reflected ray leaves the frame
/// and the provider correctly falls back to the sky, so the whole feature renders as a no-op. The
/// ridge here rises two hundred units out of the far shore and is what a low camera across the water
/// sees reflected in it.
///
/// **A bed with contrast in it.** Refraction displaces what is seen *through* the surface, and the
/// basin's bed is a smooth pale wash — so displacing it by twenty world units changed 0.18% of pixels
/// by at most 4/255, which is indistinguishable from the term being disconnected. It took forcing the
/// sample to a constant colour to establish that the plumbing worked at all. The bed here is banded
/// between the two terrain layers, so there are edges under the water for a wave to bend.
fn reflecting_terrain() -> Terrain {
    let count = (SAMPLES * SAMPLES) as usize;
    let mut elevations = Vec::with_capacity(count);
    let last = (SAMPLES - 1) as f32;
    for y in 0..SAMPLES {
        for x in 0..SAMPLES {
            let fx = x as f32 / last;
            let fy = y as f32 / last;
            // A trough across the near half, and a ridge across the far side. Both run along x, so the
            // shoreline is broadly straight and a camera looking down y sees water the whole way.
            let trough = 150.0 * (-((fy - 0.34) / 0.20).powi(2)).exp();
            let ridge = 260.0 * (-((fy - 0.82) / 0.09).powi(2)).exp();
            // Relief along the shore, so the waterline is a contour rather than a ruled line and the
            // reflection has something with shape in it to be a reflection *of*.
            let relief = 22.0 * (fx * 7.1).sin() * (fy * 3.3).cos();
            // Bed ripples, at the shortest wavelength this grid can carry. Sample spacing is eight
            // world units, so anything under sixteen aliases; these run about forty. They are here as
            // *shading* contrast rather than colour contrast — the lighting pass turns a slope into a
            // light or dark band whatever the palette is, where two layer colours this close together
            // wash out to the same pale under a strong sun. Without something like this under the
            // water there is nothing for a refracting wave to visibly displace.
            let ripples = 5.0 * (fx * 240.0).sin() * (fy * 90.0).cos();
            let elevation = 330.0 - trough + ridge + relief + ripples;
            elevations.push(elevation.round().clamp(0.0, 65_535.0) as u16);
        }
    }

    let mut ground = Vec::with_capacity(count);
    let mut rock = Vec::with_capacity(count);
    for (index, elevation) in elevations.iter().enumerate() {
        let fx = (index % SAMPLES as usize) as f32 / last;
        let fy = (index / SAMPLES as usize) as f32 / last;
        // Bands rather than an elevation ramp. An elevation split puts the layer boundary along a
        // contour, which under water runs parallel to the shore and gives a refracting wave one edge
        // to move; bands crossing the lake give it a dozen. High ground is rock regardless, so the
        // ridge still reads as rock.
        let banded = ramp(((fx * 9.0 + fy * 3.0).sin()).mul_add(0.5, 0.5), 0.35, 0.65);
        let high = ramp(f32::from(*elevation), 420.0, 520.0);
        let into_rock = banded.mul_add(1.0 - high, high);
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
    .expect("valid reflecting terrain")
}

/// Low over the near shore, looking across the water at the ridge.
///
/// Deliberately the most grazing camera in this file, at about eight degrees above the surface, and
/// that is the whole point of it: the reflected share goes as the fifth power of the incidence, so at
/// this angle it is over two thirds and the reflection is most of what the water shows. The
/// forty-five-degree shore camera is the opposite case and is where the *body* is everything — between
/// them they cover both regimes water has.
fn reflecting_pose(terrain: &Terrain, level: f32) -> CameraPose {
    let [extent_x, extent_y] = terrain.world_extent();
    // The eye sits over the near bank, not over the water: the trough only holds water between about
    // 0.21 and 0.47 of the map, and the first version of this pose put the camera at 0.06 where the
    // ground stands twenty units *above* the surface -- so it rendered the inside of a hill.
    let focus = [extent_x * 0.5, extent_y * 0.80, level + 80.0];
    let eye = [extent_x * 0.5, extent_y * 0.15, level + 45.0];
    CameraPose {
        eye,
        focus,
        forward: [focus[0] - eye[0], focus[1] - eye[1], focus[2] - eye[2]],
    }
}

/// Down at the shallows of the reflecting fixture, steeply enough to see the bed through them.
///
/// The opposite camera to [`reflecting_pose`] over the same ground, and both are needed because water
/// shows two different things at the two angles. At eight degrees the reflected share is over two
/// thirds and the body contributes almost nothing, which is where a reflection is measurable and where
/// refraction is not: the surface is nearly opaque there and there is no bed in the frame to bend.
/// At forty degrees the reflected share is a few percent and everything seen is what the water
/// transmits — so this is where refraction lives, and it is why the first attempt at measuring it
/// through the grazing pose reported a hundredth of a level out of 255.
fn refracting_pose(terrain: &Terrain, level: f32) -> CameraPose {
    let [extent_x, extent_y] = terrain.world_extent();
    // Aimed at the *shoreline*, not the middle. The trough is thirty-six units deep at its centre and
    // absorption closes over the bed within about ten, so a camera pointed at the centre of the lake
    // sees opaque water and nothing to refract -- which is what the first version of this pose did.
    let focus = [extent_x * 0.5, extent_y * 0.225, level];
    let eye = [extent_x * 0.5, extent_y * 0.09, level + 170.0];
    CameraPose {
        eye,
        focus,
        forward: [focus[0] - eye[0], focus[1] - eye[1], focus[2] - eye[2]],
    }
}

/// A flood plain with one channel cut across it, meandering as it goes.
///
/// A river needs its own fixture and cannot borrow the basin. A body laid over a bowl clips to a disc
/// however its material is set, and a disc of moving water is a lake with a grain in it; the thing
/// that makes a river read as a river is that it is *narrow and continuous*, which is a property of
/// the bed rather than of the water. Laying a map-sized body over this one clips it to a ribbon, which
/// is exactly what the shoreline clip is for.
///
/// The channel meanders, so one body's single flow vector is tangent to it only near the middle. That
/// is honest about the shape the renderer takes: a channel that bends is a chain of reaches, each
/// carrying its own tangent, and this fixture is one of them stretched over a whole map.
fn channel_terrain() -> Terrain {
    let count = (SAMPLES * SAMPLES) as usize;
    let mut elevations = Vec::with_capacity(count);
    let last = (SAMPLES - 1) as f32;
    for y in 0..SAMPLES {
        for x in 0..SAMPLES {
            let fx = x as f32 / last;
            let fy = y as f32 / last;
            // The centreline, and a trench that reaches full depth at it and nothing at the bank.
            // Smoothstepped rather than linear so the banks meet the plain without a crease, which a
            // grazing view down a channel would otherwise pick out as a hard line either side.
            let centre = 0.5 + 0.10 * (fx * 6.0).sin();
            let across = ((fy - centre).abs() / 0.055).min(1.0);
            // Twenty world units deep across eighty-four of bank, which is a river rather than a
            // gorge. The first attempt cut it four times deeper and produced a slot the water sat at
            // the bottom of: the banks then rise a whole camera height either side, the shallow
            // margin the foam and the tint ramp both live in is two units wide, and the only thing
            // the capture shows is that something green is down there.
            let trench = 40.0 * (1.0 - ramp(across, 0.0, 1.0));
            // Relief on the plain, which makes the channel widen and narrow along its length as the
            // ground either side of it rises and falls through the water level.
            let plain = 300.0 + 18.0 * ((fx * 3.1).sin() * (fy * 2.7).cos());
            elevations.push((plain - trench).round().clamp(0.0, 65_535.0) as u16);
        }
    }

    let mut ground = Vec::with_capacity(count);
    let mut rock = Vec::with_capacity(count);
    for elevation in &elevations {
        // Split across the banks rather than at the plain, so the cut reads as pale gravel and the
        // flood plain as green. Straddling the plain's own elevation instead would dapple the whole
        // map with the relief and leave the channel indistinguishable from the ground it is cut into
        // — and putting the pale layer on the plain, which the first version did, blows the entire
        // frame out to near-white under this sun and leaves nothing for the water to read against.
        let into_rock = 1.0 - ramp(f32::from(*elevation), 220.0, 290.0);
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
    .expect("valid channel terrain")
}

/// Down at the channel from over the plain beside it, along a stretch of its length.
///
/// The eye sits over the plain rather than over the cut, or the near bank fills the lower half of the
/// frame, and it looks along the channel rather than across it so several bends are in shot — a
/// straight-on view of a river fifty units wide is a green stripe and says nothing about a current
/// that runs down it. The focus is on the centreline at the far end of the view rather than on the
/// middle row of the map, because the channel meanders by a tenth of the map and the middle row is
/// bank half the time.
///
/// The pitch is thirty degrees and that figure is load-bearing. A river's chop is eleven units long,
/// and a pixel's footprint along the view runs as the reciprocal of the sine of the pitch: at the
/// twenty-two degrees the first version of this pose used, one pixel spanned nearly three world units
/// and the normal level-of-detail correctly flattened six of the train's nine components, so the
/// capture showed a smooth green ribbon and no amount of steepening the waves changed it. At thirty
/// the footprint is under a unit and a half and most of the train survives. That is not a workaround:
/// a river seen from far enough away at a shallow enough angle *is* smooth, and this camera is where
/// a player looks at one.
fn channel_pose(terrain: &Terrain, level: f32) -> CameraPose {
    let [extent_x, extent_y] = terrain.world_extent();
    let focus = [extent_x * 0.58, extent_y * 0.466, level];
    let eye = [extent_x * 0.38, extent_y * 0.36, level + 200.0];
    CameraPose {
        eye,
        focus,
        forward: [focus[0] - eye[0], focus[1] - eye[1], focus[2] - eye[2]],
    }
}

/// The reflecting fixture, its water level, and a frame looking across it at the ridge.
struct ReflectingScene {
    harness: Harness,
    level: f32,
    frame: DeferredFrame,
    water: Vec<WaterBody>,
}

fn reflecting_scene(context: &GpuContext) -> ReflectingScene {
    let harness = harness_for(context, reflecting_terrain());
    let (floor, rim) = world_elevation_range(&harness.terrain);
    // Low in the trough, so the lake is a broad shallow sheet rather than a deep pool: the shallows
    // are where a bed shows through and where refraction is visible at all.
    let level = floor + (rim - floor) * 0.16;
    let frame = DeferredFrame::new(reflecting_pose(&harness.terrain, level), WIDTH, HEIGHT);
    let [extent_x, extent_y] = harness.terrain.world_extent();
    let water = vec![
        WaterBody::new(
            context,
            WaterSurface::of_kind([0.0, 0.0, extent_x, extent_y], level, WaterKind::Lake),
            harness.deferred.water_layout(),
        )
        .expect("build lake"),
    ];
    ReflectingScene {
        harness,
        level,
        frame,
        water,
    }
}

#[test]
fn the_screen_space_provider_reflects_the_scene_where_the_sky_provider_cannot() {
    // The assertion the whole provider mechanism exists to make, and it is differential in the one way
    // that matters: the *same* scene, the same camera and the same water, rendered through both
    // providers. Anything that differs is the reflection, because nothing else changed.
    //
    // It needs saying why this fixture and not one of the others. Measured through the shore camera the
    // two providers differ over 0.2% of pixels by at most 15/255, and through the basin fixture at a
    // grazing angle they do not differ at all -- the first because a surface seen from above reflects
    // two percent of anything, the second because a bowl has nothing standing above its own water for a
    // screen-space march to find. Neither is a fair test of a reflection.
    let Some(context) = context() else { return };
    let mut scene = reflecting_scene(context);

    let sky = render_water(context, &scene.harness, &scene.water, scene.frame);
    scene.harness.deferred.set_reflection(
        context,
        ReflectionProvider::ScreenSpace,
        &scene.harness.targets,
    );
    let screen = render_water(context, &scene.harness, &scene.water, scene.frame);
    write_capture("water-reflect-sky.png", &sky);
    write_capture("water-reflect-screen.png", &screen);
    support::check_references(
        context,
        &[
            ("water-reflect-sky.png", &sky),
            ("water-reflect-screen.png", &screen),
        ],
    );

    // Where the water is: the pixels a flood changes, taken against the sky provider so the mask is the
    // lake rather than the lake plus whatever the reflection moved.
    let dry = {
        scene.harness.deferred.set_reflection(
            context,
            ReflectionProvider::Sky,
            &scene.harness.targets,
        );
        render_water(context, &scene.harness, &[], scene.frame)
    };
    let wet = wet_mask(&dry, &sky);
    let covered = wet.iter().filter(|w| **w).count();
    assert!(
        covered > (WIDTH * HEIGHT) as usize / 20,
        "the lake covers {covered} pixels, too few to measure a reflection in"
    );

    // Substantial and confined to the water. A provider that reflected the scene *everywhere* would be
    // a bug in the pipeline rather than a reflection, and the second bound is what says so.
    let on_water = mean_difference_where(&sky, &screen, |pixel| wet[pixel]);
    let off_water = mean_difference_where(&sky, &screen, |pixel| !wet[pixel]);
    assert!(
        on_water > 4.0,
        "the two providers differ by only {on_water:.2} of 255 over the water, so the screen-space \
         march is finding nothing the sky did not already answer"
    );
    assert!(
        off_water < 0.5,
        "the providers differ by {off_water:.2} of 255 away from the water, so something other than \
         the water pass changed"
    );
}

#[test]
fn a_wave_bends_what_is_seen_through_it() {
    // Refraction, stated against the same material with the term zeroed -- which is the only way to
    // measure it, since everything else about the two frames is identical.
    //
    // This fixture rather than an earlier one because refraction displaces the *bed*, and a bed with no
    // contrast in it displaces to the same colour. The basin's is a smooth pale wash: moving it twenty
    // world units changed 0.18% of pixels by at most 4/255, which is what a disconnected term looks
    // like, and it took forcing the sample to a constant to prove otherwise. This bed is banded.
    let Some(context) = context() else { return };
    let scene = reflecting_scene(context);
    let [extent_x, extent_y] = scene.harness.terrain.world_extent();
    // Its own camera, looking down into the shallows rather than across the surface.
    let frame = DeferredFrame::new(
        refracting_pose(&scene.harness.terrain, scene.level),
        WIDTH,
        HEIGHT,
    );

    // Exaggerated well past the preset, and that is the point of the test rather than a cheat.
    //
    // Refraction at this engine's scale is a *sub-unit* effect: the physical lateral shift is about a
    // quarter of the surface slope times the depth, and a lake's slope is under four degrees, so at
    // five units down the preset displaces the bed by about half a world unit. The bed it is
    // displacing is a heightfield sampled every eight units, which cannot carry detail finer than
    // sixteen — so half a unit of shift lands inside one bed feature and changes almost nothing.
    //
    // That is a fact about the fixture, not about the term. A real map's bed carries tiled albedo with
    // detail far below the heightfield's, which is where the preset shows. What this test can
    // establish is that the mechanism is connected and displaces in proportion to the figure it is
    // given — which is exactly what could not be established by staring at captures, and what took
    // forcing the sample to a constant colour to find out last time.
    let bending = WaterMaterial {
        refraction: 30.0,
        ..WaterKind::Lake.material()
    };
    let flat = WaterMaterial {
        refraction: 0.0,
        ..bending
    };
    let body = |material: WaterMaterial| {
        vec![
            WaterBody::new(
                context,
                WaterSurface::new([0.0, 0.0, extent_x, extent_y], scene.level)
                    .with_material(material),
                scene.harness.deferred.water_layout(),
            )
            .expect("build lake"),
        ]
    };

    let bent = render_water(context, &scene.harness, &body(bending), frame);
    let straight = render_water(context, &scene.harness, &body(flat), frame);
    write_capture("water-refraction.png", &bent);
    support::check_reference(context, "water-refraction.png", &bent);

    let dry = render_water(context, &scene.harness, &[], frame);
    let wet = wet_mask(&dry, &straight);
    let moved = mean_difference_where(&bent, &straight, |pixel| wet[pixel]);
    // A quarter of a level, against a measured 0.52. That looks like a low bar and is not: the same
    // measurement with the term disconnected reads 0.02, so the gap between working and not is
    // twenty-five fold and this sits in the middle of it. It is a mean over *all* the water, most of
    // which is deep enough to be opaque and can therefore show no bed at any displacement — so the
    // figure is diluted by construction rather than small at the pixels that carry it.
    assert!(
        moved > 0.25,
        "bending the transmitted view moved {moved:.2} of 255 over the water, against 0.52 when \
         it was last measured working and 0.02 when it was disconnected — so the bed is being \
         read at the same place whatever the wave face is doing"
    );
}

#[test]
fn a_river_reads_as_a_channel_with_a_current_in_it() {
    // The capture is the verification, and it is the one this whole change is for: a ribbon of silty
    // water in a cut, streaked along its own length, with the bed showing through the shallows and the
    // water piling pale against both banks.
    //
    // The assertion under it is about the current, which is the one part of a river nothing else in
    // this file would notice going missing. A `current` that was never packed into the uniform leaves
    // a body whose waves still travel, still animate, and still satisfy every other test here, while
    // the water itself stands still — so it is measured against the same material with the current
    // zeroed, which holds the wave speed the two share out of the comparison.
    let Some(context) = context() else { return };
    let harness = harness_for(context, channel_terrain());
    let (floor, rim) = world_elevation_range(&harness.terrain);
    let level = floor + (rim - floor) * 0.35;
    let frame = DeferredFrame::new(channel_pose(&harness.terrain, level), WIDTH, HEIGHT);
    let [extent_x, extent_y] = harness.terrain.world_extent();

    // Downstream is +x, which is the axis the channel runs along.
    let flowing = WaterKind::River { flow: [9.0, 0.0] }.material();
    let bodies = |material: WaterMaterial| {
        vec![
            WaterBody::new(
                context,
                WaterSurface::new([0.0, 0.0, extent_x, extent_y], level).with_material(material),
                harness.deferred.water_layout(),
            )
            .expect("build river body"),
        ]
    };

    let river = bodies(flowing);
    let capture = render_water(context, &harness, &river, frame);
    write_capture("water-river-channel.png", &capture);
    support::check_reference(context, "water-river-channel.png", &capture);

    let dry = render_water(context, &harness, &[], frame);
    let wet = wet_mask(&dry, &capture);
    let covered = wet.iter().filter(|wet| **wet).count();
    assert!(
        covered > (WIDTH * HEIGHT) as usize / 60,
        "the channel holds {covered} pixels of water, which is too few to see a river in"
    );

    // Measured as how far the water *moved*, in mean channels of 255, rather than as how many pixels
    // changed at all. Counting pixels was the first attempt and it saturated: over a third of a second
    // a still surface's own waves already alter very nearly every water pixel by something, so the two
    // came out 4.96% against 5.79% and the comparison was reporting the size of the river rather than
    // the speed of anything. The step is short for the same reason — over a long one both surfaces
    // decorrelate completely and any statistic saturates.
    let step = 0.35;
    let moved = |material: WaterMaterial| {
        let body = bodies(material);
        let before = render_water(context, &harness, &body, frame.at_time(0.0));
        let after = render_water(context, &harness, &body, frame.at_time(step));
        mean_difference_where(&before, &after, |pixel| wet[pixel])
    };
    let with_current = moved(flowing);
    let without = moved(WaterMaterial {
        current: [0.0, 0.0],
        ..flowing
    });
    assert!(
        with_current > without * 1.4,
        "a current moved the surface by {with_current:.2} of 255 over {step}s against \
         {without:.2} without one, so the water is not being carried downstream"
    );
}

#[test]
fn the_far_water_is_smoother_than_the_near_water_rather_than_noisier() {
    // The normal level-of-detail, stated as the property it exists to produce.
    //
    // Without it the shading normal carries every wave component at full strength however little of
    // the train a pixel covers, so the *far* half of a body — where a pixel spans several waves — is
    // the half that alternates hardest between the reflected sky and the dark body. That is backwards:
    // detail a pixel cannot resolve has to go, or it comes back as noise. Under a captured sky the
    // reflected term is bright enough for the difference to be several times over, which is why this
    // is measured there and not against the analytic gradient.
    //
    // Measured as mean absolute difference between horizontally adjacent water pixels, which is the
    // aliasing directly: a surface reading as a lit sheet has a low one, and a surface reading as
    // speckle has a high one however plausible its mean colour.
    let Some(context) = context() else { return };
    let scene = water_scene_under_sky(context);
    let sky = fixture_sky_bound(context, &scene.harness);
    let water = water_body(context, &scene, scene.level);
    let captured = render_water_under_sky(context, &scene.harness, &water, &sky, scene.frame);
    write_capture("water-detail-falloff.png", &captured);

    let dry = render_under_sky(context, &scene.harness, &sky, scene.frame);
    let mask = wet_mask(&dry, &captured);
    let rows: Vec<u32> = (0..HEIGHT)
        .filter(|y| (0..WIDTH).any(|x| mask[(y * WIDTH + x) as usize]))
        .collect();
    assert!(
        rows.len() > 40,
        "the lake spans only {} rows, which is too few to split into near and far",
        rows.len()
    );

    // In a frame with the horizon above the lake, a lower row is nearer. The two bands are the top and
    // bottom thirds, so the middle third is not counted twice or argued over.
    let third = rows.len() / 3;
    let roughness = |band: &[u32]| {
        let mut total = 0.0f64;
        let mut counted = 0usize;
        for y in band {
            for x in 1..WIDTH {
                let (here, left) = ((y * WIDTH + x) as usize, (y * WIDTH + x - 1) as usize);
                if !mask[here] || !mask[left] {
                    continue;
                }
                for channel in 0..3 {
                    let a = f64::from(captured.rgba()[here * 4 + channel]);
                    let b = f64::from(captured.rgba()[left * 4 + channel]);
                    total += (a - b).abs();
                }
                counted += 3;
            }
        }
        assert!(
            counted > 500,
            "only {counted} adjacent water pairs in a band"
        );
        total / counted as f64
    };
    let far = roughness(&rows[..third]);
    let near = roughness(&rows[rows.len() - third..]);
    // By half again, not merely by a hair. A bare `far < near` would be satisfied by noise, and the
    // property is not marginal: on the reference adapter the two bands come out at 0.92 and 1.88.
    assert!(
        far * 1.5 < near,
        "the far water is {far:.2} against the near water's {near:.2} in mean step between \
         adjacent pixels, which is not the falloff a damped wave normal produces — an undamped \
         one makes the far band the *rougher* of the two, because that is where a pixel spans \
         several waves"
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
fn fog_pools_in_the_low_ground_and_leaves_the_high_ground_standing() {
    // On the *shadowing* terrain, not the basin, and that choice is the whole test.
    //
    // The basin fixture cannot show height fog however it is tuned. It is a broad, gentle, near-planar
    // surface seen from a distant camera, so every ray has much the same length and crosses much the same
    // air — and fog is an integral along the ray, which smooths away whatever the density does. The result
    // was a uniform wash that four rounds of tuning could not shift, and the fault was the fixture.
    //
    // This terrain has a 900-unit spire and a steep ridge above a low plain. Rays to the spire top and to
    // the ground beside it differ enormously in both length and height, which is exactly the difference
    // height fog exists to express.
    let Some(context) = context() else { return };
    let harness = harness(context);
    let clear = render(
        context,
        &harness,
        DeferredFrame::new(pose(&harness.terrain), WIDTH, HEIGHT),
    );
    let (floor, rim) = world_elevation_range(&harness.terrain);

    let foggy =
        DeferredFrame::new(pose(&harness.terrain), WIDTH, HEIGHT).in_environment(Environment {
            fog: Fog {
                // Modest, because fog opacity is `1 - exp(-optical)` and saturates: at a density thick enough
                // to veil the frame the exponential is already near its ceiling, and any variation in density
                // stops being legible for a reason unrelated to the fog.
                density: 0.011,
                // Well under the spire's height, so the plain fills while the spire stands clear of it.
                height_falloff: (rim - floor) * 0.22,
                base: floor,
                patchiness: 0.8,
                // Large, comparable to the ray length: an integrated density averages several banks per ray, so
                // a small scale makes neighbouring pixels agree and returns the uniform wash.
                patch_scale: 950.0,
            },
            ..Environment::default()
        });
    let veiled = render(context, &harness, foggy);

    write_capture("atmosphere-fog.png", &veiled);
    support::check_reference(context, "atmosphere-fog.png", &veiled);

    let changed = fraction_differing(&clear, &veiled);
    assert!(
        changed > 0.30,
        "fog changed only {:.1}% of the frame",
        changed * 100.0
    );

    // The property that separates height fog from a distance haze: it must veil the frame *unevenly*. A
    // fog that took the same amount from every pixel would be a colour wash, and would pass a test that
    // only asked whether the spread shrank.
    let (smallest, largest, mean) = luminance_drop(&clear, &veiled);
    assert!(mean.abs() > 0.0, "fog changed nothing on the whole");
    assert!(
        largest - smallest > 0.12,
        "every pixel was veiled by about the same amount ({smallest:.4} to {largest:.4}), \
         so this is a colour wash rather than fog with height to it"
    );

    // And fog flattens: it pulls everything toward one colour, so the spread has to shrink.
    let (low_before, high_before) = clear.luminance_range();
    let (low_after, high_after) = veiled.luminance_range();
    assert!(
        (high_after - low_after) < (high_before - low_before),
        "fog did not reduce the luminance spread: {:.3} against {:.3}",
        high_after - low_after,
        high_before - low_before
    );
}

#[test]
fn wet_ground_is_darker_everywhere_rather_than_only_shinier() {
    // Wetness has to darken. A version that only dropped roughness would read as a polished floor, and the
    // mean drop below is what distinguishes the two.
    let Some(context) = context() else { return };
    let harness = harness(context);
    let dry = render(
        context,
        &harness,
        DeferredFrame::new(pose(&harness.terrain), WIDTH, HEIGHT),
    );
    let soaked = render(
        context,
        &harness,
        DeferredFrame::new(pose(&harness.terrain), WIDTH, HEIGHT).in_environment(
            Environment::default().with_weather(Weather {
                // Wetness alone, with no overcast, so the change measured is the *surface* and not the
                // dimmer sky that real rain would also bring.
                wetness: 1.0,
                ..Weather::default()
            }),
        ),
    );
    write_capture("weather-wet.png", &soaked);
    support::check_reference(context, "weather-wet.png", &soaked);

    let (_, _, mean) = luminance_drop(&dry, &soaked);
    assert!(
        mean > 0.05,
        "wet ground only lost {mean:.4} luminance on average, so it is shinier rather than wetter"
    );
    // Ground only: the sky is not wet, so a large part of the frame must be untouched.
    let changed = fraction_differing(&dry, &soaked);
    assert!(
        changed < 0.90,
        "wetness changed {:.1}% of the frame, which is more than the ground covers",
        changed * 100.0
    );
}

#[test]
fn snow_settles_on_flat_ground_and_not_on_the_steep() {
    // The claim worth testing is *selectivity*. Snow that covered everything uniformly would brighten the
    // frame just as much and would pass any assertion about the mean, so this measures the spread of the
    // per-pixel change: flats must gain far more than slopes.
    let Some(context) = context() else { return };
    let harness = harness(context);
    let bare = render(
        context,
        &harness,
        DeferredFrame::new(pose(&harness.terrain), WIDTH, HEIGHT),
    );
    let covered = render(
        context,
        &harness,
        DeferredFrame::new(pose(&harness.terrain), WIDTH, HEIGHT).in_environment(
            Environment::default().with_weather(Weather {
                snow: 1.0,
                ..Weather::default()
            }),
        ),
    );
    write_capture("weather-snow.png", &covered);
    support::check_reference(context, "weather-snow.png", &covered);

    // Snow brightens, so the drop is negative on the whole.
    let (smallest, largest, mean) = luminance_drop(&bare, &covered);
    assert!(mean < 0.0, "snow darkened the scene: mean drop {mean:.4}");
    assert!(
        largest - smallest > 0.15,
        "every changed pixel gained about the same amount ({smallest:.4} to {largest:.4}), \
         so the snow is a white wash rather than something settling by slope"
    );
    // The spire and the ridge flanks are steep enough to stay bare, so some ground must be untouched.
    let changed = fraction_differing(&bare, &covered);
    assert!(
        changed < 0.92,
        "snow reached {:.1}% of the frame, so it is not being held off the steep ground",
        changed * 100.0
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
    let first_targets = DeferredTargets::new(
        context,
        WIDTH,
        HEIGHT,
        cic_render::gpu::CAPTURE_FORMAT,
        DisplaySettings::NATIVE,
    )
    .expect("first targets");
    let first =
        DeferredRenderer::new(context, &renderer, &first_targets).expect("first deferred renderer");

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
    let second_targets = DeferredTargets::new(
        context,
        wide,
        tall,
        cic_render::gpu::CAPTURE_FORMAT,
        DisplaySettings::NATIVE,
    )
    .expect("second targets");
    let second = DeferredRenderer::new(context, &renderer, &second_targets)
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
        None,
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

#[test]
fn per_pass_timing_attributes_the_frame_to_the_passes_that_ran() {
    // The point of the whole module: that a *breakdown* exists, not that a total does. Every performance
    // question this renderer has open is a subtraction between two of these numbers, and the tests below
    // it in `timing` cover the arithmetic on synthetic ticks — what only a device can show is that the
    // slots line up with the passes, that a conditional pass is reported when it runs and absent when it
    // does not, and that the durations are physically plausible rather than nanoseconds or minutes.
    let Some(context) = context() else { return };
    if !context.supports_timing() {
        eprintln!("skipping: this adapter does not offer TIMESTAMP_QUERY");
        return;
    }
    let terrain = shadowing_terrain();
    let mut harness = harness_with(
        context,
        terrain,
        DisplaySettings::NATIVE.with_antialiasing(Antialiasing::Fxaa),
    );
    assert!(
        harness.deferred.set_timing(context, true),
        "the device reported timing support, so enabling it must take"
    );

    let frame = DeferredFrame::new(pose(&harness.terrain), WIDTH, HEIGHT);
    let _ = render(context, &harness, frame);
    let timings = harness
        .deferred
        .timings(context)
        .expect("timing is on")
        .expect("read the breakdown");
    eprintln!("{timings}");

    // Every pass with geometry in it, and the antialias pass, which ran because the settings asked for it.
    // All four cascades are in that list now: they are fitted over the scene the camera can see, so each
    // one has ground in it. Before that they were fitted from the near plane, and the first covered 88
    // units of air over an eye 614 units up -- an empty pass, cleared and rasterized for nothing.
    every_pass_is_plausible(
        &timings,
        &[
            TimedPass::ShadowCascade0,
            TimedPass::ShadowCascade1,
            TimedPass::ShadowCascade2,
            TimedPass::ShadowCascade3,
            TimedPass::Gbuffer,
            TimedPass::Occlusion,
            TimedPass::OcclusionBlur,
            TimedPass::Lighting,
            TimedPass::Composite,
            TimedPass::Antialias,
        ],
    );

    // One pass is absent, and the absence is not a claim about cost. This is the half a fixed slot layout
    // has to earn: reporting water at 0.000ms in a scene with no water would be a claim about what water
    // costs rather than about there being none.
    assert_eq!(
        timings.get(TimedPass::Water),
        None,
        "no water was drawn, so the water pass must not appear at all"
    );
    assert_eq!(timings.entries().len(), TimedPass::ALL.len() - 1);

    // The cascades drew the terrain over again, so their total is a real share of the frame rather than a
    // rounding error -- which is the first number the outstanding terrain LOD work wants.
    // Asserted as a share of the sum, not in milliseconds, so it holds on any device.
    let shadow_share = harness_share(timings.shadow_total(), timings.sum());
    eprintln!(
        "shadow cascades are {:.1}% of the summed passes",
        shadow_share * 100.0
    );
    assert!(
        shadow_share > 0.01,
        "the cascades came to {shadow_share:.4} of the frame, which would mean they are not drawing the \
         terrain they are supposed to be drawing"
    );

    // The other camera the fit is bounded differently for: this one runs out of scene rather than starting
    // late in it, and its cascades are packed into the 430 units it can see. Every cascade still has ground
    // in it, which is the property both cameras now share and used to disagree about.
    let low = DeferredFrame::new(ground_level_pose(&harness.terrain), WIDTH, HEIGHT);
    let _ = render(context, &harness, low);
    let from_the_ground = harness
        .deferred
        .timings(context)
        .expect("timing is on")
        .expect("read the breakdown");
    eprintln!("from the ground: {from_the_ground}");
    every_pass_is_plausible(&from_the_ground, TimedPass::CASCADES);
    assert_eq!(
        from_the_ground.entries().len(),
        TimedPass::ALL.len() - 1,
        "every cascade has casters from the ground, so water is the only absence left"
    );

    // The control for the *absent* cascade, which no camera looking at this terrain produces any more --
    // every cascade is fitted over ground it can see. Collapsing the shadow distance is what still empties
    // them: it is the same lever `shadows_darken_the_ground_they_fall_on` pulls to turn shadowing off, and
    // it leaves four cascades fitted into two units of air in front of the eye.
    //
    // Without this the assertions above would pass just as happily on a renderer that had gone back to
    // timing passes that rasterise nothing -- which cannot be timed at all on Metal, where the end-of-pass
    // timestamp is sampled at the end of the fragment stage, so timing one anyway reported "did not run"
    // about a pass that did, on one backend and not another.
    let mut collapsed = DeferredFrame::new(pose(&harness.terrain), WIDTH, HEIGHT);
    collapsed.shadow_distance = 0.5;
    let _ = render(context, &harness, collapsed);
    let without_casters = harness
        .deferred
        .timings(context)
        .expect("timing is on")
        .expect("read the breakdown");
    eprintln!("with the shadow distance collapsed: {without_casters}");
    for cascade in TimedPass::CASCADES {
        assert_eq!(
            without_casters.get(*cascade),
            None,
            "{cascade:?} caught no chunk, so it has nothing to attribute"
        );
    }
    assert_eq!(
        without_casters.entries().len(),
        TimedPass::ALL.len() - 1 - TimedPass::CASCADES.len(),
        "water and all four cascades are absent, and nothing else is"
    );

    // Turning it off stops the reporting rather than leaving stale numbers reachable.
    assert!(!harness.deferred.set_timing(context, false));
    assert!(harness.deferred.timings(context).is_none());
}

/// Asserts every listed pass was timed, and that its duration is physically plausible.
///
/// The bound is loose by design: this is a plausibility check, and a tight one would fail on the next
/// GPU. A pass that drew a 720x480 frame on real hardware is neither instant nor a tenth of a second.
fn every_pass_is_plausible(timings: &FrameTimings, passes: &[TimedPass]) {
    for pass in passes {
        let elapsed = timings
            .get(*pass)
            .unwrap_or_else(|| panic!("{pass:?} was not timed"));
        assert!(
            elapsed > Duration::ZERO && elapsed < Duration::from_millis(100),
            "{pass:?} took {elapsed:?}, which is not a plausible pass time"
        );
    }
}

/// One duration as a share of another, with a zero denominator answering zero.
fn harness_share(part: Duration, whole: Duration) -> f32 {
    if whole.is_zero() {
        return 0.0;
    }
    (part.as_secs_f64() / whole.as_secs_f64()) as f32
}

#[test]
fn a_terrain_that_does_not_fill_its_last_chunk_draws_no_ground_past_its_edge() {
    // The one path the other fixtures cannot reach. Chunks are 32 cells and both the shadowing terrain
    // (192 cells) and the model terrain (128) divide evenly, so every committed reference exercises the
    // chunked draw without ever producing a *partial* chunk.
    //
    // A partial chunk still submits a full chunk's vertices, and the out-of-range ones are collapsed to a
    // degenerate triangle in the vertex shader. Clamping them instead would not be harmless: `elevation`
    // clamps the coordinate it looks a height up at, but `world_position` does *not* clamp the x and y it
    // derives from the same coordinate — so a cell past the edge lands at its true world position carrying
    // the edge's height, and the terrain grows a slab hanging past its own extent. That is the water-slab
    // bug this renderer has already had once.
    //
    // Two terrains at the same spacing, one that divides evenly and one that does not, framed by the same
    // camera. Their footprints should differ by exactly the ratio of their extents; a slab out to the end
    // of the last chunk would make the ragged one almost twice the area it should be.
    let Some(context) = context() else { return };
    let spacing = 8.0;
    let exact = flat_at(96 + 1, spacing);
    let ragged = flat_at(99 + 1, spacing);

    // High enough that both terrains sit well inside the frame with sky around them, and looking straight
    // down so the footprint is the map's own rectangle rather than a perspective wedge.
    let framing = |samples: u32| {
        let extent = f32::from(u16::try_from(samples - 1).expect("small fixture")) * spacing;
        let centre = extent * 0.5;
        CameraPose {
            eye: [centre, centre - 1.0, 2_400.0],
            focus: [centre, centre, 0.0],
            forward: [0.0, 0.0, -1.0],
        }
    };

    let exact_cover = ground_fraction(context, exact, framing(97));
    let ragged_cover = ground_fraction(context, ragged, framing(100));
    // Both cameras frame their own map the same way, so a correct render gives both the same footprint.
    eprintln!("ground cover: {exact_cover:.4} at 96 cells, {ragged_cover:.4} at 99");
    assert!(
        exact_cover > 0.05,
        "the evenly divided terrain covered only {exact_cover:.4} of the frame, so this fixture is not \
         measuring a footprint at all"
    );
    let ratio = ragged_cover / exact_cover;
    assert!(
        (0.85..=1.15).contains(&ratio),
        "the ragged terrain covered {ragged_cover:.4} against {exact_cover:.4} for the evenly divided \
         one, a ratio of {ratio:.3}. Both frame their own extent identically, so the partial chunk is \
         drawing ground that is not there -- out to the end of its chunk, this reads about 1.7."
    );
}

/// A flat terrain of `samples` per side, at one elevation.
fn flat_at(samples: u32, spacing: f32) -> Terrain {
    Terrain::new(
        samples,
        samples,
        spacing,
        VERTICAL,
        vec![400; (samples * samples) as usize],
        Vec::new(),
    )
    .expect("valid flat terrain")
}

/// The fraction of a rendered frame that is ground rather than sky.
///
/// Classified by hue, as `sky_mask` does and for the same reason: the lighting pass paints a blue gradient
/// where coverage is zero, and no terrain in these fixtures is bluer than it is red.
fn ground_fraction(context: &GpuContext, terrain: Terrain, pose: CameraPose) -> f32 {
    let harness = harness_with(context, terrain, DisplaySettings::NATIVE);
    let frame = DeferredFrame::new(pose, WIDTH, HEIGHT);
    let capture = render(context, &harness, frame);
    let ground = capture
        .rgba()
        .chunks_exact(4)
        .filter(|pixel| pixel[2] <= pixel[0])
        .count();
    ground as f32 / (capture.rgba().len() / 4) as f32
}

/// Mean absolute Laplacian of luminance, over the pixels a mask selects.
///
/// A *second* difference rather than a gradient, and that choice is what makes the number mean
/// something. On a linear ramp — a sky gradient, an evenly lit slope — a pixel is the mean of its
/// neighbours and the term is zero however steep the ramp is. What survives is variation that changes
/// abruptly from one pixel to the next, which is precisely what a staircased silhouette is made of. A
/// gradient magnitude would instead report mostly how much contrast the scene happens to contain.
///
/// # Why it is masked, which was learned the hard way
///
/// Measured over the *whole* frame this number rises under supersampling, and the first version of the
/// test below read that as the resolution scale failing. It was not: magnifying the silhouette showed
/// the staircase properly graded, exactly as intended. The number rose because a higher sampling rate
/// legitimately puts *more* real high-frequency content into the frame — finer mip levels, a finer
/// occlusion estimate — and a Laplacian cannot tell detail that belongs there from aliasing that does
/// not. Two separate things were also feeding it, and only one was a fault: the composite's sharpen was
/// re-hardening the softened edges, which is fixed at the source in `composite.wgsl`.
///
/// Restricted to the silhouette the question becomes answerable, because there the correct image is
/// known to be a smooth boundary: any pixel-to-pixel step across it is aliasing and nothing else.
fn masked_edge_energy(capture: &Capture, mask: &[bool]) -> f32 {
    let (width, height) = (capture.width(), capture.height());
    assert!(width > 2 && height > 2, "frame too small to measure");
    let luma = |x: u32, y: u32| channel_luma(&capture.pixel(x, y).expect("pixel in range"));
    let mut total = 0.0;
    let mut counted = 0_u32;
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            if !mask[(y * width + x) as usize] {
                continue;
            }
            let neighbours = luma(x, y - 1) + luma(x, y + 1) + luma(x - 1, y) + luma(x + 1, y);
            total += (4.0 * luma(x, y) - neighbours).abs() / 4.0;
            counted += 1;
        }
    }
    assert!(counted > 500, "only {counted} pixels selected to measure");
    total / counted as f32
}

/// Marks the pixels the sky was painted on.
///
/// By *hue* rather than by brightness: the lighting pass paints a blue gradient where coverage is zero,
/// and this terrain's palette is a desaturated green, so the blue channel exceeding the red separates
/// them cleanly at any elevation in the gradient. A luminance threshold would not — the sky near the
/// horizon and the terrain in shadow overlap in brightness.
fn sky_mask(capture: &Capture) -> Vec<bool> {
    capture
        .rgba()
        .chunks_exact(4)
        .map(|pixel| pixel[2] > pixel[0])
        .collect()
}

/// Marks the pixels within `radius` of a change in `mask` — that is, the silhouette band.
///
/// Derived once from the *aliased* frame and then applied to every frame compared against it. That
/// matters: an antialiased edge is a blend and classifies either way, so a band recomputed per frame
/// would be measuring a different set of pixels in each and the comparison would mean nothing.
fn boundary_band(mask: &[bool], width: u32, height: u32, radius: u32) -> Vec<bool> {
    let at = |x: u32, y: u32| mask[(y * width + x) as usize];
    let mut band = vec![false; mask.len()];
    for y in 0..height {
        for x in 0..width {
            let here = at(x, y);
            let mut differs = false;
            // Clamped at the frame's edges rather than offset and range-checked, so the neighbourhood
            // walk stays in unsigned coordinates throughout.
            for ny in y.saturating_sub(radius)..=(y + radius).min(height - 1) {
                for nx in x.saturating_sub(radius)..=(x + radius).min(width - 1) {
                    if at(nx, ny) != here {
                        differs = true;
                    }
                }
            }
            band[(y * width + x) as usize] = differs;
        }
    }
    band
}

/// A mask selecting everything the band does not, so a claim about edges can be checked against a
/// control that contains no edges at all.
fn outside(band: &[bool]) -> Vec<bool> {
    band.iter().map(|inside| !inside).collect()
}

/// Mean absolute luminance difference between two captures, over the pixels a mask selects.
///
/// The measure for *where* a setting acts rather than how much. Two of these — one over the silhouette
/// band and one over everything else — say whether a change is an edge treatment or a change to the
/// whole image, which is the distinction every claim below turns on.
fn masked_difference(first: &Capture, second: &Capture, mask: &[bool]) -> f32 {
    assert_eq!(first.width(), second.width());
    assert_eq!(first.height(), second.height());
    let mut total = 0.0;
    let mut counted = 0_u32;
    for (index, (left, right)) in first
        .rgba()
        .chunks_exact(4)
        .zip(second.rgba().chunks_exact(4))
        .enumerate()
    {
        if !mask[index] {
            continue;
        }
        total += (channel_luma(left) - channel_luma(right)).abs();
        counted += 1;
    }
    assert!(counted > 500, "only {counted} pixels selected to measure");
    total / counted as f32
}

/// Rec. 709 luma of one stored pixel, matching `Capture`'s own measure.
fn channel_luma(pixel: &[u8]) -> f32 {
    let channel = |value: u8| f32::from(value) / 255.0;
    0.2126 * channel(pixel[0]) + 0.7152 * channel(pixel[1]) + 0.0722 * channel(pixel[2])
}

/// The fraction of pixels two captures agree on byte for byte.
fn fraction_identical(first: &Capture, second: &Capture) -> f32 {
    assert_eq!(first.width(), second.width());
    assert_eq!(first.height(), second.height());
    let identical = first
        .rgba()
        .chunks_exact(4)
        .zip(second.rgba().chunks_exact(4))
        .filter(|(left, right)| left == right)
        .count();
    identical as f32 / (first.rgba().len() / 4) as f32
}

/// Mean luminance across the whole frame.
fn mean_luminance(capture: &Capture) -> f32 {
    let total: f32 = capture.rgba().chunks_exact(4).map(channel_luma).sum();
    total / (capture.rgba().len() / 4) as f32
}

#[test]
fn antialiasing_softens_edges_without_dimming_or_blurring_the_frame() {
    // Three claims, because "the image changed" is not one of them and each of the three fails
    // differently.
    //
    // 1. Edge energy falls. That is the pass acting at all.
    // 2. Mean luminance does not move. A pass that redistributes brightness across an edge cannot
    //    change the frame's average by more than rounding; one whose luma weights, transfer curve, or
    //    blend direction are wrong shifts it. This is the assertion a plausible-looking wrong
    //    implementation fails.
    // 3. A substantial part of the frame is left *byte-identical*. This is the one that separates
    //    antialiasing from a blur: the luma gate has to be deciding not to fire. A pass that touches
    //    every pixel is a soft-focus filter wearing an antialiasing name — and it would satisfy both of
    //    the assertions above.
    let Some(context) = context() else { return };
    let terrain = shadowing_terrain();
    let aliased = harness_with(context, terrain.clone(), DisplaySettings::NATIVE);
    let resolved = harness_with(
        context,
        terrain,
        DisplaySettings::NATIVE.with_antialiasing(Antialiasing::Fxaa),
    );

    let frame = DeferredFrame::new(pose(&aliased.terrain), WIDTH, HEIGHT);
    let before = render(context, &aliased, frame);
    let after = render(context, &resolved, frame);
    write_capture("deferred-antialiased.png", &after);
    // The capture is the verification. A silhouette softened in the wrong direction, or shifted by half
    // a pixel, produces exactly the statistics below.
    support::check_reference(context, "deferred-antialiased.png", &after);

    assert_eq!(after.width(), WIDTH, "the output size must not change");
    assert_eq!(after.height(), HEIGHT);

    let band = boundary_band(&sky_mask(&before), WIDTH, HEIGHT, 2);
    let (rough, smooth) = (
        masked_edge_energy(&before, &band),
        masked_edge_energy(&after, &band),
    );
    eprintln!("silhouette edge energy: {rough:.5} aliased, {smooth:.5} resolved");
    assert!(
        smooth < rough * 0.8,
        "silhouette edge energy went from {rough:.5} to {smooth:.5}, so the pass is barely acting \
         where the aliasing is"
    );

    let shift = (mean_luminance(&after) - mean_luminance(&before)).abs();
    assert!(
        shift < 0.005,
        "mean luminance moved by {shift:.4}: the pass is changing the frame's brightness rather \
         than redistributing it across edges"
    );

    let untouched = fraction_identical(&before, &after);
    assert!(
        untouched > 0.2,
        "only {:.1}% of the frame was left alone, so the luma gate is not gating: this is a blur \
         rather than an antialias pass",
        untouched * 100.0
    );
    // And it must not be so timid that it is doing nothing either. A gate that never fires satisfies
    // every assertion above except the first, and it is worth recording which side of it we are on.
    //
    // The bound was 0.98 and had to be loosened, for a reason worth recording rather than hiding. While the
    // resolve passes offset their texture coordinate by half a pixel — see
    // `no_chunk_offsets_the_framebuffer_position_by_half_a_pixel` — every tap this pass took was a bilinear
    // average of two texels, so the luma gate saw a gradient almost everywhere and fired on about 3% of the
    // frame. With exact taps it fires on about 1%, and halves the silhouette edge energy while doing it. The
    // pass got *more* selective and no less effective, which is what a gate is for.
    assert!(
        untouched < 0.995,
        "{:.1}% of the frame is untouched, which is a pass that found almost no edges",
        untouched * 100.0
    );
}

#[test]
fn a_resolution_scale_raises_the_sampling_rate_rather_than_resizing_the_frame() {
    // Measured on the silhouette band and nowhere else. Over the whole frame the number goes the other
    // way, and it is right to: a higher sampling rate puts more genuine detail into the image, and no
    // second difference can separate detail that belongs there from a staircase that does not. On the
    // sky boundary the correct image is known to be a smooth curve, so there the question has an answer.
    // See `masked_edge_energy`.
    //
    // The ordering is the claim, not the direction: supersampling has to sit *between* an aliased frame
    // and a blurred one. "It got smoother" says nothing on its own, because upscaling a smaller render
    // does that too — and much further, since adjacent output pixels are then interpolated from the same
    // pair of source texels. A scale accepted and then ignored makes the first comparison an equality; a
    // downsample reading the wrong size fails it outright.
    let Some(context) = context() else { return };
    let terrain = shadowing_terrain();
    let native = harness_with(context, terrain.clone(), DisplaySettings::NATIVE);
    let supersampled = harness_with(
        context,
        terrain.clone(),
        DisplaySettings::NATIVE.at_scale(2.0),
    );
    let undersampled = harness_with(context, terrain, DisplaySettings::NATIVE.at_scale(0.5));

    assert_eq!(supersampled.targets.render_size(), (WIDTH * 2, HEIGHT * 2));
    assert_eq!(supersampled.targets.output_size(), (WIDTH, HEIGHT));
    assert_eq!(undersampled.targets.render_size(), (WIDTH / 2, HEIGHT / 2));

    let frame = DeferredFrame::new(pose(&native.terrain), WIDTH, HEIGHT);
    let at_native = render(context, &native, frame);
    let at_double = render(context, &supersampled, frame);
    let at_half = render(context, &undersampled, frame);
    write_capture("deferred-supersampled.png", &at_double);
    // A wrong downsample is a wrong *framing* — a quarter of the scene stretched across the whole target,
    // say — and every statistic here survives that. Only the image says so.
    support::check_reference(context, "deferred-supersampled.png", &at_double);

    for capture in [&at_double, &at_half] {
        assert_eq!(capture.width(), WIDTH, "the output size must not change");
        assert_eq!(capture.height(), HEIGHT);
    }

    let band = boundary_band(&sky_mask(&at_native), WIDTH, HEIGHT, 2);
    let elsewhere = outside(&band);

    // What it changes, and where. Supersampling is an edge treatment in effect even though it is not one
    // in construction: a smooth interior averages to what it already was, while a boundary pixel that
    // was wholly one side becomes a quarter, a half or three-quarters covered. So the change has to be
    // concentrated on the silhouette — which is also the assertion that fails if the downsample reads the
    // wrong size, because a quarter of the scene stretched over the whole target changes every pixel
    // about equally.
    let on_edges = masked_difference(&at_native, &at_double, &band);
    let off_edges = masked_difference(&at_native, &at_double, &elsewhere);
    eprintln!(
        "2x moved the silhouette by {on_edges:.4} and the rest of the frame by {off_edges:.4}"
    );
    assert!(
        on_edges > 0.01,
        "supersampling barely moved the silhouette at all ({on_edges:.4}), so the larger render is \
         not reaching the frame"
    );
    assert!(
        on_edges > off_edges * 4.0,
        "supersampling moved the silhouette by {on_edges:.4} and everything else by {off_edges:.4}, \
         which is not a change concentrated where the aliasing is"
    );

    // And the control that tells "rendered larger and averaged down" apart from "rendered smaller and
    // stretched". Both concentrate their change on the silhouette, because a smooth interior neither
    // averages nor interpolates to anything much different — so concentration alone does not say which
    // way the scale went. What does is the interior itself: averaging four samples of a smooth surface
    // returns very nearly what was already there, while magnifying from half as many samples shifts every
    // pixel to somewhere between two of them.
    let half_on_edges = masked_difference(&at_native, &at_half, &band);
    let half_off_edges = masked_difference(&at_native, &at_half, &elsewhere);
    eprintln!(
        "0.5x moved the silhouette by {half_on_edges:.4} and the rest of the frame by \
         {half_off_edges:.4}"
    );
    assert!(
        off_edges < half_off_edges * 0.5,
        "supersampling moved the frame's interior by {off_edges:.4} against {half_off_edges:.4} for an \
         upscale, so it is not leaving the smooth surfaces where they were"
    );

    // What this fixture cannot show, recorded so the next person does not read the silence as coverage:
    // whether supersampling *preserves* fine detail an upscale destroys. Its terrain is a smooth
    // heightfield in flat palette colours, so there is almost no detail off the silhouette either way —
    // measured at 0.00046 native against 0.00049 and 0.00045, which is three numbers agreeing rather than
    // an assertion. Making that claim needs a fixture with a texture in it, and the first candidate is the
    // world-space tiled albedo `terrain_render.rs` already exercises.

    // Supersampling must not dim the scene either. The composite's exposure and tone curve run per
    // output pixel whatever the render size, so a brightness change here would mean the downsample is
    // happening on the wrong side of that curve.
    let shift = (mean_luminance(&at_double) - mean_luminance(&at_native)).abs();
    assert!(
        shift < 0.01,
        "mean luminance moved by {shift:.4} under supersampling, so the downsample and the tone curve \
         are in the wrong order"
    );
}

#[test]
fn antialiasing_composes_with_a_resolution_scale() {
    // Both at once, which is how a settings screen will present them and therefore how they will be
    // used. It is also the arrangement with a size disagreement available in it: the composite reads a
    // target larger than the output and writes one at the output size, and the antialias pass then reads
    // *that* and must step by one output pixel rather than one render pixel. Stepping by the wrong one
    // shrinks its kernel and it quietly does much less.
    let Some(context) = context() else { return };
    let terrain = shadowing_terrain();
    let plain = harness_with(
        context,
        terrain.clone(),
        DisplaySettings::NATIVE.at_scale(1.5),
    );
    let resolved = harness_with(
        context,
        terrain,
        DisplaySettings::NATIVE
            .at_scale(1.5)
            .with_antialiasing(Antialiasing::Fxaa),
    );
    // A non-integer scale on purpose: the exact-average case at 2.0 is the easy one, and 1.5 is the
    // ratio where a downsample that assumed integer texel alignment shows itself.
    assert_eq!(plain.targets.render_size(), (1_080, 720));

    let frame = DeferredFrame::new(pose(&plain.terrain), WIDTH, HEIGHT);
    let scaled_only = render(context, &plain, frame);
    let both = render(context, &resolved, frame);
    write_capture("deferred-scaled-antialiased.png", &both);

    let band = boundary_band(&sky_mask(&scaled_only), WIDTH, HEIGHT, 2);
    let (rough, smooth) = (
        masked_edge_energy(&scaled_only, &band),
        masked_edge_energy(&both, &band),
    );
    eprintln!("silhouette edge energy at 1.5x: {rough:.5} plain, {smooth:.5} resolved");
    assert!(
        smooth < rough * 0.9,
        "with a 1.5x scale already in place the resolve took silhouette edge energy from {rough:.5} \
         only to {smooth:.5}, which is consistent with it stepping by render pixels rather than output \
         ones"
    );
    let untouched = fraction_identical(&scaled_only, &both);
    assert!(
        untouched > 0.2,
        "only {:.1}% of the supersampled frame was left alone",
        untouched * 100.0
    );
}

/// Renders a whole jitter cycle and returns the last frame.
///
/// This is what [ADR 0005](../../docs/adr/0005-antialiasing-strategy.md) predicted the harness would need:
/// a temporal accumulator makes one captured frame depend on the frames before it, so a reference image
/// stops being reproducible from a single render. The answer the ADR offered as one of two — render a fixed
/// number of frames with a pinned jitter sequence — is the one taken, and it is only available because the
/// jitter phase is a frame *parameter* rather than a counter inside the renderer.
///
/// Renders `JITTER_PHASES` frames, so every sub-pixel position has contributed. It is not fully converged at
/// that point and does not need to be: the reference is whatever the sequence produces, and the sequence is
/// deterministic.
fn render_converged(context: &GpuContext, harness: &Harness, frame: DeferredFrame) -> Capture {
    // Reset first, and that is not tidiness — it is what makes the sequence a *function* of its inputs. A
    // temporal resolve carries state across calls by construction, so without this the eight frames would
    // start from whatever the last sequence left behind and two identical calls would disagree. The first
    // draft of this helper omitted it and the reproducibility assertion below caught it immediately, which
    // is the case `reset_history` exists for.
    harness.deferred.reset_history();
    let mut capture = None;
    for phase in 0..JITTER_PHASES {
        capture = Some(render(context, harness, frame.at_jitter(phase)));
    }
    capture.expect("at least one jitter phase")
}

#[test]
fn a_temporal_resolve_accumulates_over_a_pinned_jitter_sequence() {
    // Four claims, and each of them fails differently:
    //
    // 1. The sequence is *reproducible*. Two runs of the same phases give the same bytes. Without this the
    //    reference below could not exist at all, and it is the property the whole design of `jitter` as a
    //    frame parameter is for.
    // 2. Edge energy on the silhouette falls. That is the accumulation acting where the aliasing is.
    // 3. Mean luminance does not move. A jitter that shifted the projection by a whole pixel, or an
    //    accumulation whose weights did not sum to one, moves it.
    // 4. The first frame of a sequence is *not* antialiased. There is no history to accumulate yet, so a
    //    first frame that already looked resolved would mean the pass was blending against uninitialised
    //    memory and happening to get away with it.
    let Some(context) = context() else { return };
    let terrain = shadowing_terrain();
    let aliased = harness_with(context, terrain.clone(), DisplaySettings::NATIVE);
    let temporal = harness_with(
        context,
        terrain,
        DisplaySettings::NATIVE.with_antialiasing(Antialiasing::Taa),
    );

    let frame = DeferredFrame::new(pose(&aliased.terrain), WIDTH, HEIGHT);
    let before = render(context, &aliased, frame);

    // The first frame has no predecessor, so it must be the unaccumulated image — and byte-identical to the
    // one the chain produces with no temporal path at all, because the jitter of phase 0 is the only thing
    // that differs and the resolve returns the current frame untouched.
    temporal.deferred.reset_history();
    let first = render(context, &temporal, frame.at_jitter(0));
    let converged = render_converged(context, &temporal, frame);
    write_capture("deferred-temporal-first.png", &first);
    write_capture("deferred-temporal.png", &converged);
    // The capture is the verification. A motion vector with a sign error, or a history sampled at the wrong
    // coordinate, produces exactly the statistics below.
    support::check_reference(context, "deferred-temporal.png", &converged);

    assert_eq!(converged.width(), WIDTH, "the output size must not change");
    assert_eq!(converged.height(), HEIGHT);

    let again = render_converged(context, &temporal, frame);
    assert_eq!(
        converged.rgba(),
        again.rgba(),
        "the same jitter sequence must give the same frame, or no reference could exist"
    );

    let band = boundary_band(&sky_mask(&before), WIDTH, HEIGHT, 2);
    let rough = masked_edge_energy(&before, &band);
    let unaccumulated = masked_edge_energy(&first, &band);
    let smooth = masked_edge_energy(&converged, &band);
    eprintln!(
        "silhouette edge energy: {rough:.5} aliased, {unaccumulated:.5} first frame, \
         {smooth:.5} converged"
    );
    assert!(
        smooth < rough * 0.8,
        "silhouette edge energy went from {rough:.5} to {smooth:.5}, so the accumulation is barely \
         acting where the aliasing is"
    );
    assert!(
        unaccumulated > smooth * 1.2,
        "the first frame ({unaccumulated:.5}) should be no better resolved than the aliased one, but it \
         is close to the converged frame ({smooth:.5}) -- which means the resolve blended against \
         something it should not have"
    );

    let shift = (mean_luminance(&converged) - mean_luminance(&before)).abs();
    assert!(
        shift < 0.005,
        "mean luminance moved by {shift:.4}: either the jitter is shifting the image rather than \
         sampling inside a pixel, or the accumulation weights do not sum to one"
    );
}

#[test]
fn a_temporal_resolve_follows_a_moving_camera_rather_than_smearing_it() {
    // What the motion target is for. Two accumulated sequences from *different* camera positions must
    // produce different frames, and each must stay as sharp as its own still frame — an accumulation that
    // ignored motion would blend the two positions and leave a trail.
    //
    // Measured as edge energy rather than as a difference, because a smear and a correct reprojection both
    // "change the frame". A trail is specifically a *loss* of edge energy that a still camera does not have.
    let Some(context) = context() else { return };
    let terrain = shadowing_terrain();
    let temporal = harness_with(
        context,
        terrain,
        DisplaySettings::NATIVE.with_antialiasing(Antialiasing::Taa),
    );
    let still = DeferredFrame::new(pose(&temporal.terrain), WIDTH, HEIGHT);

    // A pan of a few world units per frame, which at this standoff is several pixels — well past the point
    // where a history sampled at the wrong coordinate would be visible.
    let mut panned = still;
    let mut last = None;
    temporal.deferred.reset_history();
    for phase in 0..JITTER_PHASES {
        // Deliberately not derived from a clock: the pan is a fixed step per phase, so this sequence is as
        // reproducible as the still one.
        let step = 4.0 * f32::from(u8::try_from(phase).expect("eight phases"));
        panned.pose.eye[0] = still.pose.eye[0] + step;
        panned.pose.focus[0] = still.pose.focus[0] + step;
        panned.projection = still.projection;
        last = Some(render(context, &temporal, panned.at_jitter(phase)));
    }
    let moving = last.expect("at least one frame");
    write_capture("deferred-temporal-panned.png", &moving);

    // The reference for the moving case is the still one it must *not* resemble: a smear would pull the
    // silhouette toward where the camera was.
    let converged = render_converged(context, &temporal, still);
    assert_ne!(
        converged.rgba(),
        moving.rgba(),
        "a panned sequence must not produce the still frame"
    );

    // Sharpness, on the panned frame's own silhouette. A trail shows up here and nowhere else: the frame is
    // otherwise a plausible image of a slightly different camera.
    let band = boundary_band(&sky_mask(&moving), WIDTH, HEIGHT, 2);
    let panned_energy = masked_edge_energy(&moving, &band);
    let still_energy = masked_edge_energy(
        &converged,
        &boundary_band(&sky_mask(&converged), WIDTH, HEIGHT, 2),
    );
    eprintln!("silhouette edge energy: {still_energy:.5} still, {panned_energy:.5} panned");
    assert!(
        panned_energy > still_energy * 0.5,
        "the panned silhouette carries {panned_energy:.5} against the still frame's {still_energy:.5}, \
         so the history is being followed to the wrong place and smearing"
    );
}

#[test]
fn the_temporal_history_survives_a_frame_that_moves_nothing() {
    // The degenerate case a ping-pong gets wrong: rendering the same frame twice must keep accumulating
    // rather than reading the layer it is writing. If the swap were missed, the second frame would read its
    // own output and the result would either be the current frame alone or a validation error.
    let Some(context) = context() else { return };
    let temporal = harness_with(
        context,
        shadowing_terrain(),
        DisplaySettings::NATIVE.with_antialiasing(Antialiasing::Taa),
    );
    let frame = DeferredFrame::new(pose(&temporal.terrain), WIDTH, HEIGHT).at_jitter(3);

    temporal.deferred.reset_history();
    let mut frames = Vec::new();
    for _ in 0..6 {
        frames.push(render(context, &temporal, frame));
    }

    // The accumulation must be a *fixed point* under a repeated frame, not merely finite. Every frame after
    // the first blends the same image with a history that is already that image, so the sequence has to sit
    // still — a drift means the weights do not sum to one, and an oscillation means the history is being read
    // from the layer that is being written.
    //
    // Bit equality *is* the assertion, and it took a bug to earn it. While the resolve offset its texture
    // coordinate by half a pixel — see `no_chunk_offsets_the_framebuffer_position_by_half_a_pixel` — this
    // sequence read its own history from half a pixel away and re-filtered it every frame, decaying as
    // 48, 33, 19, 9, 6. That is a convergent sequence and it would have passed any tolerance stated as
    // "settles"; only demanding a genuine fixed point distinguishes an accumulation that is correct from
    // one that is merely stable.
    let largest = |left: &Capture, right: &Capture| {
        left.rgba()
            .iter()
            .zip(right.rgba())
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or(0)
    };
    let steps: Vec<u8> = frames
        .windows(2)
        .map(|pair| largest(&pair[0], &pair[1]))
        .collect();
    eprintln!("largest channel step between successive frames: {steps:?}");
    // Where the worst one is, because a number alone does not say whether it is an edge or the whole frame.
    if let Some(worst) = frames[0]
        .rgba()
        .iter()
        .zip(frames[1].rgba())
        .enumerate()
        .max_by_key(|(_, (a, b))| a.abs_diff(**b))
    {
        let pixel = worst.0 / 4;
        eprintln!(
            "worst at pixel ({}, {}) channel {}: {} then {}",
            pixel % WIDTH as usize,
            pixel / WIDTH as usize,
            worst.0 % 4,
            worst.1.0,
            worst.1.1
        );
    }
    for (index, step) in steps.iter().enumerate() {
        assert_eq!(
            *step,
            0,
            "frame {index} to {} moved a channel by {step}, so the accumulation is not a fixed point",
            index + 1
        );
    }
    // And it has not walked: the sixth frame is still within one step of the second, so the per-frame bound
    // above is not concealing a slow ramp of five successive one-step moves in the same direction.
    let total = largest(&frames[1], &frames[5]);
    eprintln!("largest channel step from frame 1 to frame 5: {total}");
    assert_eq!(
        total, 0,
        "the accumulation drifted by {total} over four repeated frames"
    );
}

/// A terrain split down the middle: the left half is layer 0 at full weight, the right half layer 1.
///
/// A hard boundary rather than a gradient, because what the page tests check is *where* a page reads its
/// data from, and a smooth field looks plausible under a coordinate that is off by a page.
fn split_terrain() -> Terrain {
    let samples = 65u32;
    let count = (samples * samples) as usize;
    let mut left = vec![0u8; count];
    let mut right = vec![0u8; count];
    for index in 0..count {
        if index as u32 % samples < samples / 2 {
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

/// A cache holding `layers` pages, warmed over the whole terrain at the coarse level.
fn warmed_cache(
    context: &GpuContext,
    harness: &Harness,
    layers: u32,
) -> cic_render::TerrainPageCache {
    let mut cache =
        cic_render::TerrainPageCache::new(context, &harness.renderer, layers).expect("page cache");
    let (cells_x, cells_y) = harness.renderer.cell_size();
    let composed = cache.update(
        context,
        &[TerrainDetailRequest::uniform(
            [0, 0],
            [cells_x, cells_y],
            16,
        )],
        page_view(&harness.terrain),
    );
    assert!(composed > 0, "the cache must stage pages to be useful");
    cache
}

/// Mean and worst per-channel difference between two captures of the same size.
fn channel_difference(left: &Capture, right: &Capture) -> (f64, u8) {
    let mut total = 0u64;
    let mut worst = 0u8;
    let mut counted = 0u64;
    for (a, b) in left
        .rgba()
        .chunks_exact(4)
        .zip(right.rgba().chunks_exact(4))
    {
        for channel in 0..3 {
            let difference = a[channel].abs_diff(b[channel]);
            total += u64::from(difference);
            worst = worst.max(difference);
            counted += 1;
        }
    }
    (total as f64 / counted.max(1) as f64, worst)
}

#[test]
fn terrain_sampled_from_pages_matches_the_direct_blend() {
    // The property that makes the cache worth having: the two paths compute the *same* surface, so a frame
    // drawn from pages and a frame drawn by blending must agree. If they did not, the cache would be a second
    // appearance for the same ground and the camera's distance would decide which one a player saw.
    //
    // Not byte-identical, and every reason is in the design rather than in the arithmetic: a page stores eight
    // bits per channel where the blend keeps float precision, a page sample is filtered from page texels
    // rather than from the layer weights, and a page picks its own mip level — twice over, once when the
    // compose pass chooses which albedo level to bake and again when the G-buffer chooses which page level to
    // read. So the assertion is a bound on the difference, in eight-bit steps.
    //
    // Drawn through the *deferred* chain, which is where the page lookup lives. The forward pass deliberately
    // has none: it draws terrain alone in one pass, which is the case a cache has nothing to offer — and the
    // first draft of this test used it and reported the two frames as identical for that reason.
    let Some(context) = context() else { return };
    let mut harness = harness_for(context, split_terrain());
    let frame = DeferredFrame::new(pose(&harness.terrain), WIDTH, HEIGHT);
    let direct = render(context, &harness, frame);

    let cache = warmed_cache(context, &harness, 16);
    assert!(!harness.renderer.samples_pages());
    harness.renderer.attach_pages(context, &cache);
    assert!(harness.renderer.samples_pages());
    let paged = render(context, &harness, frame);

    write_capture("terrain-direct-blend.png", &direct);
    write_capture("terrain-from-pages.png", &paged);
    // The capture is the verification. A page coordinate off by a border, a level chosen the wrong way round,
    // or a transfer function applied twice all produce a plausible image and a small mean difference.
    support::check_reference(context, "terrain-from-pages.png", &paged);

    // The frame must actually have changed hands: a bind group that silently kept its placeholders would make
    // the two captures identical and every bound below trivially true.
    assert_ne!(
        direct.rgba(),
        paged.rgba(),
        "the paged frame is byte-identical to the direct one, so the cache is not being read at all"
    );

    let (mean, worst) = channel_difference(&direct, &paged);
    eprintln!("direct against paged: mean channel difference {mean:.3}, worst {worst}");
    // Both bounds are set from what was measured — 0.004 and 5, up from 0.001 and 2 before the page carried a
    // mip chain, because at this view the page is very slightly minified and the level the G-buffer picks is
    // no longer always the base — with a wide margin rather than a generous guess, and the distinction is the
    // point. A page resolved to the wrong layer, or a coordinate off by a border, differs by *tens to
    // hundreds*; so a bound loose enough to be safe against adapter and filter variation is still two orders
    // of magnitude tighter than any real fault. Choosing one without measuring first would have meant the
    // looser number, which would have caught nothing.
    assert!(
        mean < 0.5,
        "the two paths must agree on the surface: mean channel difference {mean:.3}"
    );
    // The worst case is bounded separately, because a mean can hide a small region that is wholly wrong —
    // which is exactly what a page resolved to the wrong layer looks like.
    assert!(
        worst <= 8,
        "some pixel differs by {worst}, which is a region reading the wrong page rather than the eight-bit \
         quantisation a page store costs"
    );

    // Detaching restores the direct frame exactly, which is what says the fallback path is untouched by the
    // feature rather than merely available.
    harness.renderer.detach_pages(context);
    assert!(!harness.renderer.samples_pages());
    let restored = render(context, &harness, frame);
    assert_eq!(
        direct.rgba(),
        restored.rgba(),
        "detaching the cache must restore the direct frame byte for byte"
    );
}

#[test]
fn a_cell_with_no_resident_page_falls_back_to_the_direct_blend() {
    // The reason the direct blend stays in the shader. A cache with one slot holds one page, so almost every
    // fragment misses — and the frame has to be *the frame*, not a hole where the cache was. A cache is
    // allowed to run out of slots, so a frame that depended on it having won would turn a memory budget into a
    // correctness requirement.
    let Some(context) = context() else { return };
    let mut harness = harness_for(context, split_terrain());
    let frame = DeferredFrame::new(pose(&harness.terrain), WIDTH, HEIGHT);
    let direct = render(context, &harness, frame);

    let cache = warmed_cache(context, &harness, 1);
    assert_eq!(cache.layer_count(), 1);
    harness.renderer.attach_pages(context, &cache);
    let starved = render(context, &harness, frame);
    write_capture("terrain-one-page.png", &starved);

    // Most of the frame must be the direct blend's, byte for byte: one coarse page covers a sixteenth of this
    // terrain. A comfortable majority identical is the fallback working, and a small minority differing is the
    // one resident page being read rather than the cache being ignored.
    let identical = direct
        .rgba()
        .chunks_exact(4)
        .zip(starved.rgba().chunks_exact(4))
        .filter(|(left, right)| left[0..3] == right[0..3])
        .count();
    let share = identical as f64 / (direct.rgba().len() / 4) as f64;
    eprintln!("pixels identical to the direct blend with one page resident: {share:.3}");
    assert!(
        share > 0.8,
        "only {share:.3} of the frame fell back to the direct blend, so a cache miss is not being handled"
    );
    assert!(
        share < 1.0,
        "the whole frame is the direct blend, so the one resident page is not being read"
    );
}

/// A flat terrain whose two layers both carry a striped albedo.
///
/// Flat and striped for one reason: a mip chain is only worth anything where minification is severe and the
/// content has high spatial frequency. A flat plane seen at a shallow angle compresses without limit toward
/// the horizon, and a stripe at 16 world units per period goes sub-pixel there. A smooth heightfield in flat
/// palette colours — which is what every other fixture in this file is — measures the same energy whether the
/// chain exists or not, so it could not show this either way.
fn striped_plain() -> Terrain {
    split_terrain()
}

fn striped_materials() -> Vec<LayerMaterial> {
    [[0.85, 0.80, 0.62], [0.55, 0.62, 0.45]]
        .into_iter()
        .map(|colour| LayerMaterial::colour(colour).with_albedo(stripes(64, 4), 64.0))
        .collect()
}

/// A square image of horizontal stripes alternating between a quarter and full brightness.
fn stripes(size: u32, count: u32) -> TextureImage {
    let period = size.div_ceil(count.max(1));
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

/// A camera a few units above a flat plain, looking almost along it.
fn grazing_pose(terrain: &Terrain) -> CameraPose {
    let [extent_x, extent_y] = terrain.world_extent();
    let eye = [extent_x * 0.5, -extent_y * 0.1, 90.0];
    let focus = [extent_x * 0.5, extent_y, 50.0];
    let delta = [focus[0] - eye[0], focus[1] - eye[1], focus[2] - eye[2]];
    let length = (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt();
    CameraPose {
        eye,
        focus,
        forward: delta.map(|component| component / length),
    }
}

/// Mean squared difference between adjacent pixels along *both* axes, over the rows from `from_row` down.
///
/// The statistic aliasing shows up in. A minified pattern sampled at one level per page produces neighbouring
/// pixels that disagree far more than the surface they came from does, so the *energy* between adjacent pixels
/// rises even though the mean brightness does not move at all — which is why a mean or a luminance spread
/// reports nothing here.
///
/// Both axes, and the first version of this measured only one. The fixture's stripes run horizontally, so
/// along a row there is almost nothing to see and the figure came out at 1.49 where the vertical axis reads
/// 158 — a metric that happened to be perpendicular to the only detail in the frame. A sum over both axes
/// cannot be defeated by the orientation of the content.
fn adjacent_energy(capture: &Capture, from_row: u32) -> f64 {
    let width = capture.width();
    let rgba = capture.rgba();
    let luma = |x: u32, y: u32| {
        let at = ((y * width + x) * 4) as usize;
        0.2126 * f64::from(rgba[at])
            + 0.7152 * f64::from(rgba[at + 1])
            + 0.0722 * f64::from(rgba[at + 2])
    };
    let mut total = 0.0;
    let mut counted = 0u64;
    for y in from_row..capture.height() {
        for x in 0..width {
            if x > 0 {
                let difference = luma(x, y) - luma(x - 1, y);
                total += difference * difference;
                counted += 1;
            }
            if y > from_row {
                let difference = luma(x, y) - luma(x, y - 1);
                total += difference * difference;
                counted += 1;
            }
        }
    }
    total / counted.max(1) as f64
}

#[test]
fn a_page_sampled_at_a_grazing_angle_does_not_alias_worse_than_the_direct_blend() {
    // What the mip chain is *for*, and the one claim that decides whether the cache should be used at all. A
    // page used to hold a single density, so ground sampled under heavy minification read one texel out of
    // many it covered — while the fallback beside it, the direct layer blend, samples an albedo array that has
    // a full chain and so minifies gracefully. That made the cache correct and *worse*, on exactly the ground
    // a virtual texture exists for.
    //
    // Measured as the energy between adjacent pixels rather than as a difference from the direct frame,
    // because the two frames are legitimately allowed to differ: the question is not whether they agree but
    // whether the paged one is noisier.
    let Some(context) = context() else { return };
    let mut harness = harness_from(
        context,
        striped_plain(),
        DisplaySettings::NATIVE,
        &striped_materials(),
    );
    let frame = DeferredFrame::new(grazing_pose(&harness.terrain), WIDTH, HEIGHT);
    let direct = render(context, &harness, frame);

    let cache = warmed_cache(context, &harness, 16);
    harness.renderer.attach_pages(context, &cache);
    let paged = render(context, &harness, frame);

    write_capture("terrain-grazing-direct.png", &direct);
    write_capture("terrain-grazing-paged.png", &paged);
    assert_ne!(
        direct.rgba(),
        paged.rgba(),
        "the two frames are identical, so the cache is not being read and this measures nothing"
    );

    // Two regions, and the split is the finding rather than a convenience. The lower half of the frame is the
    // plain — above it is sky, identical in both frames, and including it would dilute both figures by the
    // same large constant. Within that, the *nearer* three fifths is ground whose minification is inside the
    // factor of eight the chain reaches, and the band above it is not.
    let plain = HEIGHT / 2;
    let near = HEIGHT * 3 / 5;
    let (direct_plain, paged_plain) = (
        adjacent_energy(&direct, plain),
        adjacent_energy(&paged, plain),
    );
    let (direct_near, paged_near) = (
        adjacent_energy(&direct, near),
        adjacent_energy(&paged, near),
    );
    eprintln!(
        "adjacent-pixel energy — whole plain: direct {direct_plain:.2}, paged {paged_plain:.2}, ratio {:.2}; \
         within the chain's reach: direct {direct_near:.2}, paged {paged_near:.2}, ratio {:.2}",
        paged_plain / direct_plain.max(f64::EPSILON),
        paged_near / direct_near.max(f64::EPSILON),
    );

    assert!(
        direct_near > 1.0,
        "the fixture shows no high-frequency detail at all ({direct_near:.4}), so it cannot say whether the \
         page path aliases: the stripes are not reaching the frame"
    );
    // Where the chain reaches, the paged frame is *smoother* than the direct blend rather than merely no
    // worse: 238 against 386, a ratio of 0.62, and the thirty-row bands within it read 0.47 to 0.89. The bound
    // is 1.0 because the claim is a comparison and not a tuned figure.
    assert!(
        paged_near <= direct_near,
        "within the chain's reach the page path aliases more than the direct blend it replaces \
         ({paged_near:.2} against {direct_near:.2}), so the chain is absent, not being sampled, or built at \
         the wrong level"
    );
    // And the whole plain, which is what stops the band above from being an exclusion nobody has to justify.
    // The topmost thirty rows of ground read 1.93 rather than 0.46, and the reason is a real limit rather than
    // a defect: four levels take a page's density down by eight, and a pixel at the horizon covers more ground
    // than that — so there the page saturates at its deepest level while the direct blend's albedo chain keeps
    // going. Ground that far should not have a page resident at all, which is the *residency* decision and so
    // the view-driven `TerrainDetailRequest` that M3 still lists, not a deeper chain. Confirmed by breaking it
    // on purpose: forcing `PAGE_MIPS` in `terrain_gbuffer.wgsl` to 1 takes this figure from 1.17 to 2.00 and
    // the horizon band from 1.93 to 2.94.
    assert!(
        paged_plain <= direct_plain * 1.4,
        "the page path aliases across the whole plain ({paged_plain:.2} against {direct_plain:.2}), which is \
         more than the horizon band alone can account for"
    );
}

// -------------------------------------------------------------------------------------------------
// A captured sky
//
// The fixture is *generated*, not a photograph. Two reasons, and the second is the one that matters:
// an HDRI is a copyrighted work and this tree carries none, and a generated sky can be made to state
// its own orientation -- a sun at a known azimuth, a distinctly coloured ground below the horizon, and
// a zenith that is not the horizon's colour -- so an image that is upside down, rotated, or mirrored is
// a failing assertion rather than something to be caught by eye.
// -------------------------------------------------------------------------------------------------

/// Where the fixture sky puts its sun, as an azimuth in radians and an elevation above the horizon.
const FIXTURE_SUN: (f32, f32) = (1.1, 0.61);

/// Angular radius of that sun's disc.
const FIXTURE_SUN_RADIUS: f32 = 0.06;

/// A synthetic equirectangular sky, encoded as a real Radiance file.
///
/// Encoded rather than handed to `SkyAsset::new` directly, so this exercises the container the way a
/// downloaded HDRI would: header, resolution line, RGBE scanlines and all. A decoder fault and a
/// renderer fault then both land here rather than one of them hiding behind a bypass.
///
/// Four features, each of which a specific mistake would destroy:
///
/// - a **blue zenith** and a **warm horizon**, so a flipped `v` swaps them;
/// - a **dark ground** below the horizon, so a sky sampled with the wrong sign appears as a black lid;
/// - a **sun disc** at [`FIXTURE_SUN`], so a rotation is measurable and a wrong one visible;
/// - radiance **far above one** in that disc, so an eight-bit path could not carry it.
fn fixture_sky() -> Vec<u8> {
    const SKY_WIDTH: u32 = 256;
    const SKY_HEIGHT: u32 = 128;
    let (sun_azimuth, sun_elevation) = FIXTURE_SUN;
    let sun = [
        sun_elevation.cos() * sun_azimuth.cos(),
        sun_elevation.cos() * sun_azimuth.sin(),
        sun_elevation.sin(),
    ];

    let mut bytes = b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n".to_vec();
    bytes.extend_from_slice(format!("-Y {SKY_HEIGHT} +X {SKY_WIDTH}\n").as_bytes());
    for y in 0..SKY_HEIGHT {
        let theta = (y as f32 + 0.5) / SKY_HEIGHT as f32 * std::f32::consts::PI;
        for x in 0..SKY_WIDTH {
            let phi = ((x as f32 + 0.5) / SKY_WIDTH as f32 - 0.5) * std::f32::consts::TAU;
            let direction = [
                theta.sin() * phi.cos(),
                theta.sin() * phi.sin(),
                theta.cos(),
            ];
            let mut colour = if direction[2] > 0.0 {
                let height = direction[2].powf(0.6);
                // Brighter and warmer on the sun's side of the sky, which every real environment is
                // and which this fixture needs for a second reason: a sky varying with elevation alone
                // would be almost rotationally symmetric, and the tests that turn it would then pass on
                // a renderer that ignored the rotation entirely.
                let facing = 0.78 + 0.34 * (phi - sun_azimuth).cos();
                [
                    (0.72 + (0.10 - 0.72) * height) * facing,
                    (0.46 + (0.22 - 0.46) * height) * facing,
                    0.30 + (0.58 - 0.30) * height,
                ]
            } else {
                // Ground. Dark and slightly warm, which is what a real environment's lower hemisphere
                // is and, more usefully here, unmistakably not the sky.
                //
                // Varied with longitude rather than flat, and that is load-bearing rather than
                // decorative: an RTS camera looks *down*, so a large share of the background a test
                // captures is below the horizon. A uniform lower hemisphere would make the rotation
                // test vacuous over exactly the pixels it has most of.
                let facing = 0.7 + 0.3 * phi.cos();
                [0.09 * facing, 0.075 * facing, 0.06 * facing]
            };
            let along = direction[0] * sun[0] + direction[1] * sun[1] + direction[2] * sun[2];
            if along.clamp(-1.0, 1.0).acos() < FIXTURE_SUN_RADIUS {
                colour = [3_000.0, 2_700.0, 2_200.0];
            }
            bytes.extend_from_slice(&to_rgbe(colour));
        }
    }
    bytes
}

/// Encodes linear radiance as one RGBE quadruple.
fn to_rgbe(colour: [f32; 3]) -> [u8; 4] {
    let peak = colour[0].max(colour[1]).max(colour[2]);
    if peak < 1.0e-32 {
        return [0, 0, 0, 0];
    }
    let exponent = peak.log2().floor() as i32 + 1;
    let scale = 256.0 / (exponent as f32).exp2();
    [
        (colour[0] * scale).clamp(0.0, 255.0) as u8,
        (colour[1] * scale).clamp(0.0, 255.0) as u8,
        (colour[2] * scale).clamp(0.0, 255.0) as u8,
        (exponent + 128) as u8,
    ]
}

fn fixture_sky_asset() -> SkyAsset {
    decode_radiance(&fixture_sky(), SkyLimits::default())
        .expect("the fixture is a valid Radiance file")
}

fn fixture_sky_bound(context: &GpuContext, harness: &Harness) -> Sky {
    Sky::new(
        context,
        harness.deferred.sky_layout(),
        &fixture_sky_asset(),
        SkySettings::default(),
    )
    .expect("upload the fixture sky")
}

/// Renders the chain with a captured sky bound and the environment taken from it.
fn render_under_sky(
    context: &GpuContext,
    harness: &Harness,
    sky: &Sky,
    frame: DeferredFrame,
) -> Capture {
    let frame = frame.in_environment(frame.environment.under_sky(sky.lighting()));
    harness
        .deferred
        .set_frame(context, &harness.renderer, &[], &[], frame)
        .expect("upload frame uniforms");
    harness.deferred.render(
        context,
        &harness.renderer,
        &[],
        &[],
        Some(sky),
        &harness.targets,
        harness.output.colour_view(),
    );
    harness
        .output
        .resolve(context, encoder(context))
        .expect("resolve composite")
}

/// Mean colour of the pixels a mask selects, as `[r, g, b]` in `0..=1`.
fn mean_colour(capture: &Capture, mut selected: impl FnMut(u32, u32) -> bool) -> [f32; 3] {
    let mut total = [0.0f32; 3];
    let mut counted = 0.0f32;
    for (index, pixel) in capture.rgba().chunks_exact(4).enumerate() {
        let x = index as u32 % capture.width();
        let y = index as u32 / capture.width();
        if !selected(x, y) {
            continue;
        }
        for channel in 0..3 {
            total[channel] += f32::from(pixel[channel]) / 255.0;
        }
        counted += 1.0;
    }
    if counted == 0.0 {
        return [0.0; 3];
    }
    [total[0] / counted, total[1] / counted, total[2] / counted]
}

#[test]
fn a_captured_sky_replaces_the_gradient_and_the_light_it_implied() {
    let Some(context) = context() else { return };
    let harness = harness(context);
    let sky = fixture_sky_bound(context, &harness);

    let pose = sky_pose(&harness.terrain);
    let plain = render(context, &harness, DeferredFrame::new(pose, WIDTH, HEIGHT));
    let captured = render_under_sky(
        context,
        &harness,
        &sky,
        DeferredFrame::new(pose, WIDTH, HEIGHT),
    );
    write_capture("deferred-sky-captured.png", &captured);
    // The image is the verification. Every assertion below passes on frames that are wrong in ways an
    // assertion cannot name -- a sky rotated a quarter turn, a horizon in the wrong place, a seam down
    // the meridian.
    support::check_reference(context, "deferred-sky-captured.png", &captured);

    // Most of the frame moved, which is the coarse statement that the environment reached both the
    // background and -- through the ambient it derives -- the ground under it.
    let moved = fraction_differing(&plain, &captured);
    assert!(
        moved > 0.9,
        "only {:.1}% of the frame changed; the environment is not reaching the passes",
        moved * 100.0
    );
}

#[test]
fn the_sky_is_sampled_by_direction_rather_than_by_screen_position() {
    // The property that separates a real environment from a gradient, and the one the analytic sky
    // *cannot* have: turning the sky must change what is behind the scene. Done by rotating the image
    // rather than the camera, which isolates it completely -- a yaw moves nothing else in the frame,
    // not the terrain, not the shadows, and not the ambient, because `lighting` is rotation-invariant.
    let Some(context) = context() else { return };
    let harness = harness(context);
    let mut sky = fixture_sky_bound(context, &harness);

    let frame = DeferredFrame::new(sky_pose(&harness.terrain), WIDTH, HEIGHT);
    let unturned = render_under_sky(context, &harness, &sky, frame);
    sky.set_settings(
        context,
        SkySettings {
            yaw: std::f32::consts::FRAC_PI_2,
            ..SkySettings::default()
        },
    );
    let turned = render_under_sky(context, &harness, &sky, frame);
    write_capture("deferred-sky-turned.png", &turned);

    // Only the sky can have moved, so the fraction differing is a lower bound on how much of the frame
    // is sky. A screen-space gradient would give exactly zero.
    let moved = fraction_differing(&unturned, &turned);
    assert!(
        moved > 0.05,
        "turning the sky a quarter turn changed {:.2}% of the frame, so it is not being sampled by \
         direction",
        moved * 100.0
    );
}

#[test]
fn the_zenith_is_at_the_top_of_the_frame_and_the_horizon_below_it() {
    // What catches a flipped `v`, which is the single most likely coordinate mistake in an
    // equirectangular lookup and the one that survives every statistical check: a sky that is upside
    // down has exactly the same histogram as one that is not.
    //
    // The fixture's zenith is blue and its horizon warm, so the test is which way round the frame is
    // rather than how bright it is.
    let Some(context) = context() else { return };
    let harness = harness(context);
    let sky = fixture_sky_bound(context, &harness);
    let capture = render_under_sky(
        context,
        &harness,
        &sky,
        DeferredFrame::new(sky_pose(&harness.terrain), WIDTH, HEIGHT),
    );

    // The top eighth of the frame is sky at this pose whatever the terrain does, and the band beneath
    // it is the sky nearer the horizon. Fixed bands rather than a located silhouette, so the
    // measurement does not depend on the fixture terrain's shape.
    let high = mean_colour(&capture, |_, y| y < HEIGHT / 8);
    let low = mean_colour(&capture, |_, y| (HEIGHT / 4..HEIGHT / 3).contains(&y));
    assert!(
        high[2] > high[0],
        "the top of the frame is not the blue zenith: {high:?}"
    );
    assert!(
        low[0] > high[0] && low[2] < high[2],
        "the sky does not warm toward the horizon: {low:?} beneath {high:?}"
    );
}

#[test]
fn aiming_the_sky_puts_its_sun_where_the_scene_says_the_sun_is() {
    // A captured sky has a sun in it and a scene has a directional light; nothing makes them agree by
    // default, and when they disagree the symptom is that every shadow falls away from a bright patch
    // of sky somewhere else. That reads as a shadow fault, which is why it is worth an assertion
    // rather than an eye.
    let Some(context) = context() else { return };
    let harness = harness(context);
    let mut sky = fixture_sky_bound(context, &harness);

    // Found from the image alone, and it must be where the fixture put it.
    assert!(
        (sky.sun_azimuth() - FIXTURE_SUN.0).abs() < FIXTURE_SUN_RADIUS * 2.0,
        "the image's sun was located at {} rather than {}",
        sky.sun_azimuth(),
        FIXTURE_SUN.0
    );

    let environment = Environment::default();
    sky.aim_at(context, environment.sun_direction());
    let [sun_x, sun_y, _] = environment.sun_direction();
    let wanted = sun_y.atan2(sun_x);
    let landed = sky.sun_azimuth() - sky.settings().yaw;
    assert!(
        (landed - wanted).abs() < 0.05,
        "aiming left the sky's sun at {landed} against the light's {wanted}; a sign error here \
         puts it twice as far out as doing nothing"
    );

    // And the rotation is real rather than only recorded.
    let frame = DeferredFrame::new(sky_pose(&harness.terrain), WIDTH, HEIGHT);
    let aimed = render_under_sky(context, &harness, &sky, frame);
    write_capture("deferred-sky-aimed.png", &aimed);
    sky.set_settings(context, SkySettings::default());
    let unaimed = render_under_sky(context, &harness, &sky, frame);
    assert!(
        fraction_differing(&aimed, &unaimed) > 0.01,
        "aiming the sky changed nothing in the frame"
    );
}

/// Renders a lake under a captured sky, with the environment taken from it.
fn render_water_under_sky(
    context: &GpuContext,
    harness: &Harness,
    water: &[WaterBody],
    sky: &Sky,
    frame: DeferredFrame,
) -> Capture {
    let frame = frame.in_environment(frame.environment.under_sky(sky.lighting()));
    harness
        .deferred
        .set_frame(context, &harness.renderer, &[], water, frame)
        .expect("upload frame uniforms");
    harness.deferred.render(
        context,
        &harness.renderer,
        &[],
        water,
        Some(sky),
        &harness.targets,
        harness.output.colour_view(),
    );
    harness
        .output
        .resolve(context, encoder(context))
        .expect("resolve composite")
}

#[test]
fn a_lake_reflects_the_captured_sky_rather_than_the_analytic_one() {
    // The second half of what an environment is for, and the half a background capture cannot show.
    // `reflection.wgsl` was written as the seam a non-analytic sky would be substituted at, and this is
    // the test that the substitution actually reached the water pass -- which binds its own groups, in
    // its own pipeline, and could perfectly well have been left behind.
    //
    // Measured over the lake alone. A reflection is a Fresnel-weighted share of the surface, so a
    // whole-frame statistic would be dominated by the terrain around it and would pass on a lake that
    // reflected nothing at all.
    let Some(context) = context() else { return };
    let scene = water_scene_under_sky(context);
    let sky = fixture_sky_bound(context, &scene.harness);
    let water = water_body(context, &scene, scene.level);

    let analytic = render_water(context, &scene.harness, &water, scene.frame);
    let captured = render_water_under_sky(context, &scene.harness, &water, &sky, scene.frame);
    write_capture("water-sky-analytic.png", &analytic);
    write_capture("water-sky-captured.png", &captured);
    // The image is the verification: a reflection that is present, upside down, or the wrong colour all
    // satisfy every number below.
    support::check_reference(context, "water-sky-captured.png", &captured);

    // Where the lake is: the pixels a flood changes against the same scene dry, under the same sky. Taken
    // against the *captured* frame so the mask is the lake rather than the lake plus everything the
    // ambient moved.
    let dry = render_under_sky(context, &scene.harness, &sky, scene.frame);
    let lake: Vec<bool> = dry
        .rgba()
        .chunks_exact(4)
        .zip(captured.rgba().chunks_exact(4))
        .map(|(a, b)| a != b)
        .collect();
    let covered = lake.iter().filter(|wet| **wet).count();
    assert!(
        covered > (WIDTH * HEIGHT) as usize / 40,
        "the lake covers {covered} pixels, which is too few to measure a reflection in"
    );

    let on_lake = |x: u32, y: u32| lake[(y * WIDTH + x) as usize];
    let analytic_lake = mean_colour(&analytic, on_lake);
    let captured_lake = mean_colour(&captured, on_lake);

    // The fixture sky is warm at the horizon and the analytic one is cold everywhere, and a lake seen at
    // this angle reflects the sky *near the horizon* -- so the reflected share has to carry the warmth
    // across. Stated as a ratio rather than as absolute channels, because the ambient moved too and
    // brightness alone would be satisfied by a lake that merely got lighter.
    let warmth = |colour: [f32; 3]| colour[0] / colour[2].max(1.0e-4);
    assert!(
        warmth(captured_lake) > warmth(analytic_lake) * 1.15,
        "the lake did not take the captured sky's colour: {captured_lake:?} against \
         {analytic_lake:?}"
    );

    // This capture used to show the surface **sparkling**, with dark pixels scattered through the
    // reflection, and said so here because no assertion bounded it. It was the Fresnel term aliasing
    // on the wave normals rather than anything to do with the sky lookup — it survived forcing the
    // reflection to a mip level where the environment is a flat wash — and it was pre-existing: at a
    // ten-degree grazing view a six-degree change in a wave's slope moves the reflected share from
    // 0.19 to 0.72, so neighbouring pixels alternated between the bright sky and the dark body. The
    // analytic sky had hidden it by being nearly as dark as the body, so there was nothing to
    // alternate *with*. The water pass now damps each wave component of the shading normal by how much
    // of it a pixel covers; `the_far_water_is_smoother_than_the_near_water_rather_than_noisier` is the
    // assertion that was missing, and this capture is where it is visible.
    //
    // And turning the sky moves what the lake reflects. This is what separates a reflection from a
    // tint: a Fresnel share of a constant would not care which way the environment is facing.
    let mut turned_sky = fixture_sky_bound(context, &scene.harness);
    turned_sky.set_settings(
        context,
        SkySettings {
            yaw: std::f32::consts::PI,
            ..SkySettings::default()
        },
    );
    let turned = render_water_under_sky(context, &scene.harness, &water, &turned_sky, scene.frame);
    let moved = lake
        .iter()
        .enumerate()
        .filter(|(index, wet)| {
            **wet
                && captured.rgba()[index * 4..index * 4 + 3]
                    != turned.rgba()[index * 4..index * 4 + 3]
        })
        .count();
    // A fifth of the lake, not all of it. A surface this flat, seen at this angle, reflects a narrow
    // band around the horizon, and half a turn changes what is in that band only by as much as the sky
    // varies with longitude. The bound separates "reflects the environment" from "is tinted by it"; it
    // is not a measure of how much of the sky a lake can see.
    assert!(
        moved * 5 > covered,
        "turning the sky half a turn moved only {moved} of the lake's {covered} pixels, so the \
         surface is tinted by the environment rather than reflecting it"
    );
}

/// The basin scene, seen from the shallow camera the sky tests use.
///
/// [`water_scene`] uses [`pose`], which has no horizon in frame — so its lake reflects only the
/// environment's lower hemisphere and the reflection is a flat colour whichever sky is bound. The
/// grazing angle this camera gives is also what makes the reflected share large enough to measure:
/// water's reflectance at normal incidence is two percent, and it is the Fresnel rise toward the
/// horizon that turns a lake into a mirror.
fn water_scene_under_sky(context: &GpuContext) -> WaterScene {
    let harness = harness_for(context, basin_terrain());
    let frame = DeferredFrame::new(sky_pose(&harness.terrain), WIDTH, HEIGHT);
    let dry = render_water(context, &harness, &[], frame);
    let (floor, rim) = world_elevation_range(&harness.terrain);
    let level = floor + (rim - floor) * 0.45;
    WaterScene {
        harness,
        dry,
        level,
        frame,
    }
}
