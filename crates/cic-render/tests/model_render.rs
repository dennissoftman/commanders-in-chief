//! Instanced models rendered through the deferred chain, verified against a real GPU device.
//!
//! Captures land in `CARGO_TARGET_TMPDIR`. Assertions are a tripwire; the images are the verification.
//!
//! The models here are built in Rust rather than imported from glTF. `cic_assets::Model` is a plain
//! public struct, so a box needs no asset file — which keeps these tests about *rendering* rather than
//! about importing, a thing `cic-assets` already covers.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

mod support;

use std::path::PathBuf;
use std::sync::OnceLock;

use cic_assets::Terrain;
use cic_assets::model::{Model, ModelImage, ModelMaterial, ModelPrimitive, ModelVertex};
use cic_camera::CameraPose;
use cic_render::{
    Capture, CaptureTarget, DeferredFrame, DeferredRenderer, DeferredTargets, GpuContext,
    ModelBatch, ModelInstance, TerrainRenderer,
};

const WIDTH: u32 = 720;
const HEIGHT: u32 = 480;
const SAMPLES: u32 = 129;
const SPACING: f32 = 8.0;

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

/// Flat ground, so anything visible above it is a model and any shadow on it is cast by one.
fn flat_terrain() -> Terrain {
    Terrain::new(
        SAMPLES,
        SAMPLES,
        SPACING,
        0.5,
        vec![200u16; (SAMPLES * SAMPLES) as usize],
        Vec::new(),
    )
    .expect("valid flat terrain")
}

/// A unit box centred on its base, with two materials so per-vertex material indexing is exercised.
///
/// Two materials rather than one is the point: with a single material a broken index still reads the
/// right colour, so the test could not tell.
fn box_model(size: f32, height: f32) -> Model {
    textured_box_model(size, height, Vec::new(), [None, None])
}

/// The same box, with images attached and each material pointed at one of them.
fn textured_box_model(
    size: f32,
    height: f32,
    images: Vec<ModelImage>,
    textures: [Option<usize>; 2],
) -> Model {
    let half = size * 0.5;
    // (normal, four corners counter-clockwise seen from outside)
    let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
        // Top
        (
            [0.0, 0.0, 1.0],
            [
                [-half, -half, height],
                [half, -half, height],
                [half, half, height],
                [-half, half, height],
            ],
        ),
        // Bottom
        (
            [0.0, 0.0, -1.0],
            [
                [-half, half, 0.0],
                [half, half, 0.0],
                [half, -half, 0.0],
                [-half, -half, 0.0],
            ],
        ),
        // South
        (
            [0.0, -1.0, 0.0],
            [
                [-half, -half, 0.0],
                [half, -half, 0.0],
                [half, -half, height],
                [-half, -half, height],
            ],
        ),
        // North
        (
            [0.0, 1.0, 0.0],
            [
                [half, half, 0.0],
                [-half, half, 0.0],
                [-half, half, height],
                [half, half, height],
            ],
        ),
        // East
        (
            [1.0, 0.0, 0.0],
            [
                [half, -half, 0.0],
                [half, half, 0.0],
                [half, half, height],
                [half, -half, height],
            ],
        ),
        // West
        (
            [-1.0, 0.0, 0.0],
            [
                [-half, half, 0.0],
                [-half, -half, 0.0],
                [-half, -half, height],
                [-half, half, height],
            ],
        ),
    ];

    let mut primitives = Vec::new();
    for (index, (normal, corners)) in faces.into_iter().enumerate() {
        let vertices = corners
            .into_iter()
            .enumerate()
            .map(|(corner, position)| ModelVertex {
                position,
                normal,
                uv: quad_uv(corner),
            })
            .collect();
        primitives.push(ModelPrimitive {
            vertices,
            indices: vec![0, 1, 2, 0, 2, 3],
            // The roof takes the second material; the walls take the first.
            material: Some(usize::from(index == 0)),
        });
    }

    Model {
        name: "box".to_owned(),
        primitives,
        materials: vec![
            ModelMaterial {
                name: "wall".to_owned(),
                base_color: [0.62, 0.58, 0.52, 1.0],
                metallic: 0.0,
                roughness: 0.85,
                base_color_texture: textures[0],
                blended: false,
            },
            ModelMaterial {
                name: "roof".to_owned(),
                base_color: [0.34, 0.18, 0.14, 1.0],
                metallic: 0.0,
                roughness: 0.7,
                base_color_texture: textures[1],
                blended: false,
            },
        ],
        images,
        has_skin: false,
        has_animation: false,
    }
}

