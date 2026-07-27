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

use std::path::PathBuf;
use std::sync::OnceLock;

use cic_assets::Terrain;
use cic_assets::model::{Model, ModelMaterial, ModelPrimitive, ModelVertex};
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
                uv: [(corner & 1) as f32, ((corner >> 1) & 1) as f32],
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
                base_color_texture: None,
                blended: false,
            },
            ModelMaterial {
                name: "roof".to_owned(),
                base_color: [0.34, 0.18, 0.14, 1.0],
                metallic: 0.0,
                roughness: 0.7,
                base_color_texture: None,
                blended: false,
            },
        ],
        has_skin: false,
        has_animation: false,
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
    let targets = DeferredTargets::new(context, WIDTH, HEIGHT).expect("targets");
    let deferred = DeferredRenderer::new(
        context,
        &renderer,
        &targets,
        cic_render::gpu::CAPTURE_FORMAT,
    )
    .expect("deferred renderer");
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
        .set_frame(context, &harness.renderer, models, frame, WIDTH, HEIGHT)
        .expect("upload uniforms");
    harness.deferred.render(
        context,
        &harness.renderer,
        models,
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
