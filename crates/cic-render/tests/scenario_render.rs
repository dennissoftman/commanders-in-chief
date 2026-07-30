//! An activated scenario drawn through the deferred chain: the first time kernel state is visible.
//!
//! The pipeline under test is presentation reading a simulation snapshot — `Forces` objects grouped
//! by template, each group becoming one instanced batch, owners becoming tints — with the kernel
//! advanced not at all, because a placed object has no behaviour yet. Captures land in
//! `CARGO_TARGET_TMPDIR`; assertions are a tripwire and the image is the verification.
//!
//! Deliberately **no committed reference**: model rendering is already regression-covered by the
//! model scenes, and what this test adds is the snapshot-to-instances translation, whose correctness
//! is the counts and groupings asserted below plus a pair of eyes on the capture. A reference would
//! buy a lavapipe regeneration cycle for nothing those assertions do not already say.

// The shared harness support compiles once per test binary, and this is the one binary that uses its
// device sharing without its reference comparison — deliberately, per the module docs above.
#[allow(dead_code)]
mod support;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use cic_assets::Terrain;
use cic_assets::model::{Model, ModelMaterial, ModelPrimitive, ModelVertex};
use cic_assets::scenario::{ObjectPlacement, PlayerSlot, Position, Scenario, TerrainReference};
use cic_assets::templates::{Template, TemplateKind, TemplateSet};
use cic_camera::CameraPose;
use cic_render::{
    Capture, CaptureTarget, DeferredFrame, DeferredRenderer, DeferredTargets, GpuContext,
    ModelBatch, ModelInstance, TerrainRenderer,
};
use cic_sim::activation::FORCES;
use cic_sim::{Forces, Kernel, KernelConfig, ObjectId, Placed, activate};

const WIDTH: u32 = 720;
const HEIGHT: u32 = 480;
const SAMPLES: u32 = 129;
const SPACING: f32 = 8.0;

static CONTEXT: OnceLock<Option<GpuContext>> = OnceLock::new();

fn context() -> Option<&'static GpuContext> {
    CONTEXT.get_or_init(support::shared_context).as_ref()
}

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

/// A one-material box centred on its base: enough silhouette to see a placement and its tint.
///
/// No bottom face — it sits on the ground — and no top-UV care, because what this test looks at is
/// where boxes stand and what colour they took, not their texturing, which the model scenes cover.
fn box_model(size: f32, height: f32) -> Model {
    let half = size * 0.5;
    let faces: [([f32; 3], [[f32; 3]; 4]); 5] = [
        (
            [0.0, 0.0, 1.0],
            [
                [-half, -half, height],
                [half, -half, height],
                [half, half, height],
                [-half, half, height],
            ],
        ),
        (
            [0.0, -1.0, 0.0],
            [
                [-half, -half, 0.0],
                [half, -half, 0.0],
                [half, -half, height],
                [-half, -half, height],
            ],
        ),
        (
            [0.0, 1.0, 0.0],
            [
                [half, half, 0.0],
                [-half, half, 0.0],
                [-half, half, height],
                [half, half, height],
            ],
        ),
        (
            [1.0, 0.0, 0.0],
            [
                [half, -half, 0.0],
                [half, half, 0.0],
                [half, half, height],
                [half, -half, height],
            ],
        ),
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
    let primitives = faces
        .into_iter()
        .map(|(normal, corners)| {
            let vertices = corners
                .into_iter()
                .map(|position| ModelVertex {
                    position,
                    normal,
                    ..ModelVertex::default()
                })
                .collect();
            ModelPrimitive {
                vertices,
                indices: vec![0, 1, 2, 0, 2, 3],
                material: Some(0),
            }
            .with_generated_tangents()
        })
        .collect();
    Model {
        name: "placed box".to_owned(),
        primitives,
        materials: vec![ModelMaterial {
            name: "shell".to_owned(),
            base_color: [0.72, 0.68, 0.60, 1.0],
            metallic: 0.0,
            roughness: 0.85,
            ..ModelMaterial::default()
        }],
        images: Vec::new(),
        has_animation: false,
        has_skin: false,
    }
}

/// The demo map: two players with starts, a depot each, and neutral pines between them.
fn demo(terrain: &Terrain) -> (Scenario, TemplateSet) {
    let [extent_x, extent_y] = terrain.world_extent();
    let position = |fx: f32, fy: f32| Position {
        x: extent_x * fx,
        y: extent_y * fy,
        z: 0.0,
    };
    let player = |id: &str, team: u32, fx: f32, fy: f32| PlayerSlot {
        id: id.to_owned(),
        name: id.to_owned(),
        faction: "faction/vanguard".to_owned(),
        start: position(fx, fy),
        team,
    };
    let place =
        |template: &str, owner: Option<&str>, fx: f32, fy: f32, rotation: f32| ObjectPlacement {
            template: template.to_owned(),
            position: position(fx, fy),
            rotation,
            scale: 1.0,
            owner: owner.map(str::to_owned),
        };
    let scenario = Scenario {
        format_version: 1,
        name: "Activation demo".to_owned(),
        description: String::new(),
        terrain: TerrainReference {
            path: "terrain/demo.cict".to_owned(),
        },
        players: vec![
            player("north", 1, 0.30, 0.72),
            player("south", 2, 0.70, 0.28),
        ],
        objects: vec![
            place("structure/depot", Some("north"), 0.36, 0.68, 0.0),
            place("structure/depot", Some("south"), 0.64, 0.32, 45.0),
            place("prop/pine", None, 0.46, 0.52, 0.0),
            place("prop/pine", None, 0.52, 0.47, 120.0),
            place("prop/pine", None, 0.57, 0.55, 240.0),
        ],
        waypoints: Vec::new(),
        scripts: Vec::new(),
    };
    let template = |id: &str, kind, model: Option<&str>| Template {
        id: id.to_owned(),
        kind,
        model: model.map(str::to_owned),
        name: None,
        speed: None,
    };
    let templates = TemplateSet {
        format_version: 1,
        templates: vec![
            template(
                "structure/depot",
                TemplateKind::Structure,
                Some("models/depot.glb"),
            ),
            template("prop/pine", TemplateKind::Prop, Some("models/pine.glb")),
            template("faction/vanguard", TemplateKind::Faction, None),
        ],
    };
    (scenario, templates)
}

/// The snapshot-to-instances translation a drawing host performs: group by template, ground each
/// object on the terrain, tint by owner.
fn instances_by_template(
    forces: &Forces,
    terrain: &Terrain,
) -> BTreeMap<String, Vec<ModelInstance>> {
    const TEAM_TINTS: [[f32; 4]; 2] = [[0.55, 0.70, 1.0, 1.0], [1.0, 0.62, 0.42, 1.0]];
    let mut grouped: BTreeMap<String, Vec<ModelInstance>> = BTreeMap::new();
    for placed in forces.objects().values() {
        let instance = instance_for(placed, terrain);
        let tinted = match placed.owner {
            Some(owner) => instance.with_tint(TEAM_TINTS[usize::from(owner.0) % TEAM_TINTS.len()]),
            None => instance,
        };
        grouped
            .entry(placed.template.clone())
            .or_default()
            .push(tinted);
    }
    grouped
}

/// One placed object as a model instance: presentation narrows simulation state freely, because
/// nothing here feeds back into it.
#[expect(
    clippy::cast_possible_truncation,
    reason = "presentation narrows simulation state freely; nothing feeds back"
)]
fn instance_for(placed: &Placed, terrain: &Terrain) -> ModelInstance {
    let [x, y] = [placed.position[0] as f32, placed.position[1] as f32];
    let ground = terrain.elevation_at_world(x, y).unwrap_or(0.0);
    let radians = (f64::from(placed.rotation) / 4_294_967_296.0 * std::f64::consts::TAU) as f32;
    ModelInstance::placed([x, y, ground], radians, placed.scale as f32)
}