/// Texture coordinates for one corner of a quad whose corners run anticlockwise from `[-, -]`.
///
/// Not `[corner & 1, corner >> 1]`, which is the obvious form and is wrong: it walks the unit square
/// in Z order while the corners walk it in a ring, so the last two swap and every face's texture
/// arrives sheared along a diagonal. Invisible while these fixtures had no textures on them.
fn quad_uv(corner: usize) -> [f32; 2] {
    const RING: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    RING[corner % 4]
}

/// A checkerboard, as an image a model can carry.
///
/// High contrast on purpose: a texture that resembles its material's flat colour cannot distinguish a
/// working sampler from a broken one, because the frame looks the same either way.
fn checkerboard(size: u32, squares: u32, dark: [u8; 3], light: [u8; 3]) -> ModelImage {
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    let step = size.div_ceil(squares.max(1));
    for y in 0..size {
        for x in 0..size {
            let colour = if ((x / step) + (y / step)).is_multiple_of(2) {
                dark
            } else {
                light
            };
            rgba.extend_from_slice(&[colour[0], colour[1], colour[2], u8::MAX]);
        }
    }
    ModelImage {
        width: size,
        height: size,
        rgba,
    }
}

fn pose(terrain: &Terrain) -> CameraPose {
    let [extent_x, extent_y] = terrain.world_extent();
    let focus = [extent_x * 0.5, extent_y * 0.5, 100.0];
    CameraPose {
        eye: [
            focus[0] + extent_x * 0.30,
            focus[1] - extent_y * 0.52,
            300.0,
        ],
        focus,
        forward: [-0.30, 0.52, -0.40],
    }
}

/// A grid of instances, so instancing is exercised rather than a single draw.
fn instances(terrain: &Terrain) -> Vec<ModelInstance> {
    let [extent_x, extent_y] = terrain.world_extent();
    let mut placed = Vec::new();
    for row in 0..3u16 {
        for column in 0..4u16 {
            let x = extent_x * (0.26 + 0.16 * f32::from(column));
            let y = extent_y * (0.36 + 0.14 * f32::from(row));
            let rotation = 0.35 * f32::from(row * 4 + column);
            let instance =
                ModelInstance::placed([x, y, 100.0], rotation, 1.0 + 0.1 * f32::from(column));
            // Every third instance is tinted, standing in for the same hull under other markings.
            placed.push(if (row + column) % 3 == 0 {
                instance.with_tint([0.55, 0.72, 0.85, 1.0])
            } else {
                instance
            });
        }
    }
    placed
}

fn capture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
}

fn write_capture(name: &str, capture: &Capture) {
    let path = capture_dir().join(name);
    std::fs::write(&path, capture.png().expect("encode png")).expect("write png");
    eprintln!("wrote {}", path.display());
}

fn encoder(context: &GpuContext) -> wgpu::CommandEncoder {
    context
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("test resolve"),
        })
}

struct Harness {
    terrain: Terrain,
    renderer: TerrainRenderer,
    deferred: DeferredRenderer,
    targets: DeferredTargets,
    output: CaptureTarget,
}

fn harness(context: &GpuContext) -> Harness {
    let terrain = flat_terrain();
    let renderer = TerrainRenderer::new(context, &terrain, &[]).expect("terrain renderer");
    let targets = DeferredTargets::new(
        context,
        WIDTH,
        HEIGHT,
        cic_render::gpu::CAPTURE_FORMAT,
        cic_render::DisplaySettings::NATIVE,
    )
    .expect("targets");
    let deferred = DeferredRenderer::new(context, &renderer, &targets).expect("deferred renderer");
    let output = CaptureTarget::new(context, WIDTH, HEIGHT).expect("output");
    Harness {
        terrain,
        renderer,
        deferred,
        targets,
        output,
    }
}

