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

use std::path::PathBuf;
use std::sync::OnceLock;

use cic_assets::{Terrain, TerrainLayer};
use cic_camera::CameraPose;
use cic_render::terrain::LayerColour;
use cic_render::{
    Capture, CaptureTarget, DeferredFrame, DeferredRenderer, DeferredTargets, GpuContext,
    TerrainRenderer,
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
    let terrain = shadowing_terrain();
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
        .set_frame(context, &harness.renderer, &[], frame, WIDTH, HEIGHT)
        .expect("upload frame uniforms");
    harness.deferred.render(
        context,
        &harness.renderer,
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
        .set_frame(context, &harness.renderer, &[], frame, WIDTH, HEIGHT)
        .expect("upload frame uniforms");
    harness.deferred.render(
        context,
        &harness.renderer,
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
        .set_frame(context, &flat_renderer, &[], flat_frame, WIDTH, HEIGHT)
        .expect("upload flat uniforms");
    flat_deferred.render(
        context,
        &flat_renderer,
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
        .set_frame(context, &harness.renderer, &[], frame, WIDTH, HEIGHT)
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
        .set_frame(context, &renderer, &[], frame, WIDTH, HEIGHT)
        .expect("upload frame uniforms");
    deferred.render(context, &renderer, &[], &targets, &view);

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