fn capture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
}

fn write_capture(name: &str, capture: &Capture) {
    let path = capture_dir().join(name);
    std::fs::write(&path, capture.png().expect("encode png")).expect("write png");
    eprintln!("wrote {}", path.display());
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

struct Harness {
    renderer: TerrainRenderer,
    deferred: DeferredRenderer,
    targets: DeferredTargets,
    output: CaptureTarget,
}

fn harness(context: &GpuContext, terrain: &Terrain) -> Harness {
    let renderer = TerrainRenderer::new(context, terrain, &[]).expect("terrain renderer");
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
    let encoder = context
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("test resolve"),
        });
    harness.output.resolve(context, encoder).expect("resolve")
}

#[test]
fn an_activated_scenario_is_drawn_from_its_snapshot() {
    let Some(context) = context() else { return };
    let terrain = flat_terrain();

    // Activate: the same path a real host takes, ending in a snapshot presentation reads.
    let (scenario, templates) = demo(&terrain);
    let mut kernel = Kernel::new(KernelConfig {
        seed: 3,
        ticks_per_second: 30,
    });
    activate(&mut kernel, &scenario, &templates).expect("the demo scenario activates");
    let forces = kernel
        .subsystem(FORCES)
        .and_then(|subsystem| subsystem.as_any().downcast_ref::<Forces>())
        .expect("forces registered");

    // Translate: group by template, tint by owner. Two placeable templates, so two batches; the
    // depot batch has an instance per player and the pine batch the three neutrals.
    let grouped = instances_by_template(forces, &terrain);
    assert_eq!(grouped.len(), 2, "two placeable templates were declared");
    assert_eq!(grouped["structure/depot"].len(), 2);
    assert_eq!(grouped["prop/pine"].len(), 3);
    assert_eq!(
        forces.objects().len(),
        grouped.values().map(Vec::len).sum::<usize>(),
        "every constructed object is drawn exactly once"
    );
    // Authored order: the first declared placement took ObjectId(1).
    assert!(forces.objects().contains_key(&ObjectId(1)));

    let harness = harness(context, &terrain);
    let models: Vec<ModelBatch> = [
        ("structure/depot", box_model(36.0, 52.0)),
        ("prop/pine", box_model(10.0, 34.0)),
    ]
    .into_iter()
    .map(|(template, model)| {
        ModelBatch::new(
            context,
            &model,
            &grouped[template],
            harness.deferred.material_layout(),
        )
        .expect("upload batch")
    })
    .collect();

    let frame = DeferredFrame::new(pose(&terrain), WIDTH, HEIGHT);
    let with = render(context, &harness, &models, frame);
    write_capture("scenario-activated.png", &with);
    let without = render(context, &harness, &[], frame);

    let differing = with
        .rgba()
        .chunks_exact(4)
        .zip(without.rgba().chunks_exact(4))
        .filter(|(ours, theirs)| ours[0..3] != theirs[0..3])
        .count();
    assert!(
        differing > 500,
        "the activated objects must be visible: only {differing} pixels changed"
    );
}