fn render(
    context: &GpuContext,
    harness: &Harness,
    models: &[ModelBatch],
    frame: DeferredFrame,
) -> Capture {
    harness
        .deferred
        .set_frame(context, &harness.renderer, models, &[], frame)
        .expect("upload uniforms");
    harness.deferred.render(
        context,
        &harness.renderer,
        models,
        &[],
        &harness.targets,
        harness.output.colour_view(),
    );
    harness
        .output
        .resolve(context, encoder(context))
        .expect("resolve")
}

fn frame_with_low_sun(terrain: &Terrain) -> DeferredFrame {
    let mut frame = DeferredFrame::new(pose(terrain), WIDTH, HEIGHT);
    // Low and from the far side, so model shadows fall toward the camera across open ground.
    frame.light.direction = [-0.34, 0.82, 0.46];
    frame
}

#[test]
fn instanced_models_appear_on_the_terrain() {
    let Some(context) = context() else { return };
    let harness = harness(context);
    let batch = ModelBatch::new(
        context,
        &box_model(40.0, 70.0),
        &instances(&harness.terrain),
        harness.deferred.material_layout(),
    )
    .expect("upload model batch");
    assert_eq!(batch.instance_count(), 12);
    assert_eq!(batch.triangle_count(), 12, "a box is six quads");

    let frame = frame_with_low_sun(&harness.terrain);
    let with_models = render(context, &harness, std::slice::from_ref(&batch), frame);
    write_capture("model-instances.png", &with_models);

    let without = render(context, &harness, &[], frame);
    write_capture("model-terrain-only.png", &without);

    // Models must change the frame, and by a substantial area: twelve boxes plus their shadows.
    let mut changed = 0usize;
    for (bare, populated) in without
        .rgba()
        .chunks_exact(4)
        .zip(with_models.rgba().chunks_exact(4))
    {
        if bare[0..3] != populated[0..3] {
            changed += 1;
        }
    }
    eprintln!("pixels changed by models: {changed}");
    assert!(
        changed > 8_000,
        "twelve boxes and their shadows should cover real area, got {changed} pixels"
    );
}

#[test]
fn models_cast_shadows_onto_the_terrain() {
    // Terrain here is perfectly flat and unpainted, so it can cast nothing at all. Any shadow in the
    // frame therefore came from a model, which makes this test unable to pass on terrain shadowing.
    let Some(context) = context() else { return };
    let harness = harness(context);
    let batch = ModelBatch::new(
        context,
        &box_model(40.0, 70.0),
        &instances(&harness.terrain),
        harness.deferred.material_layout(),
    )
    .expect("upload model batch");
    let models = std::slice::from_ref(&batch);

    let frame = frame_with_low_sun(&harness.terrain);
    let shadowed = render(context, &harness, models, frame);

    // The control differs in shadowing alone: collapsing the shadow distance puts every receiver
    // outside all four cascades, leaving the same geometry lit identically but unshadowed.
    let mut control = frame;
    control.shadow_distance = 0.5;
    let unshadowed = render(context, &harness, models, control);
    write_capture("model-shadows.png", &shadowed);
    write_capture("model-unshadowed-control.png", &unshadowed);

    let mut darkened = 0usize;
    for (lit, shade) in unshadowed
        .rgba()
        .chunks_exact(4)
        .zip(shadowed.rgba().chunks_exact(4))
    {
        if i32::from(lit[1]) - i32::from(shade[1]) > 12 {
            darkened += 1;
        }
    }
    eprintln!("pixels darkened by model shadows: {darkened}");
    assert!(
        darkened > 3_000,
        "models should cast a substantial shadow onto flat ground, got {darkened} pixels"
    );
}

