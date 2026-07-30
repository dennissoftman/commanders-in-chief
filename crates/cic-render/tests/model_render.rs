//! Instanced models rendered through the deferred chain, verified against a real GPU device.
//!
//! Captures land in `CARGO_TARGET_TMPDIR`. Assertions are a tripwire; the images are the verification.
//!
//! The models here are built in Rust rather than imported from glTF. `cic_assets::Model` is a plain
//! public struct, so a box needs no asset file — which keeps these tests about *rendering* rather than
//! about importing, a thing `cic-assets` already covers.

// The fixture builders here are tables of corner coordinates. Splitting one to satisfy a line count would
// put half a cube in one function and half in another, which is less readable rather than more.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::too_many_lines
)]

mod support;

use std::path::PathBuf;
use std::sync::OnceLock;

use cic_assets::Terrain;
use cic_assets::model::{
    AlphaMode, Model, ModelImage, ModelMaterial, ModelPrimitive, ModelTextures, ModelVertex,
};
use cic_assets::texture::{BlockFormat, TextureAsset, TextureLimits};
use cic_camera::CameraPose;
use cic_render::{
    Capture, CaptureTarget, DeferredFrame, DeferredRenderer, DeferredTargets, GpuContext,
    ModelBatch, ModelInstance, SwayProfile, TerrainRenderer,
};

const WIDTH: u32 = 720;
const HEIGHT: u32 = 480;
const SAMPLES: u32 = 129;
const SPACING: f32 = 8.0;

static CONTEXT: OnceLock<Option<GpuContext>> = OnceLock::new();