#[test]
fn a_tint_changes_only_the_tinted_instances() {
    // The salvage case: one mesh, per-instance colour. If tint were applied per *batch* rather than
    // per instance, tinting some would tint all and this comparison would find far more difference.
    let Some(context) = context() else { return };
    let harness = harness(context);
    let model = box_model(40.0, 70.0);
    let plain: Vec<ModelInstance> = instances(&harness.terrain)
        .into_iter()
        .map(|instance| instance.with_tint([1.0; 4]))
        .collect();

    let untinted = ModelBatch::new(context, &model, &plain, harness.deferred.material_layout())
        .expect("untinted batch");
    let tinted = ModelBatch::new(
        context,
        &model,
        &instances(&harness.terrain),
        harness.deferred.material_layout(),
    )
    .expect("tinted batch");

    let frame = frame_with_low_sun(&harness.terrain);
    let a = render(context, &harness, std::slice::from_ref(&untinted), frame);
    let b = render(context, &harness, std::slice::from_ref(&tinted), frame);
    write_capture("model-tinted.png", &b);

    let mut differing = 0usize;
    for (left, right) in a.rgba().chunks_exact(4).zip(b.rgba().chunks_exact(4)) {
        if left[0..3] != right[0..3] {
            differing += 1;
        }
    }
    eprintln!("pixels differing by tint: {differing}");
    assert!(
        differing > 200,
        "tinted instances should differ, got {differing}"
    );
    // Four of twelve instances are tinted, and shadows are unaffected by colour, so most of the
    // model area must be untouched. A per-batch tint would push this far higher.
    let total = a.rgba().len() / 4;
    assert!(
        differing < total / 4,
        "only some instances are tinted, but {differing} of {total} pixels changed"
    );
}

#[test]
fn an_empty_batch_and_no_batches_render_identically() {
    // An empty batch must be a no-op rather than a validation error, so a caller can keep a batch
    // around across frames where nothing of that kind exists.
    let Some(context) = context() else { return };
    let harness = harness(context);
    let empty = ModelBatch::new(
        context,
        &box_model(40.0, 70.0),
        &[],
        harness.deferred.material_layout(),
    )
    .expect("an empty batch is legal");
    assert_eq!(empty.instance_count(), 0);

    let frame = frame_with_low_sun(&harness.terrain);
    let with_empty = render(context, &harness, std::slice::from_ref(&empty), frame);
    let with_none = render(context, &harness, &[], frame);
    assert_eq!(
        with_empty.rgba(),
        with_none.rgba(),
        "an empty batch must draw nothing"
    );
}

#[test]
fn a_model_without_geometry_is_refused() {
    let Some(context) = context() else { return };
    let harness = harness(context);
    let empty_model = Model {
        name: "nothing".to_owned(),
        primitives: Vec::new(),
        materials: Vec::new(),
        images: Vec::new(),
        has_skin: false,
        has_animation: false,
    };
    let error = ModelBatch::new(
        context,
        &empty_model,
        &[ModelInstance::default()],
        harness.deferred.material_layout(),
    )
    .expect_err("a model with no geometry must be refused");
    assert!(
        matches!(error, cic_render::RenderError::EmptyModel),
        "got {error:?}"
    );
}

#[test]
fn a_base_colour_texture_reaches_the_frame() {
    let Some(context) = context() else { return };
    let harness = harness(context);
    let instances = instances(&harness.terrain);

    let plain = ModelBatch::new(
        context,
        &box_model(40.0, 70.0),
        &instances,
        harness.deferred.material_layout(),
    )
    .expect("untextured batch");

    // Only the walls are textured. The roof keeps its flat colour, which is what makes this able to
    // fail for the right reason: a shader that ignored the material's slice and sampled the whole
    // model would change the roof too, and the roof-area assertion below would catch it.
    let textured_model = textured_box_model(
        40.0,
        70.0,
        vec![checkerboard(64, 8, [20, 20, 24], [240, 235, 220])],
        [Some(0), None],
    );
    let textured = ModelBatch::new(
        context,
        &textured_model,
        &instances,
        harness.deferred.material_layout(),
    )
    .expect("textured batch");
    assert_eq!(
        textured.base_colour().layer_count(),
        1,
        "one image, one array slice"
    );
    assert_eq!(
        textured.base_colour().mip_level_count(),
        7,
        "a 64-pixel texture reduces to 1x1 in seven levels"
    );

    let frame = frame_with_low_sun(&harness.terrain);
    let without = render(context, &harness, std::slice::from_ref(&plain), frame);
    let with = render(context, &harness, std::slice::from_ref(&textured), frame);
    write_capture("model-untextured.png", &without);
    write_capture("model-textured.png", &with);
    // Pins instanced geometry, per-vertex material indexing, and the base-colour array together. A
    // sheared UV mapping passed every assertion in this file until an image was finally looked at.
    support::check_reference(context, "model-textured.png", &with);

    let mut changed = 0usize;
    for (bare, patterned) in without
        .rgba()
        .chunks_exact(4)
        .zip(with.rgba().chunks_exact(4))
    {
        if bare[0..3] != patterned[0..3] {
            changed += 1;
        }
    }
    eprintln!("pixels changed by the wall texture: {changed}");
    assert!(
        changed > 1_000,
        "a checkerboard on every wall should change real area, got {changed} pixels"
    );

    // The texture must add variation, not merely shift the colour. A checkerboard across twelve
    // boxes widens the spread of brightness in the frame; a flat replacement would not.
    assert!(
        with.luminance_deviation() > without.luminance_deviation(),
        "textured {} should vary more than untextured {}",
        with.luminance_deviation(),
        without.luminance_deviation()
    );
}

#[test]
fn an_untextured_material_is_unaffected_by_another_material_s_texture() {
    // The failure this exists for: sampling unconditionally and forgetting to discard the result for
    // a material that has no texture. That renders the roof as the wall's checkerboard, and every
    // "the texture appeared" assertion still passes.
    let Some(context) = context() else { return };
    let harness = harness(context);

    // One instance, looked at from above, so the roof is most of what the frame contains.
    let [extent_x, extent_y] = harness.terrain.world_extent();
    let centre = [extent_x * 0.5, extent_y * 0.5];
    let mut frame = DeferredFrame::new(
        CameraPose {
            eye: [centre[0], centre[1] - 60.0, 400.0],
            focus: [centre[0], centre[1], 170.0],
            forward: [0.0, 0.4, -0.9],
        },
        WIDTH,
        HEIGHT,
    );
    frame.light.direction = [-0.34, 0.82, 0.46];
    let placement = [ModelInstance::placed(
        [centre[0], centre[1], 100.0],
        0.0,
        3.0,
    )];

    let untextured_roof = textured_box_model(
        40.0,
        70.0,
        vec![checkerboard(64, 8, [20, 20, 24], [240, 235, 220])],
        [Some(0), None],
    );
    let textured_roof = textured_box_model(
        40.0,
        70.0,
        vec![checkerboard(64, 8, [20, 20, 24], [240, 235, 220])],
        [Some(0), Some(0)],
    );

    let a = ModelBatch::new(
        context,
        &untextured_roof,
        &placement,
        harness.deferred.material_layout(),
    )
    .expect("batch");
    let b = ModelBatch::new(
        context,
        &textured_roof,
        &placement,
        harness.deferred.material_layout(),
    )
    .expect("batch");

    let plain_roof = render(context, &harness, std::slice::from_ref(&a), frame);
    let patterned_roof = render(context, &harness, std::slice::from_ref(&b), frame);
    write_capture("model-untextured-roof.png", &plain_roof);
    write_capture("model-textured-roof.png", &patterned_roof);

    let mut differing = 0usize;
    for (left, right) in plain_roof
        .rgba()
        .chunks_exact(4)
        .zip(patterned_roof.rgba().chunks_exact(4))
    {
        if left[0..3] != right[0..3] {
            differing += 1;
        }
    }
    eprintln!("pixels differing when the roof gains the same texture: {differing}");
    assert!(
        differing > 500,
        "texturing the roof must change the roof, got {differing} pixels"
    );
}

#[test]
fn instances_can_be_updated_without_reuploading_geometry() {
    let Some(context) = context() else { return };
    let harness = harness(context);
    let all = instances(&harness.terrain);
    let mut batch = ModelBatch::new(
        context,
        &box_model(40.0, 70.0),
        &all,
        harness.deferred.material_layout(),
    )
    .expect("upload batch");

    let frame = frame_with_low_sun(&harness.terrain);
    let full = render(context, &harness, std::slice::from_ref(&batch), frame);

    batch
        .set_instances(context, &all[..4])
        .expect("fewer instances fit the existing buffer");
    assert_eq!(batch.instance_count(), 4);
    let fewer = render(context, &harness, std::slice::from_ref(&batch), frame);
    assert_ne!(
        full.rgba(),
        fewer.rgba(),
        "removing instances must change the frame"
    );

    // Growing past the allocation is refused rather than silently truncated.
    let mut grown = all.clone();
    grown.extend_from_slice(&all);
    assert!(batch.set_instances(context, &grown).is_err());
}