fn context() -> Option<&'static GpuContext> {
    CONTEXT.get_or_init(support::shared_context).as_ref()
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
                ..ModelVertex::default()
            })
            .collect();
        primitives.push(
            ModelPrimitive {
                vertices,
                indices: vec![0, 1, 2, 0, 2, 3],
                // The roof takes the second material; the walls take the first.
                material: Some(usize::from(index == 0)),
            }
            // These models are built in Rust, so the importer's derivation never ran on them and every
            // tangent is the unset zero. A normal map is then correctly ignored, which is exactly the
            // shape of failure a fixture must not have: the test would pass while measuring nothing.
            .with_generated_tangents(),
        );
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
                ..ModelMaterial::default()
            },
            ModelMaterial {
                name: "roof".to_owned(),
                base_color: [0.34, 0.18, 0.14, 1.0],
                metallic: 0.0,
                roughness: 0.7,
                base_color_texture: textures[1],
                ..ModelMaterial::default()
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
        // Unnamed, so no block-compressed sidecar is looked up for it.
        name: String::new(),
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

/// Encodes a flat colour as a block-compressed texture at a given size, with a full mip chain.
///
/// Flat rather than patterned, and that is the point: a flat block has exactly one encoding, so this
/// fixture can state the colour the frame must show without depending on a compressor's choices. What is
/// under test is the *upload* — that blocks reach the texture unit with the right row pitch, the right mip
/// level and the right layer — not how well anything compresses.
fn compressed_solid(size: u32, format: BlockFormat, colour: [u8; 4]) -> TextureAsset {
    TextureAsset::solid(size, size, format, colour, TextureLimits::default())
        .expect("a flat texture is always encodable")
}

#[test]
fn a_block_compressed_base_colour_renders_the_same_frame_as_an_uncompressed_one() {
    // The end-to-end claim of ADR 2001: the same picture through two upload paths. A flat colour is used
    // precisely so the two are comparable — the compressed path cannot resample or re-mip, so any
    // difference here is the upload itself rather than a compression loss.
    //
    // On an adapter without TEXTURE_COMPRESSION_BC this still runs and still asserts, because the
    // compressed model falls back to decoding the sidecar to RGBA8. That is the fallback being tested
    // rather than skipped.
    let Some(context) = context() else { return };
    let harness = harness(context);
    let instances = instances(&harness.terrain);
    let colour = [201u8, 199, 87, 255];

    // The uncompressed control: the same flat colour as a plain image.
    let control_model = textured_box_model(
        40.0,
        70.0,
        vec![ModelImage {
            width: 32,
            height: 32,
            rgba: colour.repeat(32 * 32),
            // Unnamed, so no block-compressed sidecar is looked up for it.
            name: String::new(),
        }],
        [Some(0), Some(0)],
    );
    let control = ModelBatch::new(
        context,
        &control_model,
        &instances,
        harness.deferred.material_layout(),
    )
    .expect("uncompressed batch");
    assert_eq!(
        control.base_colour().block_format(),
        None,
        "a plain image must take the RGBA8 path"
    );

    // The same model, its one image overridden by a BC7 sRGB sidecar.
    let mut compressed_model = control_model.clone();
    compressed_model.images[0].name = "hull_basecolor".to_owned();
    let textures = ModelTextures::new(vec![Some(compressed_solid(
        32,
        BlockFormat::Bc7UnormSrgb,
        colour,
    ))]);
    let compressed = ModelBatch::with_textures(
        context,
        &compressed_model,
        &textures,
        &instances,
        harness.deferred.material_layout(),
    )
    .expect("compressed batch");

    let took_the_fast_path = context.supports_block_compression();
    assert_eq!(
        compressed.base_colour().block_format(),
        took_the_fast_path.then_some(BlockFormat::Bc7UnormSrgb),
        "the path taken must follow the device's capability, not chance"
    );
    assert_eq!(
        compressed.base_colour().mip_level_count(),
        6,
        "a 32-pixel texture reaches 1x1 in six levels, whichever path uploaded it"
    );

    let frame = frame_with_low_sun(&harness.terrain);
    let plain = render(context, &harness, std::slice::from_ref(&control), frame);
    let blocks = render(context, &harness, std::slice::from_ref(&compressed), frame);
    write_capture("model-uncompressed-control.png", &plain);
    write_capture("model-block-compressed.png", &blocks);

    // Byte-for-byte equality, not a tolerance -- and the colour is chosen to make that available. All four
    // of its channels are odd, so they agree on the low bit BC7 mode 6 shares between them, and the block
    // reproduces the colour exactly. A colour whose channels disagreed would be within one
    // least-significant bit instead, and this assertion would have to soften to a tolerance that could hide
    // things.
    //
    // What it rules out is the failure this path really has: a wrong row pitch, a wrong mip extent or a
    // wrong layer origin. None of those shifts a frame by a shade — they scramble it. The bug found while
    // writing this was exactly one of them, a copy extent given in logical rather than block-aligned
    // texels, and `wgpu` validation caught it before any pixel did.
    let differing = plain
        .rgba()
        .chunks_exact(4)
        .zip(blocks.rgba().chunks_exact(4))
        .filter(|(control_texel, block_texel)| control_texel != block_texel)
        .count();
    assert_eq!(
        differing, 0,
        "{differing} texels differ between the two upload paths"
    );

    // And the frame is not simply empty, which every tolerance test has to rule out separately: a pass
    // that drew nothing would agree with another pass that drew nothing, perfectly.
    let lit = blocks
        .rgba()
        .chunks_exact(4)
        .filter(|texel| texel[0] > 40 || texel[1] > 40)
        .count();
    assert!(
        lit > 1_000,
        "only {lit} texels are lit; the models did not draw"
    );
}

#[test]
fn a_slot_whose_sidecars_disagree_falls_back_rather_than_refusing() {
    // The rule ADR 2001 sets, exercised where it bites: two base-colour images, one converted. A
    // compressed array cannot mix a compressed slice with an uncompressed one, and it has no resample
    // available -- so the slot waits, on a path that works, rather than failing the model load.
    let Some(context) = context() else { return };
    let harness = harness(context);
    let model = textured_box_model(
        40.0,
        70.0,
        vec![
            ModelImage {
                width: 32,
                height: 32,
                rgba: [200u8, 60, 60, 255].repeat(32 * 32),
                name: "converted".to_owned(),
            },
            ModelImage {
                width: 32,
                height: 32,
                rgba: [60u8, 60, 200, 255].repeat(32 * 32),
                // Unnamed, so no block-compressed sidecar is looked up for it.
                name: String::new(),
            },
        ],
        [Some(0), Some(1)],
    );

    let half_converted = ModelTextures::new(vec![
        Some(compressed_solid(
            32,
            BlockFormat::Bc7UnormSrgb,
            [200, 60, 60, 255],
        )),
        None,
    ]);
    let batch = ModelBatch::with_textures(
        context,
        &model,
        &half_converted,
        &instances(&harness.terrain),
        harness.deferred.material_layout(),
    )
    .expect("a half-converted slot must load, not fail");
    assert_eq!(
        batch.base_colour().block_format(),
        None,
        "one image of two converted is not a compressed array"
    );
    assert_eq!(
        batch.base_colour().layer_count(),
        2,
        "both images still get a slice, so a material index still means what it did"
    );

    // Sidecars at different sizes are the other rejected case, and it is rejected for the same reason:
    // resampling blocks would mean decoding and re-encoding them at load time.
    let mismatched = ModelTextures::new(vec![
        Some(compressed_solid(
            32,
            BlockFormat::Bc7UnormSrgb,
            [200, 60, 60, 255],
        )),
        Some(compressed_solid(
            16,
            BlockFormat::Bc7UnormSrgb,
            [60, 60, 200, 255],
        )),
    ]);
    let batch = ModelBatch::with_textures(
        context,
        &model,
        &mismatched,
        &instances(&harness.terrain),
        harness.deferred.material_layout(),
    )
    .expect("mismatched sizes must load, not fail");
    assert_eq!(batch.base_colour().block_format(), None);

    // And when they agree, the slot does take the fast path -- so the two assertions above are about the
    // disagreement rather than about the feature never working.
    let both = ModelTextures::new(vec![
        Some(compressed_solid(
            32,
            BlockFormat::Bc7UnormSrgb,
            [200, 60, 60, 255],
        )),
        Some(compressed_solid(
            32,
            BlockFormat::Bc7UnormSrgb,
            [60, 60, 200, 255],
        )),
    ]);
    let mut named = model.clone();
    named.images[1].name = "also_converted".to_owned();
    let batch = ModelBatch::with_textures(
        context,
        &named,
        &both,
        &instances(&harness.terrain),
        harness.deferred.material_layout(),
    )
    .expect("a fully converted slot");
    assert_eq!(
        batch.base_colour().block_format(),
        context
            .supports_block_compression()
            .then_some(BlockFormat::Bc7UnormSrgb)
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

/// A tangent-space normal map whose surface is a grid of shallow pyramids.
///
/// Not noise and not a flat field: a pyramid has four faces tilting in four known directions, so a
/// capture shows whether the perturbation follows the surface basis or is rotated, mirrored, or ignored.
/// A noise map would produce plausible variation whichever of those went wrong.
fn pyramid_normals(size: u32, cells: u32, tilt: f32) -> ModelImage {
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    let step = size.div_ceil(cells.max(1)) as f32;
    for y in 0..size {
        for x in 0..size {
            // Position within the cell, mapped to -1..1 on each axis, so each quadrant of the cell
            // tilts away from its centre.
            let local_x = ((x as f32 % step) / step) * 2.0 - 1.0;
            let local_y = ((y as f32 % step) / step) * 2.0 - 1.0;
            let normal_x = -local_x.signum() * tilt;
            let normal_y = -local_y.signum() * tilt;
            let encode = |value: f32| ((value * 0.5 + 0.5) * 255.0).clamp(0.0, 255.0) as u8;
            rgba.extend_from_slice(&[
                encode(normal_x),
                encode(normal_y),
                // The stored z. The shader rebuilds it from xy rather than reading this, so a sensible
                // value here is documentation rather than data.
                encode(
                    (1.0 - normal_x * normal_x - normal_y * normal_y)
                        .max(0.0)
                        .sqrt(),
                ),
                255,
            ]);
        }
    }
    ModelImage {
        // Unnamed, so no block-compressed sidecar is looked up for it.
        name: String::new(),
        width: size,
        height: size,
        rgba,
    }
}

/// A metallic-roughness map, uniform in both channels. Roughness is green, metallic is blue.
fn metallic_roughness(size: u32, roughness: u8, metallic: u8) -> ModelImage {
    ModelImage {
        // Unnamed, so no block-compressed sidecar is looked up for it.
        name: String::new(),
        width: size,
        height: size,
        rgba: [0, roughness, metallic, 255].repeat((size * size) as usize),
    }
}

/// An image whose alpha is a filled circle: the shape a leaf card cuts out of its own quad.
///
/// A circle specifically, because it has no straight edges. The failure worth catching is an alpha test
/// that does not run at all, and a rectangular cut-out is indistinguishable from the untested quad.
fn circle_cutout(size: u32, colour: [u8; 3]) -> ModelImage {
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    let centre = size as f32 * 0.5;
    let radius = size as f32 * 0.44;
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 + 0.5 - centre;
            let dy = y as f32 + 0.5 - centre;
            let inside = dx * dx + dy * dy <= radius * radius;
            rgba.extend_from_slice(&[
                colour[0],
                colour[1],
                colour[2],
                if inside { 255 } else { 0 },
            ]);
        }
    }
    ModelImage {
        // Unnamed, so no block-compressed sidecar is looked up for it.
        name: String::new(),
        width: size,
        height: size,
        rgba,
    }
}

/// Two crossed quads on a bare stem: the standard way foliage is authored, and the case that needs both
/// the alpha test and double-sided drawing.
///
/// The stem is opaque and the canopy is masked, so one model exercises both index ranges — which is the
/// arrangement that catches a split putting the wrong primitives in the wrong run.
fn foliage_model(spread: f32, height: f32, canopy_base: f32) -> Model {
    let half = spread * 0.5;
    let stem = spread * 0.06;
    let mut primitives = Vec::new();

    let quad = |normal: [f32; 3], corners: [[f32; 3]; 4], material: usize| {
        ModelPrimitive {
            vertices: corners
                .into_iter()
                .enumerate()
                .map(|(corner, position)| ModelVertex {
                    position,
                    normal,
                    uv: quad_uv(corner),
                    ..ModelVertex::default()
                })
                .collect(),
            indices: vec![0, 1, 2, 0, 2, 3],
            material: Some(material),
        }
        .with_generated_tangents()
    };

    // The stem: a square column, opaque, material 0.
    for (normal, corners) in [
        (
            [0.0, -1.0, 0.0],
            [
                [-stem, -stem, 0.0],
                [stem, -stem, 0.0],
                [stem, -stem, canopy_base],
                [-stem, -stem, canopy_base],
            ],
        ),
        (
            [1.0, 0.0, 0.0],
            [
                [stem, -stem, 0.0],
                [stem, stem, 0.0],
                [stem, stem, canopy_base],
                [stem, -stem, canopy_base],
            ],
        ),
        (
            [0.0, 1.0, 0.0],
            [
                [stem, stem, 0.0],
                [-stem, stem, 0.0],
                [-stem, stem, canopy_base],
                [stem, stem, canopy_base],
            ],
        ),
        (
            [-1.0, 0.0, 0.0],
            [
                [-stem, stem, 0.0],
                [-stem, -stem, 0.0],
                [-stem, -stem, canopy_base],
                [-stem, stem, canopy_base],
            ],
        ),
    ] {
        primitives.push(quad(normal, corners, 0));
    }

    // The canopy: two quads crossing at right angles, masked, material 1.
    for (normal, corners) in [
        (
            [0.0, -1.0, 0.0],
            [
                [-half, 0.0, canopy_base],
                [half, 0.0, canopy_base],
                [half, 0.0, height],
                [-half, 0.0, height],
            ],
        ),
        (
            [1.0, 0.0, 0.0],
            [
                [0.0, -half, canopy_base],
                [0.0, half, canopy_base],
                [0.0, half, height],
                [0.0, -half, height],
            ],
        ),
    ] {
        primitives.push(quad(normal, corners, 1));
    }

    Model {
        name: "foliage".to_owned(),
        primitives,
        materials: vec![
            ModelMaterial {
                name: "trunk".to_owned(),
                base_color: [0.32, 0.24, 0.18, 1.0],
                metallic: 0.0,
                roughness: 0.9,
                ..ModelMaterial::default()
            },
            ModelMaterial {
                name: "leaves".to_owned(),
                base_color: [1.0, 1.0, 1.0, 1.0],
                metallic: 0.0,
                roughness: 0.8,
                base_color_texture: Some(0),
                alpha_mode: AlphaMode::Masked { cutoff: 0.5 },
                double_sided: true,
                ..ModelMaterial::default()
            },
        ],
        images: vec![circle_cutout(64, [96, 140, 62])],
        has_skin: false,
        has_animation: false,
    }
}

/// A grid of plants, all rigid or all swaying.
fn plantings(terrain: &Terrain, profile: Option<SwayProfile>) -> Vec<ModelInstance> {
    let [extent_x, extent_y] = terrain.world_extent();
    let mut placed = Vec::new();
    for row in 0..3u16 {
        for column in 0..4u16 {
            let position = [
                extent_x * (0.26 + 0.16 * f32::from(column)),
                extent_y * (0.36 + 0.14 * f32::from(row)),
                100.0,
            ];
            let rotation = 0.4 * f32::from(row * 4 + column);
            placed.push(match profile {
                Some(profile) => ModelInstance::planted(position, rotation, 1.0, profile),
                None => ModelInstance::placed(position, rotation, 1.0),
            });
        }
    }
    placed
}

/// Whether a pixel belongs to the fixture canopy.
///
/// Green dominance. The canopy texture is the only green thing in these scenes -- terrain is a warm grey
/// and the sky is blue -- so this measures canopy *area*, which is what an alpha test changes. Counting
/// newly-visible sky instead was tried and is a far thinner signal, because a canopy silhouetted against
/// the sky is a small part of the frame while its area is not.
fn is_canopy(pixel: &[u8]) -> bool {
    pixel[1] > pixel[0].saturating_add(10) && pixel[1] > pixel[2].saturating_add(10)
}

/// Whether a pixel is open sky rather than lit geometry.
///
/// Blue *dominance*, not darkness. An absolute brightness threshold was tried first and was wrong: the
/// lighting pass's horizon band is (55, 70, 87), so a cutoff anywhere near it excludes most of the sky
/// and the counts collapse to nothing. The gradient is strongly blue-dominant at every height, and the
/// surfaces in the fixtures using this are warm, so the ordering of the channels separates them at any
/// exposure.
///
/// Only usable where nothing in the scene is blue: the `instances` helper tints every third box bluish
/// for the per-instance colour tests, and those pixels would classify as sky. Tests over tinted geometry
/// measure something else — see the coverage comparison in the normal-map test.
fn is_sky(pixel: &[u8]) -> bool {
    pixel[2] > pixel[0].saturating_add(15) && pixel[2] > pixel[1].saturating_add(5)
}

/// The pixels where a capture differs from a reference capture of the same scene without the models.
///
/// An exact statement of what the geometry touched, arrived at without classifying colours: a pixel
/// differs from the model-free frame if and only if a model covered it or a model shadowed it. Both of
/// those are properties of geometry alone, so two frames whose geometry agrees have identical sets
/// however differently they are shaded.
fn touched_pixels(capture: &Capture, without_models: &Capture) -> Vec<bool> {
    capture
        .rgba()
        .chunks_exact(4)
        .zip(without_models.rgba().chunks_exact(4))
        .map(|(pixel, bare)| pixel[0..3] != bare[0..3])
        .collect()
}

/// Mean channel sum over the pixels a predicate accepts.
fn mean_channel_sum(capture: &Capture, accept: fn(&[u8]) -> bool) -> f64 {
    let mut sum = 0u64;
    let mut count = 0u64;
    for pixel in capture.rgba().chunks_exact(4) {
        if accept(pixel) {
            sum += u64::from(pixel[0]) + u64::from(pixel[1]) + u64::from(pixel[2]);
            count += 1;
        }
    }
    sum as f64 / count.max(1) as f64
}

/// How many pixels differ in colour between two captures of the same size.
fn differing_pixels(left: &Capture, right: &Capture) -> usize {
    left.rgba()
        .chunks_exact(4)
        .zip(right.rgba().chunks_exact(4))
        .filter(|(a, b)| a[0..3] != b[0..3])
        .count()
}

#[test]
fn a_normal_map_perturbs_the_shading_without_moving_the_silhouette() {
    // The property that distinguishes a normal map from displacement, and the one a still capture can
    // check: the shading changes and the outline does not. A map wired into the position instead of the
    // normal would pass a "did anything change" assertion and fail this one.
    let Some(context) = context() else { return };
    let harness = harness(context);
    let placement = instances(&harness.terrain);

    let flat = ModelBatch::new(
        context,
        &box_model(40.0, 70.0),
        &placement,
        harness.deferred.material_layout(),
    )
    .expect("flat batch");

    let mut mapped_model = box_model(40.0, 70.0);
    mapped_model.images = vec![pyramid_normals(64, 4, 0.8)];
    for material in &mut mapped_model.materials {
        material.normal_texture = Some(0);
        material.roughness = 0.45;
    }
    // The flat batch takes the same roughness, so the only difference between the two frames is the map.
    let mut flat_model = box_model(40.0, 70.0);
    for material in &mut flat_model.materials {
        material.roughness = 0.45;
    }
    let flat_smooth = ModelBatch::new(
        context,
        &flat_model,
        &placement,
        harness.deferred.material_layout(),
    )
    .expect("smooth flat batch");
    drop(flat);
    let mapped = ModelBatch::new(
        context,
        &mapped_model,
        &placement,
        harness.deferred.material_layout(),
    )
    .expect("normal-mapped batch");

    let frame = frame_with_low_sun(&harness.terrain);
    let without = render(context, &harness, std::slice::from_ref(&flat_smooth), frame);
    let with = render(context, &harness, std::slice::from_ref(&mapped), frame);
    write_capture("model-flat-normals.png", &without);
    write_capture("model-normal-mapped.png", &with);
    support::check_reference(context, "model-normal-mapped.png", &with);

    let changed = differing_pixels(&without, &with);
    eprintln!("pixels changed by the normal map: {changed}");
    assert!(
        changed > 1_000,
        "a normal map on every face should change real area, got {changed}"
    );
    // It must add variation rather than shifting the whole surface, which is what a map decoded in the
    // wrong colour space does: a uniform tilt everywhere reads as a different light direction.
    assert!(
        with.luminance_deviation() > without.luminance_deviation(),
        "normal-mapped {} should vary more than flat {}",
        with.luminance_deviation(),
        without.luminance_deviation()
    );

    // The silhouette is unchanged, stated exactly. Both frames are compared against the same scene with
    // no models in it, which gives the set of pixels the geometry touched without classifying any colour
    // -- and a colour classifier is precisely what cannot do this job, because a fragment the map
    // darkened can cross a threshold while sitting still.
    let bare = render(context, &harness, &[], frame);
    let flat_coverage = touched_pixels(&without, &bare);
    let mapped_coverage = touched_pixels(&with, &bare);
    let moved = flat_coverage
        .iter()
        .zip(&mapped_coverage)
        .filter(|(left, right)| left != right)
        .count();
    let touched = flat_coverage.iter().filter(|touched| **touched).count();
    eprintln!("coverage: {touched} pixels touched, {moved} of them changed");
    assert!(
        touched > 10_000,
        "the fixture must cover real area for that comparison to mean anything"
    );
    // Not zero, and the residue is real rather than tolerated slop: a shaded box fragment can land on
    // exactly the terrain colour behind it, in which case it registers as untouched in one frame and
    // touched in the other purely because the shading changed. That is a fraction of a percent and it is
    // scattered; a geometric displacement moves whole edges and would be two orders of magnitude larger.
    assert!(
        moved * 500 < touched,
        "a normal map must not move geometry: {moved} of {touched} touched pixels changed coverage"
    );
}

#[test]
fn a_metallic_material_loses_its_diffuse_term() {
    // Two boxes differing only in the metallic channel of one map. A metal has no subsurface scattering,
    // so it loses the whole diffuse term and regains only a narrow coloured lobe — which makes it
    // *darker*, and the direction of the change is the assertion. A metallic path wired backwards would
    // also "change the frame".
    let Some(context) = context() else { return };
    let harness = harness(context);
    let placement = instances(&harness.terrain);

    let build = |roughness: u8, metallic: u8| {
        let mut model = box_model(40.0, 70.0);
        model.images = vec![metallic_roughness(4, roughness, metallic)];
        for material in &mut model.materials {
            material.metallic_roughness_texture = Some(0);
            // Both factors at one, so the map alone decides. glTF multiplies rather than replacing.
            material.metallic = 1.0;
            material.roughness = 1.0;
        }
        ModelBatch::new(
            context,
            &model,
            &placement,
            harness.deferred.material_layout(),
        )
        .expect("batch")
    };
    let dielectric = build(90, 0);
    let metal = build(90, 255);

    let frame = frame_with_low_sun(&harness.terrain);
    let painted = render(context, &harness, std::slice::from_ref(&dielectric), frame);
    let plated = render(context, &harness, std::slice::from_ref(&metal), frame);
    write_capture("model-dielectric.png", &painted);
    write_capture("model-metallic.png", &plated);
    support::check_reference(context, "model-metallic.png", &plated);

    // Over the whole frame, including the sky. The sky is identical in both, so it dilutes the difference
    // rather than biasing it -- and the sky classifier is not available here, because this placement
    // tints every third instance bluish.
    let dull = mean_channel_sum(&painted, |_| true);
    let shiny = mean_channel_sum(&plated, |_| true);
    eprintln!("mean channel sum: dielectric {dull}, metal {shiny}");
    assert!(
        shiny < dull,
        "a metal loses its diffuse term, so {shiny} must be below {dull}"
    );

    // And the roughness half of the same map reached the surface: a fully rough material has no
    // highlight at all, so raising roughness must change the frame.
    let rough = build(255, 0);
    let matte = render(context, &harness, std::slice::from_ref(&rough), frame);
    assert!(
        differing_pixels(&painted, &matte) > 500,
        "the roughness channel must reach the surface too"
    );
}

#[test]
fn alpha_tested_foliage_cuts_its_own_silhouette_and_its_own_shadow() {
    // The whole reason the alpha test has to exist in the cascades and not only in the lit frame. Two
    // renders of the same canopy, one masked and one opaque: the masked one must show sky through the
    // gaps *and* must not lay a solid rectangle of shadow on the ground.
    let Some(context) = context() else { return };
    let harness = harness(context);
    let placement = plantings(&harness.terrain, None);

    let masked_model = foliage_model(70.0, 110.0, 40.0);
    let masked = ModelBatch::new(
        context,
        &masked_model,
        &placement,
        harness.deferred.material_layout(),
    )
    .expect("masked batch");
    assert!(masked.has_cutout(), "the canopy cuts and is two-sided");
    assert!(masked.has_solid(), "the stem is neither");
    assert!(
        !ModelBatch::new(
            context,
            &box_model(10.0, 10.0),
            &placement,
            harness.deferred.material_layout(),
        )
        .expect("box batch")
        .has_cutout(),
        "a single-sided opaque model must stay on the solid path"
    );
    assert_eq!(
        masked.triangle_count(),
        12,
        "four stem quads and two canopy quads"
    );

    // The same model with the cut turned off, which is what the frame looked like before the alpha test
    // existed: two solid quads per plant.
    // Only the cutoff changes. `double_sided` stays set, so this model takes the same pipeline and the
    // same culling as the masked one — which is what makes the comparison below about the alpha test
    // alone. Comparing against a single-sided variant instead was tried and confounded the measurement:
    // culling the back of a crossed quad removes canopy area too, and it removed *more* than the alpha
    // test did.
    let mut solid_model = masked_model.clone();
    solid_model.materials[1].alpha_mode = AlphaMode::Opaque;
    let solid = ModelBatch::new(
        context,
        &solid_model,
        &placement,
        harness.deferred.material_layout(),
    )
    .expect("uncut batch");
    assert!(
        solid.has_cutout(),
        "still two-sided, so still the same path"
    );

    let frame = frame_with_low_sun(&harness.terrain);
    let cut = render(context, &harness, std::slice::from_ref(&masked), frame);
    let uncut = render(context, &harness, std::slice::from_ref(&solid), frame);
    write_capture("model-foliage-masked.png", &cut);
    write_capture("model-foliage-solid.png", &uncut);
    support::check_reference(context, "model-foliage-masked.png", &cut);

    // Canopy area: the cut must remove a substantial and *predictable* share of it. The texture's alpha
    // is a disc of radius 0.44 in a unit quad, so it keeps pi times 0.44 squared -- about 61% -- and the
    // masked frame must therefore show something near two fifths less canopy. A range rather than a
    // threshold, because both a test that never fired and a test that discarded everything would pass a
    // one-sided one.
    let count = |capture: &Capture| {
        capture
            .rgba()
            .chunks_exact(4)
            .filter(|pixel| is_canopy(pixel))
            .count()
    };
    let (through, blocked) = (count(&cut), count(&uncut));
    let kept = through as f64 / blocked as f64;
    eprintln!("canopy pixels: masked {through}, solid {blocked}, kept {kept:.3}");
    assert!(
        (0.45..=0.78).contains(&kept),
        "a disc of radius 0.44 keeps about 0.61 of its quad, but {kept:.3} survived"
    );

    // And the ground is lighter, because the shadow the canopy casts is now perforated rather than
    // solid. Measured over the lit part of the frame: a cascade still casting the full rectangle would
    // leave this unchanged even with the lit silhouette correct, which is exactly the half-done state
    // worth catching.
    let perforated = mean_channel_sum(&cut, |pixel| !is_sky(pixel));
    let slab = mean_channel_sum(&uncut, |pixel| !is_sky(pixel));
    eprintln!("mean lit channel sum: masked {perforated}, solid {slab}");
    assert!(
        perforated > slab,
        "a perforated shadow must leave the ground lighter: {perforated} against {slab}"
    );
}

#[test]
fn sway_moves_a_canopy_over_time_and_leaves_it_still_in_calm_air() {
    // Three properties in one place, because they are only meaningful together: sway moves geometry,
    // sway is *reproducible* at a given time, and still air is a genuine no-op rather than a small
    // displacement. The third is what keeps every committed reference valid — the whole existing set was
    // rendered in calm air.
    let Some(context) = context() else { return };
    let harness = harness(context);
    let model = foliage_model(70.0, 110.0, 40.0);

    let rigid = ModelBatch::new(
        context,
        &model,
        &plantings(&harness.terrain, None),
        harness.deferred.material_layout(),
    )
    .expect("rigid batch");
    let swaying = ModelBatch::new(
        context,
        &model,
        &plantings(&harness.terrain, Some(SwayProfile::GRASS)),
        harness.deferred.material_layout(),
    )
    .expect("swaying batch");

    let calm = frame_with_low_sun(&harness.terrain);
    let mut windy = calm;
    windy.environment.weather.wind = [12.0, 4.0];

    // Still air: the swaying batch must render identically at two different times, byte for byte. A sway
    // that displaced anything at zero wind would have invalidated every reference in the tree, all of
    // which were rendered in calm air.
    //
    // Compared against *itself* at another time rather than against the rigid batch, which would fail for
    // a reason that is not a bug: a swaying batch reports a taller caster than a rigid one — see
    // `ModelInstance::sway_headroom` — so its shadow cascades are fitted to what it *can* do rather than
    // to what the wind is doing this frame, and the fitted matrices differ by a hair in calm air too.
    let still_rigid = render(context, &harness, std::slice::from_ref(&rigid), calm);
    let still_swaying = render(context, &harness, std::slice::from_ref(&swaying), calm);
    let still_swaying_later = render(
        context,
        &harness,
        std::slice::from_ref(&swaying),
        calm.at_time(7.5),
    );
    assert_eq!(
        still_swaying.rgba(),
        still_swaying_later.rgba(),
        "sway in calm air must be exactly nothing, at any time"
    );

    // Wind, at two times. Both must differ from the still frame and from each other.
    let early = render(
        context,
        &harness,
        std::slice::from_ref(&swaying),
        windy.at_time(1.0),
    );
    let later = render(
        context,
        &harness,
        std::slice::from_ref(&swaying),
        windy.at_time(1.9),
    );
    write_capture("model-sway-early.png", &early);
    write_capture("model-sway-later.png", &later);
    support::check_reference(context, "model-sway-early.png", &early);

    assert_ne!(
        still_swaying.rgba(),
        early.rgba(),
        "wind must displace the canopy"
    );
    assert_ne!(
        early.rgba(),
        later.rgba(),
        "the canopy must move between two times"
    );

    // Reproducible: the same time gives the same frame. Nothing in the renderer may read a clock.
    let again = render(
        context,
        &harness,
        std::slice::from_ref(&swaying),
        windy.at_time(1.0),
    );
    assert_eq!(
        early.rgba(),
        again.rgba(),
        "the same scene time must give the same frame"
    );

    // And a rigid batch is unaffected by wind, so the displacement comes from the profile rather than
    // from the wind reaching every vertex.
    let rigid_in_wind = render(
        context,
        &harness,
        std::slice::from_ref(&rigid),
        windy.at_time(1.0),
    );
    assert_eq!(
        still_rigid.rgba(),
        rigid_in_wind.rgba(),
        "wind must not move a rigid instance"
    );
}
