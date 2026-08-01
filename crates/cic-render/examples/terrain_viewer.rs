//! An interactive terrain viewer.
//!
//! ```text
//! cargo run -p cic-render --example terrain_viewer --release
//! cargo run -p cic-render --example terrain_viewer --release -- path/to/map.cicmap
//! cargo run -p cic-render --example terrain_viewer --release -- sky/kloofendal.hdr
//! ```
//!
//! With no argument it generates a terrain, so the viewer runs before any content exists. An
//! argument ending in `.hdr` is read as an equirectangular sky rather than as a map, so the two can
//! be given in either order and neither needs a flag.
//!
//! # Controls
//!
//! | Input | Action |
//! |---|---|
//! | `W` `A` `S` `D`, arrows | Pan |
//! | Right or middle drag | Pan |
//! | Wheel | Zoom |
//! | `Q` `E` | Rotate |
//! | `R` | Reset height and rotation |
//! | `F` | Reset rotation only |
//! | `T` | Cycle antialiasing: none, post pass, temporal |
//! | `[` `]` | Step the resolution scale |
//! | `G` | Cycle weather: clear, rain, thunderstorm, snowfall |
//! | `,` `.` | Step the time of day, held to scrub the sun |
//! | `P` | Toggle the per-pass GPU timing printout |
//! | `V` | Toggle the virtual-texture page cache |
//! | `J` | Cycle the water: lake, river, ocean |
//! | `K` | Toggle the captured sky, when one was given |
//! | `Esc` | Quit |
//!
//! Antialiasing, the resolution scale, the weather and the hour are all here for one reason: each is a
//! rendering change a still capture reports badly or not at all. Antialiasing's whole subject is what an
//! edge does *as the camera moves*; a resolution scale trades frame rate for sampling rate; and the
//! environment terms move on their own — cloud shadows drift at a rate the wind sets, and the sun
//! travels. Each prints what it took effect as, so a screenshot of the terminal says what the window is
//! showing.
//!
//! The temporal tier is the strongest case for that. A converged capture of it is a reference the harness
//! can compare, and it says nothing at all about the two things that decide whether the setting is usable:
//! whether a pan smears, and whether a stationary camera settles or shimmers.
//!
//! The weather keys close a gap of the same kind. Cloud shadows, wetness and lying snow reach the shaders
//! only through an environment, and this viewer set none — so those three terms were reachable from a
//! test capture and from nowhere a person could look at them moving.
//!
//! `V` closes the last one. The virtual-texture cache is a filter with a residency policy attached, and all
//! three of its interesting failures are motion artefacts: a seam crawling along a page boundary as the
//! camera pans, a visible step between mip levels, and a page arriving a frame after the ground it covers.
//! None of them is in a still. It also shows the cache running out of slots on a large map, which is the
//! honest state of the residency request today — the ground it loses falls back to the direct blend, and
//! where that boundary sits is what a view-driven request has to fix.
//!
//! `K` is here for a reason of the same shape and a stronger one. A captured sky is *rotated* to put its
//! own sun where the scene's light is, and whether that rotation is right is a question about two things
//! in different parts of the frame -- a bright patch of sky, and the direction every shadow falls. A
//! still shows both and invites the eye to accept them; scrubbing the hour with `,` and `.` moves the
//! sky and the shadows together, and a disagreement is then impossible to miss. Toggling the sky off and
//! on is the other half: it is the only way to see what the environment is contributing to the ambient
//! and the fog, which are changes to the *ground* rather than to the sky.

// The generator clamps before converting and its inputs are bounded constants, so the width casts
// below cannot lose anything.
// The geometry builders here are tables of corner coordinates. Splitting one to satisfy a line count would
// put half a cube in one function and half in another, which is less readable rather than more.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::too_many_lines
)]

use std::collections::BTreeMap;
use std::error::Error;
use std::sync::Arc;
use std::time::Instant;

use cic_assets::model::{Model, ModelImage, ModelMaterial, ModelPrimitive, ModelVertex};
use cic_assets::scenario::{ObjectPlacement, PlayerSlot, Position, Scenario, TerrainReference};
use cic_assets::sky::{SkyAsset, SkyLimits, decode_radiance};
use cic_assets::templates::{Footprint, Template, TemplateKind, TemplateSet};
use cic_assets::texture::{TextureAsset, TextureLimits};
use cic_assets::{MapPackage, PackageLimits, Terrain, TerrainLayer, resolve_terrain_textures};
use cic_camera::{RtsCamera, RtsCameraProfile};
use cic_render::detail::TerrainDetailRequest;
use cic_render::display::{MAX_RESOLUTION_SCALE, MIN_RESOLUTION_SCALE};
use cic_render::sky::{Sky, SkySettings};
use cic_render::terrain_virtual::{VIRTUAL_PAGE_LAYERS, VirtualPageView};
use cic_render::view::Projection;
use cic_render::{
    Action, Antialiasing, DeferredFrame, DisplaySettings, Environment, GpuContext, InputState,
    LayerMaterial, ModelBatch, ModelInstance, SurfaceRenderer, TerrainGround, TerrainPageCache,
    TerrainRenderer, TextureImage, WaterBody, WaterKind, WaterSurface, Weather,
};
use cic_sim::activation::FORCES;
use cic_sim::units::UNITS;
use cic_sim::{
    Command, Forces, Ground, GroundRules, Kernel, KernelConfig, ObjectId, PlayerId,
    TickAccumulator, Units, activate, move_group_facing_command, spawn_command,
};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

/// The weather states `G` cycles, in the order it cycles them.
///
/// Presets rather than a slider per field, because the interesting thing about weather here is that its
/// fields are *not* independent — [`Environment::with_weather`] derives fog density and cloud coverage
/// from the overcast it is given, and a preset exercises that derivation the way a caller would. The
/// clear state comes first so the viewer opens in the state every committed reference was captured in.
const WEATHER_CYCLE: [WeatherPreset; 4] = [
    WeatherPreset::new("clear", Weather::default),
    WeatherPreset::new("rain", Weather::rain),
    WeatherPreset::new("thunderstorm", Weather::thunderstorm),
    WeatherPreset::new("snowfall", Weather::snowfall),
];

/// One entry of [`WEATHER_CYCLE`]: what to call it, and how to build it.
struct WeatherPreset {
    name: &'static str,
    build: fn() -> Weather,
}

impl WeatherPreset {
    const fn new(name: &'static str, build: fn() -> Weather) -> Self {
        Self { name, build }
    }
}

/// Hours the time-of-day keys step by.
///
/// A quarter hour, because the sun's elevation is what shadow length follows and near dawn or dusk a
/// whole hour moves it far enough to skip the angles where cascade fitting is most strained.
const HOUR_STEP: f32 = 0.25;

/// World units a one-pixel drag pans at the default height.
const DRAG_PIXELS_TO_UNITS: f32 = 0.05;

/// Longest frame step applied to the camera.
///
/// A stall — a breakpoint, a window drag, a shader recompile — otherwise arrives as one enormous
/// delta and flings the camera across the map before the first frame after it is drawn.
const MAXIMUM_FRAME_SECONDS: f32 = 0.1;

/// How much one press of `[` or `]` moves the resolution scale.
///
/// A quarter, so the offered range is six steps rather than a continuum. A finer step would let someone
/// walk to a cost they cannot afford one imperceptible increment at a time.
const RESOLUTION_SCALE_STEP: f32 = 0.25;

/// How often the per-pass breakdown is read back while timing is on.
///
/// Once a second, because reading it *blocks until the GPU has finished the frame*. Every frame would
/// serialise the CPU against the GPU and change the numbers being measured, which is the trap a
/// profiler-shaped diagnostic invites.
const TIMING_REPORT_SECONDS: f32 = 1.0;

fn main() -> Result<(), Box<dyn Error>> {
    // Split by extension rather than by position, so the two arguments can be given in either order and
    // neither needs a flag. A viewer is a tool, and a tool that makes someone remember which slot the
    // sky goes in is a tool with a worse interface than the one-line match below deserves.
    let mut map_path = None;
    let mut sky_path = None;
    for argument in std::env::args().skip(1) {
        if argument.to_ascii_lowercase().ends_with(".hdr") {
            sky_path = Some(argument);
        } else {
            map_path = Some(argument);
        }
    }
    let sky = match sky_path {
        Some(path) => Some(load_sky(&path)?),
        None => None,
    };

    let (terrain, layer_textures, activation) = if let Some(path) = map_path {
        // A loaded package carries a scenario but no template set yet, so its placements stay
        // undrawn until packages do — the demo path below is what that will reuse when they do.
        let (terrain, textures) = load_package(&path)?;
        (terrain, textures, None)
    } else {
        eprintln!("no map given; generating a terrain and a scenario to activate on it");
        let terrain = generated_terrain();
        let activation = demo_scenario(&terrain);
        // A generated terrain has no package to carry textures, so its layers use the procedural
        // surfaces.
        (terrain, Vec::new(), Some(activation))
    };
    eprintln!(
        "terrain {}x{} samples, {:?} world units, peak {:.0}",
        terrain.width(),
        terrain.height(),
        terrain.world_extent(),
        highest_elevation(&terrain),
    );

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = Viewer::new(terrain, layer_textures, activation, sky);
    event_loop.run_app(&mut app)?;
    if let Some(error) = app.failure {
        return Err(error.into());
    }
    Ok(())
}

/// Reads an equirectangular sky and reports what it will contribute.
///
/// Decoded here, at startup, rather than when the window appears. A malformed file is then a message
/// before anything opens, instead of a black sky in a window that gives no reason for it.
///
/// The printed figures are the point of printing anything: `intensity` is the one setting a captured
/// environment usually needs turned, and the ambient it derives is what says whether it needs turning.
/// An HDRI whose ambient comes out at 4.0 will wash the scene out, and the number says so before the
/// frame does.
fn load_sky(path: &str) -> Result<SkyAsset, Box<dyn Error>> {
    let bytes = std::fs::read(path)?;
    let sky = decode_radiance(&bytes, SkyLimits::default())?;
    let lighting = sky.lighting();
    eprintln!(
        "sky: {}x{} from {path}, horizon {:?}, ambient {:?}",
        sky.width(),
        sky.height(),
        lighting
            .horizon
            .map(|value| (value * 100.0).round() / 100.0),
        lighting
            .ambient
            .map(|value| (value * 100.0).round() / 100.0),
    );
    Ok(sky)
}

/// A terrain and the block-compressed texture, if any, each of its layers is surfaced with.
type LoadedTerrain = (Terrain, Vec<Option<TextureAsset>>);

/// Opens a map package and returns its terrain and whatever layer textures it carries.
///
/// The textures are resolved here rather than later because this is the only place holding the package,
/// and the package owns the mount they are read through. A map with no `textures/` directory yields an
/// empty set and the layers fall back to the procedural surfaces below — which is what every map in the
/// tree does today, since none has been authored with converted textures yet.
fn load_package(path: &str) -> Result<LoadedTerrain, Box<dyn Error>> {
    let bytes = std::fs::read(path)?;
    let package = MapPackage::open(&bytes, PackageLimits::default())?;
    let textures = resolve_terrain_textures(
        package.terrain(),
        package.contents(),
        TextureLimits::default(),
    )?;
    let converted = textures.iter().filter(|texture| texture.is_some()).count();
    eprintln!(
        "loaded {} from {path}: {} layers, {converted} with a block-compressed texture",
        package.scenario().name,
        package.terrain().layers().len(),
    );
    Ok((package.terrain().clone(), textures))
}

/// A terrain with a ridge, a spire, and a bowl, so lighting and shadows have something to work on.
fn generated_terrain() -> Terrain {
    const SAMPLES: u32 = 257;
    let last = f64::from(SAMPLES - 1);
    let mut elevations = Vec::with_capacity((SAMPLES * SAMPLES) as usize);
    for y in 0..SAMPLES {
        for x in 0..SAMPLES {
            let fx = f64::from(x) / last;
            let fy = f64::from(y) / last;
            let ridge = 520.0 * (-((fy - 0.66).powi(2)) / 0.0009).exp();
            let spire = 820.0 * (-((fx - 0.30).powi(2) + (fy - 0.30).powi(2)) / 0.0007).exp();
            let bowl = -160.0 * (-((fx - 0.70).powi(2) + (fy - 0.36).powi(2)) / 0.008).exp();
            let hills = 90.0 * (-((fx - 0.55).powi(2) + (fy - 0.80).powi(2)) / 0.02).exp();
            let undulation =
                34.0 * ((fx * 7.3).sin() * (fy * 5.1).cos() + 0.5 * (fx * 13.0 + fy * 9.0).sin());
            let elevation = 220.0 + undulation + ridge + spire + bowl + hills;
            elevations.push(elevation.round().clamp(0.0, 65_535.0) as u16);
        }
    }

    let ramp = |value: f32, edge_a: f32, edge_b: f32| {
        let t = ((value - edge_a) / (edge_b - edge_a)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let mut sand = Vec::with_capacity(elevations.len());
    let mut grass = Vec::with_capacity(elevations.len());
    let mut rock = Vec::with_capacity(elevations.len());
    for elevation in &elevations {
        let height = f32::from(*elevation);
        let above_sand = ramp(height, 190.0, 260.0);
        let into_rock = ramp(height, 430.0, 620.0);
        sand.push(((1.0 - above_sand) * 255.0).round() as u8);
        grass.push((above_sand * (1.0 - into_rock) * 255.0).round() as u8);
        rock.push((into_rock * 255.0).round() as u8);
    }

    Terrain::new(
        SAMPLES,
        SAMPLES,
        8.0,
        0.5,
        elevations,
        vec![
            TerrainLayer {
                name: "sand".to_owned(),
                weights: sand,
            },
            TerrainLayer {
                name: "grass".to_owned(),
                weights: grass,
            },
            TerrainLayer {
                name: "rock".to_owned(),
                weights: rock,
            },
        ],
    )
    .expect("generated terrain is valid")
}

/// A simple building: a box with a distinct roof material.
///
/// Built in Rust rather than loaded, so the viewer runs with no asset files at all. The importer that
/// would replace this is already tested in `cic-assets`; what this exercises is rendering.
fn building_model() -> Model {
    let half = 18.0f32;
    let height = 46.0f32;
    let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
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
            [0.0, 0.0, -1.0],
            [
                [-half, half, 0.0],
                [half, half, 0.0],
                [half, -half, 0.0],
                [-half, -half, 0.0],
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
        .enumerate()
        .map(|(index, (normal, corners))| {
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
                material: Some(usize::from(index == 0)),
            }
            // Built in Rust, so the importer's tangent derivation never ran. See `model_render.rs`.
            .with_generated_tangents()
        })
        .collect();

    Model {
        name: "building".to_owned(),
        primitives,
        materials: vec![
            ModelMaterial {
                name: "wall".to_owned(),
                base_color: [0.66, 0.62, 0.55, 1.0],
                metallic: 0.0,
                roughness: 0.85,
                base_color_texture: Some(0),
                ..ModelMaterial::default()
            },
            ModelMaterial {
                name: "roof".to_owned(),
                base_color: [0.36, 0.20, 0.16, 1.0],
                metallic: 0.0,
                roughness: 0.65,
                base_color_texture: Some(1),
                ..ModelMaterial::default()
            },
        ],
        images: vec![wall_image(), roof_image()],
        has_skin: false,
        has_animation: false,
    }
}

/// Texture coordinates for one corner of a quad whose corners run anticlockwise from `[-, -]`.
///
/// Not `[corner & 1, corner >> 1]`: that walks the unit square in Z order while the corners walk it
/// in a ring, so the last two swap and the texture arrives sheared along a diagonal.
fn quad_uv(corner: usize) -> [f32; 2] {
    const RING: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    RING[corner % 4]
}

/// Rows of windows over rendered concrete, generated rather than loaded.
///
/// Both this and the terrain layers below are procedural for the same reason the geometry is: the
/// viewer must run with no asset files at all, and what it is here to exercise is the renderer.
fn wall_image() -> ModelImage {
    const SIZE: u32 = 128;
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let in_window = (y % 32) >= 8 && (y % 32) < 22 && (x % 24) >= 5 && (x % 24) < 19;
            let grain = noise(x, y, 11) * 0.14;
            let colour = if in_window {
                [0.10 + grain * 0.4, 0.13 + grain * 0.4, 0.17 + grain * 0.4]
            } else {
                [0.70 - grain, 0.67 - grain, 0.61 - grain]
            };
            push_srgb(&mut rgba, colour);
        }
    }
    ModelImage {
        // Unnamed, so no block-compressed sidecar is looked up for it.
        name: String::new(),
        width: SIZE,
        height: SIZE,
        rgba,
    }
}

/// Corrugated roofing: ribs along one axis, with rust breaking them up.
fn roof_image() -> ModelImage {
    const SIZE: u32 = 128;
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let rib = if (x / 6) % 2 == 0 { 0.0 } else { 0.18 };
            let rust = noise(x, y, 29).powi(2) * 0.5;
            push_srgb(
                &mut rgba,
                [
                    0.42 + rib + rust * 0.35,
                    0.30 + rib + rust * 0.10,
                    0.26 + rib,
                ],
            );
        }
    }
    ModelImage {
        // Unnamed, so no block-compressed sidecar is looked up for it.
        name: String::new(),
        width: SIZE,
        height: SIZE,
        rgba,
    }
}

/// Terrain layer surfaces, one per weight layer, at the world scale each tiles at.
///
/// A layer with a converted texture in the package uses it, at the scale the procedural stand-in was
/// tiling at; a layer without one keeps the stand-in. That is the ordinary mixed state of content being
/// converted a texture at a time, and the renderer resolves it per array — see
/// `TerrainRenderer::with_materials`.
fn layer_materials(terrain: &Terrain, textures: &[Option<TextureAsset>]) -> Vec<LayerMaterial> {
    let procedural = procedural_layer_materials();
    let count = terrain.layers().len().max(procedural.len());
    (0..count)
        .map(|index| {
            let base = procedural.get(index).cloned().unwrap_or_default();
            match textures.get(index).and_then(Option::as_ref) {
                Some(texture) => {
                    let scale = base.detail_scale;
                    base.with_compressed_albedo(texture.clone(), scale)
                }
                None => base,
            }
        })
        .collect()
}

/// The generated surfaces a layer falls back to when the package carries no texture for it.
fn procedural_layer_materials() -> Vec<LayerMaterial> {
    vec![
        LayerMaterial::colour([1.0; 3])
            .with_albedo(grain_image(96, [0.78, 0.70, 0.49], 0.22, 7), 14.0)
            .with_roughness(0.94),
        LayerMaterial::colour([1.0; 3])
            .with_albedo(
                clumped_image(96, [0.15, 0.26, 0.10], [0.44, 0.54, 0.24], 13),
                20.0,
            )
            .with_roughness(0.90),
        LayerMaterial::colour([1.0; 3])
            .with_albedo(
                clumped_image(96, [0.26, 0.25, 0.24], [0.62, 0.60, 0.56], 23),
                30.0,
            )
            .with_roughness(0.78),
    ]
}

/// Fine even grain, for sand.
fn grain_image(size: u32, base: [f32; 3], spread: f32, seed: u32) -> TextureImage {
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let shade = (noise(x, y, seed) - 0.5) * 2.0 * spread;
            push_srgb(
                &mut rgba,
                [base[0] + shade, base[1] + shade, base[2] + shade],
            );
        }
    }
    TextureImage::new(size, size, rgba).expect("generated layer image is valid")
}

/// Two colours in soft patches, for grass and rock.
fn clumped_image(size: u32, low: [f32; 3], high: [f32; 3], seed: u32) -> TextureImage {
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            // Two frequencies, so the result has both patches and a grain inside them rather than a
            // single scale that reads as uniform static at any distance.
            let coarse = noise(x / 8, y / 8, seed);
            let fine = noise(x, y, seed.wrapping_add(1));
            let mix = (coarse * 0.75 + fine * 0.25).clamp(0.0, 1.0);
            push_srgb(
                &mut rgba,
                [
                    low[0] + (high[0] - low[0]) * mix,
                    low[1] + (high[1] - low[1]) * mix,
                    low[2] + (high[2] - low[2]) * mix,
                ],
            );
        }
    }
    TextureImage::new(size, size, rgba).expect("generated layer image is valid")
}

/// A deterministic hash in `0..=1`. Not good noise; good enough to break up a flat colour, and it
/// costs no dependency and no asset file.
fn noise(x: u32, y: u32, seed: u32) -> f32 {
    let mut value = x
        .wrapping_mul(374_761_393)
        .wrapping_add(y.wrapping_mul(668_265_263))
        .wrapping_add(seed.wrapping_mul(2_246_822_519));
    value ^= value >> 13;
    value = value.wrapping_mul(1_274_126_177);
    value ^= value >> 16;
    f32::from(value as u16) / f32::from(u16::MAX)
}

/// Appends one linear colour as sRGB-encoded opaque RGBA.
fn push_srgb(rgba: &mut Vec<u8>, colour: [f32; 3]) {
    for channel in colour {
        let clamped = channel.clamp(0.0, 1.0);
        let encoded = if clamped <= 0.003_130_8 {
            clamped * 12.92
        } else {
            1.055 * clamped.powf(1.0 / 2.4) - 0.055
        };
        rgba.push((encoded * 255.0 + 0.5) as u8);
    }
    rgba.push(u8::MAX);
}

/// Scatters buildings across the terrain, each sitting on the ground beneath it.
///
/// Placements follow the heightfield rather than a fixed Z, which is the same lookup the camera uses to
/// hold its height -- and the reason a building on a slope does not float or sink.
fn building_placements(terrain: &Terrain) -> Vec<ModelInstance> {
    let [extent_x, extent_y] = terrain.world_extent();
    let mut placed = Vec::new();
    for row in 0..6u16 {
        for column in 0..6u16 {
            let x = extent_x * (0.10 + 0.16 * f32::from(column));
            let y = extent_y * (0.06 + 0.13 * f32::from(row));
            // Skip anything the terrain does not cover, rather than placing it at zero.
            let Some(ground) = terrain.elevation_at_world(x, y) else {
                continue;
            };
            // Leave the steepest ground clear, so buildings do not obviously intersect a slope.
            let slope_probe = terrain.elevation_at_world(x + 20.0, y).unwrap_or(ground);
            if (slope_probe - ground).abs() > 14.0 {
                continue;
            }
            let rotation = 0.41 * f32::from(row * 6 + column);
            let instance =
                ModelInstance::placed([x, y, ground], rotation, 0.8 + 0.05 * f32::from(column));
            placed.push(if (row + column) % 4 == 0 {
                instance.with_tint([0.62, 0.74, 0.86, 1.0])
            } else {
                instance
            });
        }
    }
    placed
}

/// A pine stand-in: a tall thin box in one green material.
///
/// Built in Rust for the same reason the building is — the viewer runs with no asset files — and it
/// is a placeholder shape by design: the template's `model` names a `.glb`, and a real tree replaces
/// this the day one exists in content.
fn pine_model() -> Model {
    let half = 4.5f32;
    let height = 34.0f32;
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
                material: Some(0),
            }
            .with_generated_tangents()
        })
        .collect();
    Model {
        name: "pine".to_owned(),
        primitives,
        materials: vec![ModelMaterial {
            name: "canopy".to_owned(),
            base_color: [0.18, 0.34, 0.16, 1.0],
            metallic: 0.0,
            roughness: 0.95,
            ..ModelMaterial::default()
        }],
        images: Vec::new(),
        has_skin: false,
        has_animation: false,
    }
}

/// The demo map the generated mode activates: two players, a depot each, and neutral pines.
///
/// The same shape a `.cicmap` will carry once packages hold a template set: a scenario with owned and
/// neutral placements, and the set their `template:` names resolve against.
fn demo_scenario(terrain: &Terrain) -> (Scenario, TemplateSet) {
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
    let mut objects = vec![
        place("structure/depot", Some("north"), 0.24, 0.70, 0.0),
        place("structure/depot", Some("north"), 0.31, 0.76, 30.0),
        place("structure/depot", Some("south"), 0.76, 0.26, 45.0),
        place("structure/depot", Some("south"), 0.69, 0.20, 75.0),
    ];
    for step in 0..14u16 {
        let along = f32::from(step) / 13.0;
        objects.push(place(
            "prop/pine",
            None,
            0.30 + 0.40 * along,
            0.68 - 0.40 * along + 0.06 * (f32::from(step % 3) - 1.0),
            27.0 * f32::from(step),
        ));
    }
    let scenario = Scenario {
        format_version: 1,
        name: "Activation demo".to_owned(),
        description: String::new(),
        terrain: TerrainReference {
            path: "terrain/generated.cict".to_owned(),
        },
        players: vec![
            player("north", 1, 0.27, 0.73),
            player("south", 2, 0.73, 0.23),
        ],
        objects,
        waypoints: Vec::new(),
        scripts: Vec::new(),
    };
    let template = |id: &str, kind, model: Option<&str>| Template {
        id: id.to_owned(),
        kind,
        model: model.map(str::to_owned),
        name: None,
        speed: None,
        radius: None,
        footprint: None,
        passage: None,
    };
    let mut scout = template("unit/scout", TemplateKind::Unit, Some("models/scout.glb"));
    scout.speed = Some(26.0);
    // The scout stand-in is a box 6.8 units across, so this is the model's own half-width: two
    // scouts crossing paths now pass each other rather than through each other.
    scout.radius = Some(3.4);
    let mut depot = template(
        "structure/depot",
        TemplateKind::Structure,
        Some("models/depot.glb"),
    );
    // Thirty-six metres of building over an eight-metre grid, so five cells square covers it.
    // ADR 3001 decision 4: a structure denies the ground it stands on, and the scouts patrolling
    // past it now walk round rather than through.
    depot.footprint = Some(Footprint { cells: [5, 5] });
    let templates = TemplateSet {
        format_version: 1,
        templates: vec![
            depot,
            template("prop/pine", TemplateKind::Prop, Some("models/pine.glb")),
            template("faction/vanguard", TemplateKind::Faction, None),
            scout,
        ],
    };
    (scenario, templates)
}

/// How many cells the grid refuses, which is worth printing twice: once for what the terrain
/// derived and once after the placements have stamped it, because the difference is the mechanic.
fn impassable_cells(ground: &Ground) -> usize {
    (0..ground.height())
        .flat_map(|y| (0..ground.width()).map(move |x| (x, y)))
        .filter(|(x, y)| !ground.passable(*x, *y))
        .count()
}

/// The snapshot-to-instances translation: group the forces by template, ground each object on the
/// terrain, and tint by owner. Presentation narrows simulation state freely — nothing feeds back.
#[expect(
    clippy::cast_possible_truncation,
    reason = "presentation narrows simulation state freely; nothing feeds back"
)]
fn activated_instances(forces: &Forces, terrain: &Terrain) -> BTreeMap<String, Vec<ModelInstance>> {
    /// Seat colours: north cool, south warm, in team order.
    const TEAM_TINTS: [[f32; 4]; 2] = [[0.55, 0.70, 1.0, 1.0], [1.0, 0.62, 0.42, 1.0]];
    let mut grouped: BTreeMap<String, Vec<ModelInstance>> = BTreeMap::new();
    for placed in forces.objects().values() {
        let [x, y] = [placed.position[0] as f32, placed.position[1] as f32];
        let Some(ground) = terrain.elevation_at_world(x, y) else {
            continue;
        };
        let radians = (f64::from(placed.rotation) / 4_294_967_296.0 * std::f64::consts::TAU) as f32;
        let instance = ModelInstance::placed([x, y, ground], radians, placed.scale as f32);
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

/// A scout stand-in: a low box in one pale material, tinted by its owner when drawn.
fn scout_model() -> Model {
    let half = 3.4f32;
    let height = 6.5f32;
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
                material: Some(0),
            }
            .with_generated_tangents()
        })
        .collect();
    Model {
        name: "scout".to_owned(),
        primitives,
        materials: vec![ModelMaterial {
            name: "hull".to_owned(),
            base_color: [0.80, 0.80, 0.78, 1.0],
            metallic: 0.1,
            roughness: 0.55,
            ..ModelMaterial::default()
        }],
        images: Vec::new(),
        has_skin: false,
        has_animation: false,
    }
}

/// A slot marker: a flat plate on the ground, drawn where a unit has been told to stand.
///
/// This is [ADR 3003](../../../docs/adr/3003-formation-movement.md) made visible. A formation is a
/// set of *destinations*, so it is legible before anybody arrives and invisible afterwards — which
/// is exactly when a player wants to see it. Tinted by owner like the scouts, and lifted a hair off
/// the terrain so it does not fight the ground for depth.
fn marker_model() -> Model {
    let half = 2.2f32;
    let lift = 0.35f32;
    let corners = [
        [-half, -half, lift],
        [half, -half, lift],
        [half, half, lift],
        [-half, half, lift],
    ];
    Model {
        name: "marker".to_owned(),
        primitives: vec![
            ModelPrimitive {
                vertices: corners
                    .into_iter()
                    .enumerate()
                    .map(|(corner, position)| ModelVertex {
                        position,
                        normal: [0.0, 0.0, 1.0],
                        uv: quad_uv(corner),
                        ..ModelVertex::default()
                    })
                    .collect(),
                indices: vec![0, 1, 2, 0, 2, 3],
                material: Some(0),
            }
            .with_generated_tangents(),
        ],
        materials: vec![ModelMaterial {
            name: "paint".to_owned(),
            base_color: [1.0, 1.0, 1.0, 1.0],
            metallic: 0.0,
            roughness: 0.9,
            emissive: [0.25, 0.25, 0.25],
            ..ModelMaterial::default()
        }],
        images: Vec::new(),
        has_skin: false,
        has_animation: false,
    }
}

/// The demo's opening inputs: each player spawns three scouts beside their start.
fn demo_spawns(scenario: &Scenario) -> Vec<Command> {
    let mut commands = Vec::new();
    for (seat, slot) in scenario.players.iter().enumerate() {
        for spread in 0..3i16 {
            let offset = f64::from(spread - 1) * 24.0;
            commands.push(Command {
                tick: 0,
                player: PlayerId(u8::try_from(seat).expect("the demo has two seats")),
                payload: spawn_command(
                    "unit/scout",
                    f64::from(slot.start.x) + offset,
                    f64::from(slot.start.y) - 40.0,
                ),
            });
        }
    }
    commands
}

/// The demo's standing orders: every five seconds, each side's scouts are sent **as one group** to
/// the next corner of a square around the map's middle, the two sides half a lap apart.
///
/// One order per side rather than one per scout, because that is what makes this a demonstration of
/// [ADR 3003](../../../docs/adr/3003-formation-movement.md) rather than of three coincidental
/// destinations: three scouts sent to one point would arrive in a pile that avoidance then has to
/// untangle, and three scouts sent as a group arrive in the shape they set out in.
///
/// This is host-side input generation, not simulation: the commands it produces are ordinary
/// tick-stamped inputs, exactly what a lobby's network session or a player's clicks would feed in.
fn demo_orders(kernel: &Kernel, extent: [f32; 2]) -> Vec<Command> {
    let tick = kernel.tick();
    if tick % 150 != 30 {
        return Vec::new();
    }
    let Some(units) = kernel
        .subsystem(UNITS)
        .and_then(|subsystem| subsystem.as_any().downcast_ref::<Units>())
    else {
        return Vec::new();
    };
    let corners: [[f64; 2]; 4] = [
        [f64::from(extent[0]) * 0.35, f64::from(extent[1]) * 0.35],
        [f64::from(extent[0]) * 0.65, f64::from(extent[1]) * 0.35],
        [f64::from(extent[0]) * 0.65, f64::from(extent[1]) * 0.65],
        [f64::from(extent[0]) * 0.35, f64::from(extent[1]) * 0.65],
    ];
    let phase = tick / 150;

    // Grouped by seat, in a `BTreeMap` for the reason everything in this loop is ordered: the
    // commands it produces are recorded input, and input that came out in a different order on
    // another machine is a replay that does not reproduce.
    let mut sides: BTreeMap<u8, Vec<ObjectId>> = BTreeMap::new();
    for (id, unit) in units.units() {
        sides.entry(unit.owner.0).or_default().push(*id);
    }
    sides
        .into_iter()
        .map(|(seat, group)| {
            let leg = usize::try_from((phase + u64::from(seat) * 2) % 4).expect("below four");
            let corner = corners[leg];
            // The facing a player would have dragged: along the leg after this one, so each side
            // arrives at the corner already turned for the way it is about to march. Standing in
            // for a mouse the viewer does not have, and it is what makes the turn visible —
            // without it the patrol would translate round the square without ever wheeling.
            let next = corners[(leg + 1) % corners.len()];
            let facing = [next[0] - corner[0], next[1] - corner[1]];
            Command {
                tick,
                player: PlayerId(seat),
                payload: move_group_facing_command(&group, corner[0], corner[1], facing),
            }
        })
        .collect()
}

/// Where one unit stood at the end of a tick, and whose it is.
#[derive(Debug, Clone, Copy)]
struct UnitPose {
    position: [f64; 2],
    owner: u8,
    /// Where it has been told to stand, when it has been told anything.
    ///
    /// The last waypoint of its route, which after a group order *is* its slot in the formation —
    /// so drawing these is drawing the formation, with no simulation state added to do it.
    slot: Option<[f64; 2]>,
}

/// Every unit's pose at the end of the tick just computed.
fn unit_poses(units: &Units) -> BTreeMap<ObjectId, UnitPose> {
    units
        .units()
        .iter()
        .map(|(id, unit)| {
            (
                *id,
                UnitPose {
                    position: unit.position,
                    owner: unit.owner.0,
                    slot: unit.destination(),
                },
            )
        })
        .collect()
}

/// How fast a unit may turn on screen, in radians a second.
///
/// [ADR 3001](../../../docs/adr/3001-pathfinding.md) decision 9: facing is derived from motion and
/// smoothed with a turn-rate limit, in presentation floats, because the motion it is derived from is
/// already deterministic. Without the limit a unit reaching a waypoint pivots between one frame and
/// the next, which is most of what makes grid movement look mechanical.
const TURN_RATE: f32 = 6.0;

/// The units as instances, **interpolated between the last two ticks**.
///
/// The simulation runs at a fixed thirty ticks a second and the window draws at whatever rate it
/// can; drawing the raw tick position means a unit that moves thirty times a second in front of
/// somebody looking at a hundred and forty frames of it. `alpha` is where between the two ticks the
/// current instant falls, which is the whole reason [`TickAccumulator::alpha`] exists.
///
/// Interpolating *between two computed ticks* rather than extrapolating past the latest one is the
/// choice that keeps this honest: presentation shows a moment the simulation has already been
/// through, one tick behind, and never a position the simulation did not compute.
///
/// The facing uses `atan2` freely — this is presentation, which ADR 0007 decision 9 leaves
/// unrestricted, and nothing here feeds back into simulation state.
fn unit_instances(
    previous: &BTreeMap<ObjectId, UnitPose>,
    current: &BTreeMap<ObjectId, UnitPose>,
    headings: &mut BTreeMap<ObjectId, f32>,
    terrain: &Terrain,
    alpha: f32,
    delta: f32,
) -> Vec<ModelInstance> {
    const TEAM_TINTS: [[f32; 4]; 2] = [[0.55, 0.70, 1.0, 1.0], [1.0, 0.62, 0.42, 1.0]];

    headings.retain(|id, _| current.contains_key(id));
    current
        .iter()
        .filter_map(|(id, pose)| {
            // A unit that has just spawned has no previous pose, so it is drawn where it is rather
            // than interpolated from nowhere.
            let was = previous.get(id).unwrap_or(pose);
            let step = [
                pose.position[0] - was.position[0],
                pose.position[1] - was.position[1],
            ];
            let x = (was.position[0] + step[0] * f64::from(alpha)) as f32;
            let y = (was.position[1] + step[1] * f64::from(alpha)) as f32;
            let ground = terrain.elevation_at_world(x, y)?;

            // Derived from the motion, which is what decision 9 says presentation does. A stationary
            // unit keeps the heading it had rather than snapping to zero and facing east.
            let heading = headings.entry(*id).or_insert(0.0);
            if step[0] != 0.0 || step[1] != 0.0 {
                let target = (step[1] as f32).atan2(step[0] as f32);
                let mut difference = target - *heading;
                let turn = std::f32::consts::TAU;
                difference -= (difference / turn).round() * turn;
                let allowed = TURN_RATE * delta;
                *heading += difference.clamp(-allowed, allowed);
            }

            Some(
                ModelInstance::placed([x, y, ground], *heading, 1.0)
                    .with_tint(TEAM_TINTS[usize::from(pose.owner) % TEAM_TINTS.len()]),
            )
        })
        .collect()
}

/// One flat plate per unit that has somewhere to be, on the ground at its slot.
///
/// Not interpolated and not turned: a slot does not move between ticks, and a plate has no front.
fn slot_instances(current: &BTreeMap<ObjectId, UnitPose>, terrain: &Terrain) -> Vec<ModelInstance> {
    const TEAM_TINTS: [[f32; 4]; 2] = [[0.55, 0.70, 1.0, 1.0], [1.0, 0.62, 0.42, 1.0]];
    current
        .values()
        .filter_map(|pose| {
            let slot = pose.slot?;
            #[expect(
                clippy::cast_possible_truncation,
                reason = "presentation narrows simulation state freely; nothing feeds back"
            )]
            let (x, y) = (slot[0] as f32, slot[1] as f32);
            let ground = terrain.elevation_at_world(x, y)?;
            Some(
                ModelInstance::placed([x, y, ground], 0.0, 1.0)
                    .with_tint(TEAM_TINTS[usize::from(pose.owner) % TEAM_TINTS.len()]),
            )
        })
        .collect()
}

/// The three kinds `J` cycles through, in the order it cycles them.
///
/// A lake first, because that is what a whole-map water table over a generated heightfield actually
/// is and what the committed references were captured through. The flow and wind vectors are the same
/// direction, so stepping river to ocean changes the kind and nothing else about the heading.
const WATER_CYCLE: [(&str, WaterKind); 3] = [
    ("lake", WaterKind::Lake),
    ("river", WaterKind::River { flow: [7.0, 2.5] }),
    ("ocean", WaterKind::Ocean { wind: [7.0, 2.5] }),
];

/// A water table just above the terrain's low point, spanning the whole map.
///
/// The elevation comes from [`Terrain::water_level`] rather than from a derivation of the viewer's
/// own, because the simulation refuses to walk under the same line and two derivations of one water
/// level is a unit wading through a lake. The shoreline needs no outline: the water pass clips the
/// surface wherever the bed rises through it, so one rectangle fills every basin the terrain has.
fn water_table(terrain: &Terrain, kind: WaterKind) -> WaterSurface {
    let [extent_x, extent_y] = terrain.world_extent();
    WaterSurface::of_kind([0.0, 0.0, extent_x, extent_y], terrain.water_level(), kind)
}

/// Everything created once a window exists.
/// The simulation the viewer runs when it generated its own scenario: the kernel, the fixed-timestep
/// accumulator that drives it from frame time, and the model its units wear.
struct Simulation {
    kernel: Kernel,
    accumulator: TickAccumulator,
    scout: Model,
    /// The plate drawn at each unit's slot, so a formation is legible before it arrives.
    marker: Model,
    /// Unit poses at the end of the tick before last, and at the end of the last one. Presentation
    /// draws between them; see [`unit_instances`].
    previous: BTreeMap<ObjectId, UnitPose>,
    current: BTreeMap<ObjectId, UnitPose>,
    /// The heading each unit is currently drawn at, turn-rate limited toward where it is moving.
    /// Retained across frames because a turn rate is only meaningful against where it turned from.
    headings: BTreeMap<ObjectId, f32>,
}

struct Active {
    window: Arc<Window>,
    context: GpuContext,
    terrain_renderer: TerrainRenderer,
    surface: SurfaceRenderer,
    models: Vec<ModelBatch>,
    /// How many of `models` are static scenery. Everything past this index is rebuilt per frame
    /// from the units snapshot, so the vector is truncated back to it before each rebuild.
    static_models: usize,
    simulation: Option<Simulation>,
    water: Vec<WaterBody>,
    /// The captured sky, uploaded once the device exists.
    ///
    /// `None` when no `.hdr` was given, and then every pass takes the analytic branch — so the viewer
    /// still opens in the state the committed references were captured in.
    sky: Option<Sky>,
    /// The virtual-texture cache, when `V` has switched it on.
    ///
    /// Off by default, for the reason the environment starts at its own default: the first frame in the
    /// window is the frame the committed references were captured from, and a key press is the only way out
    /// of it.
    pages: Option<TerrainPageCache>,
}

struct Viewer {
    terrain: Terrain,
    /// Block-compressed layer textures the package carried, by layer. Empty for a generated terrain.
    layer_textures: Vec<Option<TextureAsset>>,
    camera: RtsCamera,
    input: InputState,
    active: Option<Active>,
    last_frame: Option<Instant>,
    /// Seconds since the first frame, which is what animates the water.
    elapsed: f32,
    /// Which entry of [`WATER_CYCLE`] the table is built from. Held here rather than inside [`Active`]
    /// so the choice survives the window being rebuilt, the same as the display settings.
    water_kind: usize,
    /// How many frames have been presented, which drives the temporal jitter sequence.
    frame_ordinal: u32,
    /// Held here rather than only inside the surface, so the setting survives the window being rebuilt.
    display: DisplaySettings,
    /// The air and the light, which the time-of-day and weather keys drive.
    ///
    /// Held across frames because it is *state* a key mutates, not something derivable from the frame
    /// ordinal. It starts at [`Environment::default`], whose every field is chosen so a frame rendered
    /// through it is identical to one rendered before an environment existed — so the viewer still opens
    /// in the state the committed captures were taken in, and a key press is the only way out of it.
    environment: Environment,
    /// Which entry of [`WEATHER_CYCLE`] is in force.
    weather: usize,
    /// Seconds since the last per-pass breakdown was printed, or `None` when timing is off.
    timing_countdown: Option<f32>,
    /// The sky image given on the command line, held across window rebuilds.
    ///
    /// The decoded asset rather than the uploaded texture, because a resize replaces the renderer and
    /// everything built against it, and re-reading the file to recover from that would be absurd.
    sky_asset: Option<SkyAsset>,
    /// Whether `K` currently has the captured sky switched on. Ignored with no image loaded.
    sky_enabled: bool,
    /// A scenario to activate into a kernel and draw, in place of the building scatter.
    ///
    /// `Some` for the generated map, `None` for a loaded package — packages do not carry a template
    /// set yet, so their placements have nothing to resolve against.
    activation: Option<(Scenario, TemplateSet)>,
    failure: Option<String>,
}

impl Viewer {
    fn new(
        terrain: Terrain,
        layer_textures: Vec<Option<TextureAsset>>,
        activation: Option<(Scenario, TemplateSet)>,
        sky_asset: Option<SkyAsset>,
    ) -> Self {
        let [extent_x, extent_y] = terrain.world_extent();
        let camera = RtsCamera::new(
            RtsCameraProfile::default(),
            [extent_x * 0.5, extent_y * 0.35],
            &TerrainGround(&terrain),
        );
        Self {
            terrain,
            layer_textures,
            camera,
            input: InputState::default(),
            active: None,
            last_frame: None,
            elapsed: 0.0,
            // A lake, for the same reason the display and the environment start where they do.
            water_kind: 0,
            frame_ordinal: 0,
            // Starts where the headless captures are, so the first frame in the window is the frame the
            // references were rendered from and pressing a key is the only difference from them.
            display: DisplaySettings::NATIVE,
            // Same reasoning as the display above: the default environment is the one the references
            // were captured through, so the window opens on the frame they pin.
            environment: Environment::default(),
            weather: 0,
            timing_countdown: None,
            // On from the moment a file is given, because loading one and then having to find the key
            // that switches it on is a worse first run than the toggle is worth.
            sky_enabled: sky_asset.is_some(),
            sky_asset,
            activation,
            failure: None,
        }
    }

    /// Turns the per-pass timing printout on or off.
    fn toggle_timing(&mut self) {
        let Some(active) = &mut self.active else {
            return;
        };
        let wanted = self.timing_countdown.is_none();
        let enabled = active.surface.set_timing(&active.context, wanted);
        self.timing_countdown = enabled.then_some(TIMING_REPORT_SECONDS);
        if wanted && !enabled {
            eprintln!("timing: unavailable, this device does not offer TIMESTAMP_QUERY");
        } else {
            eprintln!("timing: {}", if enabled { "on" } else { "off" });
        }
    }

    /// Prints the per-pass breakdown when the interval has elapsed.
    fn report_timings(&mut self, delta: f32) {
        let Some(remaining) = &mut self.timing_countdown else {
            return;
        };
        *remaining -= delta;
        if *remaining > 0.0 {
            return;
        }
        self.timing_countdown = Some(TIMING_REPORT_SECONDS);
        let Some(active) = &self.active else { return };
        match active.surface.timings(&active.context) {
            Some(Ok(timings)) => eprintln!("timing: {timings}"),
            Some(Err(error)) => eprintln!("timing: could not read the breakdown: {error}"),
            None => self.timing_countdown = None,
        }
    }

    /// Applies a weather or time-of-day key, returning whether the key was one.
    ///
    /// # Why the viewer needed this at all
    ///
    /// Cloud shadows, wetness and lying snow reach the shaders through the environment, and the viewer
    /// set none — so it always ran the default, whose whole purpose is to change nothing. Every one of
    /// those terms was therefore reachable only from a test capture, and two of them cannot be judged
    /// from a still image at all: cloud shadows *drift*, at a rate the wind sets, and the sun *moves*.
    /// That is the same argument the antialiasing key is there for — what something does as the frame
    /// changes is not something a capture reports.
    fn change_environment(&mut self, code: KeyCode, repeat: bool) -> bool {
        match code {
            KeyCode::KeyG if !repeat => {
                self.weather = (self.weather + 1) % WEATHER_CYCLE.len();
                let preset = &WEATHER_CYCLE[self.weather];
                // Through `with_weather` rather than by assigning the field, because that is what
                // derives the fog and cloud coverage the overcast implies. Assigning `weather` directly
                // would set a storm's saturation and leave its sky clear.
                //
                // From a *fresh* environment rather than the one in force, keeping only the hour.
                // `with_weather` raises fog and cloud coverage to what the overcast implies and never
                // lowers them, deliberately, so a caller that set a foggy morning does not have it taken
                // away by fair weather. Cycling on top of the accumulated value inherits every previous
                // state's maximum instead, which the first run of this key showed: stepping thunderstorm
                // to snowfall left the snowfall reporting the storm's 0.81 coverage rather than its own
                // 0.60, and a full lap back to clear would have arrived at an overcast sky called clear.
                self.environment = Environment {
                    time_of_day: self.environment.time_of_day,
                    ..Environment::default()
                }
                .with_weather((preset.build)());
            }
            KeyCode::Comma => self.environment.time_of_day -= HOUR_STEP,
            KeyCode::Period => self.environment.time_of_day += HOUR_STEP,
            KeyCode::KeyK if !repeat => {
                if self.sky_asset.is_none() {
                    eprintln!("sky: no image was given; pass an .hdr on the command line");
                    return true;
                }
                self.sky_enabled = !self.sky_enabled;
                eprintln!(
                    "sky: {}",
                    if self.sky_enabled {
                        "captured environment"
                    } else {
                        "analytic gradient"
                    }
                );
            }
            _ => return false,
        }
        // Wrapped through the accessor rather than left to grow without bound, so the printed hour and
        // the stored one agree after a few hundred presses.
        self.environment.time_of_day = self.environment.hour();
        report_environment(&self.environment, WEATHER_CYCLE[self.weather].name);
        true
    }

    /// Switches the virtual-texture cache on or off, rebuilding the terrain's bindings either way.
    ///
    /// # Why the cache needs a key rather than being always on
    ///
    /// Two reasons, and the first is the standing rule: what a *filter* does as the camera moves is not
    /// something a still capture reports, and a page cache is a filter with a residency policy attached. A
    /// crawling seam at a page boundary, a visible step between mip levels, and a page arriving a frame late
    /// are all motion artefacts — the same argument the antialiasing key exists for. Comparing the two paths
    /// live is the only way to see them, and off-by-default keeps the window's first frame identical to the
    /// captures.
    ///
    /// The second is that the request below is a placeholder for a decision that does not exist yet: nothing
    /// derives a [`TerrainDetailRequest`] from a camera, so this asks for the whole map at the coarse density
    /// and lets the residency map rank what fits. On a large map that is far more pages than the budget
    /// holds, which is not a flaw to hide — the ground the cache loses falls back to the direct blend, and
    /// watching where that boundary sits as the camera moves is exactly what a view-driven request has to fix.
    fn toggle_pages(&mut self) {
        let Some(active) = &mut self.active else {
            return;
        };
        if active.pages.take().is_some() {
            active.terrain_renderer.detach_pages(&active.context);
            eprintln!("pages: off, terrain is blending layers per fragment");
            active.window.request_redraw();
            return;
        }
        let budget = u32::try_from(VIRTUAL_PAGE_LAYERS).unwrap_or(u32::MAX);
        match TerrainPageCache::new(&active.context, &active.terrain_renderer, budget) {
            Ok(cache) => {
                active
                    .terrain_renderer
                    .attach_pages(&active.context, &cache);
                let (cells_x, cells_y) = active.terrain_renderer.cell_size();
                eprintln!(
                    "pages: on, {budget} slots over {cells_x}x{cells_y} cells at 16 texels per cell"
                );
                active.pages = Some(cache);
                active.window.request_redraw();
            }
            Err(error) => eprintln!("pages: could not allocate the cache: {error}"),
        }
    }

    /// Rebuilds the water table as the next kind in [`WATER_CYCLE`].
    ///
    /// Here for the same reason the weather and antialiasing keys are, and with a stronger case than
    /// either. Two of the three things that separate a river from a lake are *motion* — the current
    /// carrying the whole surface downstream, and the chop running along the channel rather than
    /// across it — and a still capture cannot report either. The third, a surf line advancing and
    /// retreating along a shore with the swell, is motion as well. Everything a headless reference can
    /// show about these three kinds is their colour.
    ///
    /// A body's material is fixed at construction, so this replaces the body rather than assigning
    /// through it. That is cheap: the uniform is 112 bytes and the grid is procedural, so there is no
    /// vertex buffer to rebuild — only a buffer, a bind group, and the cell count the new wavelength
    /// implies.
    fn cycle_water(&mut self) {
        self.water_kind = (self.water_kind + 1) % WATER_CYCLE.len();
        let (name, kind) = WATER_CYCLE[self.water_kind];
        let Some(active) = &mut self.active else {
            return;
        };
        let table = water_table(&self.terrain, kind);
        match WaterBody::new(&active.context, table, active.surface.water_layout()) {
            Ok(body) => {
                eprintln!("water: {name}, {} vertices", body.vertex_count());
                active.water = vec![body];
                active.window.request_redraw();
            }
            Err(error) => eprintln!("water: could not build a {name}: {error}"),
        }
    }

    /// Stages the pages this frame's camera wants, if the cache is on.
    ///
    /// The view is built from the camera's own pose and the projection the frame renders with, so the
    /// residency ranking and the image agree about where the camera is looking. `right` and `up` are derived
    /// from `forward` against world up, which is what the view matrix does with the same inputs.
    fn stage_pages(&mut self) {
        let Some(active) = &mut self.active else {
            return;
        };
        let Some(cache) = &mut active.pages else {
            return;
        };
        let pose = self.camera.pose();
        let (width, height) = active.surface.size();
        let projection = Projection::for_viewport(width, height);
        let forward = pose.forward;
        let right = normalise(cross(forward, [0.0, 0.0, 1.0]));
        let up = cross(right, forward);
        let [extent_x, extent_y] = self.terrain.world_extent();
        let (cells_x, cells_y) = active.terrain_renderer.cell_size();
        let view = VirtualPageView::new(
            pose.eye,
            forward,
            right,
            up,
            (
                [0.0, 0.0, 0.0],
                [extent_x, extent_y, highest_elevation(&self.terrain)],
            ),
            (projection.vertical_fov * 0.5).tan(),
            projection.aspect_ratio,
            self.terrain.horizontal_scale(),
        );
        let requests: [TerrainDetailRequest; 1] = [TerrainDetailRequest::uniform(
            [0, 0],
            [cells_x, cells_y],
            16,
        )];
        let composed = cache.update(&active.context, &requests, view);
        // Only when it did something, so the steady state — a camera that has not moved far — prints nothing
        // and the line means "the cache is working" rather than "the cache is here".
        if composed > 0 {
            eprintln!("pages: composed {composed}");
        }
    }

    /// Applies a change to the display settings and reports what took effect.
    ///
    /// Reported rather than assumed: the resolution scale is clamped and rounded on its way to a target
    /// size, so the figure that matters is the one the chain allocated and not the one asked for.
    fn change_display(&mut self, event_loop: &ActiveEventLoop, display: DisplaySettings) {
        self.display = display;
        let Some(active) = &mut self.active else {
            return;
        };
        if let Err(error) =
            active
                .surface
                .set_display(&active.context, &active.terrain_renderer, display)
        {
            self.fail(event_loop, error.to_string());
            return;
        }
        report_display(&active.surface);
        active.window.request_redraw();
    }

    /// Records a fatal error and asks the loop to stop.
    fn fail(&mut self, event_loop: &ActiveEventLoop, message: String) {
        eprintln!("error: {message}");
        self.failure = Some(message);
        event_loop.exit();
    }
}

impl ApplicationHandler for Viewer {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.active.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("Commanders in Chief — terrain viewer")
            .with_inner_size(LogicalSize::new(1280, 800));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => return self.fail(event_loop, error.to_string()),
        };

        let (context, surface) = match pollster::block_on(GpuContext::for_window(window.clone())) {
            Ok(pair) => pair,
            Err(error) => return self.fail(event_loop, error.to_string()),
        };
        eprintln!("adapter: {}", context.adapter_info().name);

        let terrain_renderer = match TerrainRenderer::with_materials(
            &context,
            &self.terrain,
            &layer_materials(&self.terrain, &self.layer_textures),
        ) {
            Ok(renderer) => renderer,
            Err(error) => return self.fail(event_loop, error.to_string()),
        };
        let albedo = terrain_renderer.layer_albedo();
        eprintln!(
            "terrain layers: {} slices at {:?}, {} mip levels, {}",
            albedo.layer_count(),
            albedo.size(),
            albedo.mip_level_count(),
            // Which upload path the array took. Worth printing rather than inferring: the two produce an
            // array a shader binds identically, so nothing about the frame says which one ran -- and the
            // answer depends on both the content and the adapter.
            match albedo.block_format() {
                Some(format) => format.name(),
                None => "uncompressed RGBA8",
            }
        );
        let size = window.inner_size();
        let surface = match SurfaceRenderer::new(
            &context,
            &terrain_renderer,
            surface,
            size.width,
            size.height,
            self.display,
        ) {
            Ok(surface) => surface,
            Err(error) => return self.fail(event_loop, error.to_string()),
        };
        eprintln!("surface: {:?} at {:?}", surface.format(), surface.size());
        report_display(&surface);

        // What stands on the terrain. With a scenario present this is kernel state made visible:
        // activate, read the `Forces` snapshot, and build one batch per template — owners as tints,
        // binary turns as poses. Without one (a loaded package, until packages carry a template
        // set), the old building scatter keeps the shadow pass a caster that is not terrain.
        let mut simulation = None;
        let models = if let Some((scenario, templates)) = &self.activation {
            let mut kernel = Kernel::new(KernelConfig {
                seed: 0xC1C_DE30,
                ticks_per_second: 30,
            });
            if let Err(error) = activate(&mut kernel, scenario, templates) {
                return self.fail(event_loop, error.to_string());
            }
            // The grid goes in *before* the units that consult it, because a subsystem reads peers
            // registered earlier as they stand this tick. Registered the other way round, a unit
            // would path against the previous tick's ground — correct today, when nothing edits it,
            // and wrong the moment something does.
            let ground = Ground::derive(&self.terrain, GroundRules::derived(&self.terrain))
                .with_templates(templates);
            let derived_impassable = impassable_cells(&ground);
            eprintln!(
                "ground: {}x{} cells at {:.1} m, {derived_impassable} impassable",
                ground.width(),
                ground.height(),
                ground.cell_size(),
            );
            kernel.add_subsystem(Box::new(ground));
            kernel.add_subsystem(Box::new(Units::new(templates)));
            // Tick zero carries the opening inputs: each seat spawns its scouts. From here on the
            // draw loop advances the kernel at its fixed rate, whatever the frame rate does.
            if let Err(error) = kernel.advance(&demo_spawns(scenario)) {
                return self.fail(event_loop, error.to_string());
            }
            // And tick zero is also when the depots stamp their footprints, because the grid reads
            // what stands on it through a peer and a peer is only readable inside a tick.
            if let Some(stamped) = kernel
                .subsystem(cic_sim::GROUND)
                .and_then(|subsystem| subsystem.as_any().downcast_ref::<Ground>())
            {
                eprintln!(
                    "ground: {} impassable once the placements stamped, {} newly denied",
                    impassable_cells(stamped),
                    impassable_cells(stamped) - derived_impassable,
                );
            }
            let Some(forces) = kernel
                .subsystem(FORCES)
                .and_then(|subsystem| subsystem.as_any().downcast_ref::<Forces>())
            else {
                return self.fail(event_loop, "activation registered no forces".to_owned());
            };
            eprintln!(
                "kernel: {} players, {} objects, ticking at 30 Hz from tick {}",
                forces.players().len(),
                forces.objects().len(),
                kernel.tick(),
            );
            let grouped = activated_instances(forces, &self.terrain);
            let mut batches = Vec::new();
            for (template, instances) in &grouped {
                let model = match template.as_str() {
                    "structure/depot" => building_model(),
                    _ => pine_model(),
                };
                match ModelBatch::new(&context, &model, instances, surface.material_layout()) {
                    Ok(batch) => {
                        eprintln!(
                            "models: {} instances of `{template}`, {} triangles",
                            batch.instance_count(),
                            batch.triangle_count()
                        );
                        batches.push(batch);
                    }
                    Err(error) => return self.fail(event_loop, error.to_string()),
                }
            }
            let accumulator = TickAccumulator::new(kernel.tick_seconds(), 8);
            // Both snapshots start at tick zero's state, so the first frame interpolates between a
            // pose and itself rather than sliding every unit in from wherever an empty map put it.
            let opening = kernel
                .subsystem(UNITS)
                .and_then(|subsystem| subsystem.as_any().downcast_ref::<Units>())
                .map(unit_poses)
                .unwrap_or_default();
            simulation = Some(Simulation {
                kernel,
                accumulator,
                scout: scout_model(),
                marker: marker_model(),
                previous: opening.clone(),
                current: opening,
                headings: BTreeMap::new(),
            });
            batches
        } else {
            let placements = building_placements(&self.terrain);
            match ModelBatch::new(
                &context,
                &building_model(),
                &placements,
                surface.material_layout(),
            ) {
                Ok(batch) => {
                    eprintln!(
                        "models: {} instances of {} triangles",
                        batch.instance_count(),
                        batch.triangle_count()
                    );
                    vec![batch]
                }
                Err(error) => return self.fail(event_loop, error.to_string()),
            }
        };

        // One water table across the whole map. The shader clips it wherever the bed rises through
        // it, so a single rectangle fills every depression the heightfield happens to have — which is
        // what makes this work for a loaded map as well as for the generated one.
        let (name, kind) = WATER_CYCLE[self.water_kind];
        let table = water_table(&self.terrain, kind);
        let water = match WaterBody::new(&context, table, surface.water_layout()) {
            Ok(body) => {
                eprintln!(
                    "water: {name} surface at {:.1}, {} vertices",
                    table.elevation,
                    body.vertex_count()
                );
                vec![body]
            }
            Err(error) => return self.fail(event_loop, error.to_string()),
        };

        // The environment, uploaded once. Aimed at the current sun before the first frame rather
        // than left at the file's own rotation, so the window opens with the sky and the shadows in
        // agreement instead of requiring an hour key to be pressed to fix it.
        let sky = match &self.sky_asset {
            Some(asset) => match Sky::new(
                &context,
                surface.sky_layout(),
                asset,
                SkySettings::default(),
            ) {
                Ok(mut sky) => {
                    sky.aim_at(&context, self.environment.sun_direction());
                    eprintln!(
                        "sky: bound, turned {:.0} degrees to put its sun on the light",
                        sky.settings().yaw.to_degrees()
                    );
                    Some(sky)
                }
                Err(error) => return self.fail(event_loop, error.to_string()),
            },
            None => None,
        };

        let static_models = models.len();
        self.active = Some(Active {
            window,
            context,
            terrain_renderer,
            surface,
            models,
            static_models,
            simulation,
            water,
            sky,
            pages: None,
        });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(active) = &mut self.active
                    && let Err(error) = active.surface.resize(
                        &active.context,
                        &active.terrain_renderer,
                        size.width,
                        size.height,
                    )
                {
                    self.fail(event_loop, error.to_string());
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let pressed = event.state == ElementState::Pressed;
                if let PhysicalKey::Code(code) = event.physical_key {
                    if code == KeyCode::Escape && pressed {
                        event_loop.exit();
                        return;
                    }
                    // Display settings act once per press rather than every frame they are held: each one
                    // reallocates every intermediate target, and repeating that at the key-repeat rate
                    // would rebuild the chain dozens of times a second.
                    if pressed && code == KeyCode::KeyP {
                        self.toggle_timing();
                        return;
                    }
                    if pressed && code == KeyCode::KeyV {
                        self.toggle_pages();
                        return;
                    }
                    if pressed && !event.repeat && code == KeyCode::KeyJ {
                        self.cycle_water();
                        return;
                    }
                    if pressed && let Some(display) = display_change(code, self.display) {
                        self.change_display(event_loop, display);
                        return;
                    }
                    // Unlike a display change this reallocates nothing — an environment is uniform data
                    // — so it applies in place rather than rebuilding the chain, and the time-of-day
                    // keys deliberately act on key *repeat* too, because scrubbing the sun across the
                    // sky is the point of having them. The weather cycle suppresses repeat, since
                    // holding it would race through four presets faster than any of them could be seen.
                    if pressed && self.change_environment(code, event.repeat) {
                        return;
                    }
                    if let Some(action) = action_for(code) {
                        self.input.set_action(action, pressed);
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if matches!(button, MouseButton::Right | MouseButton::Middle) {
                    self.input.set_dragging(state == ElementState::Pressed);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // A line delta and a pixel delta differ by orders of magnitude, so they cannot share
                // a scale. One line is one notch; pixels are divided down to roughly match.
                let amount = match delta {
                    MouseScrollDelta::LineDelta(_, lines) => lines,
                    MouseScrollDelta::PixelDelta(position) => position.y as f32 / 120.0,
                };
                self.input.add_scroll(amount);
            }
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.draw() {
                    self.fail(event_loop, error);
                }
            }
            _ => {}
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device: winit::event::DeviceId,
        event: winit::event::DeviceEvent,
    ) {
        // Raw motion rather than cursor position, so a drag keeps panning when the pointer reaches the
        // screen edge instead of silently stopping there.
        if let winit::event::DeviceEvent::MouseMotion { delta } = event {
            self.input
                .add_pointer_motion(delta.0 as f32, delta.1 as f32);
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(active) = &self.active {
            active.window.request_redraw();
        }
    }
}

impl Viewer {
    fn draw(&mut self) -> Result<(), String> {
        // Nothing to draw into, and the check is here rather than at the call site because the borrow is
        // taken again after the camera has moved — staging pages needs the camera *and* the device.
        if self.active.is_none() {
            return Ok(());
        }
        let now = Instant::now();
        let delta = self
            .last_frame
            .map_or(1.0 / 60.0, |last| now.duration_since(last).as_secs_f32())
            .min(MAXIMUM_FRAME_SECONDS);
        self.last_frame = Some(now);

        let intent = self.input.take_intent(DRAG_PIXELS_TO_UNITS);
        self.camera
            .update(intent, delta, &TerrainGround(&self.terrain));

        // Accumulated here rather than read from a clock inside the renderer, which is what lets a
        // headless capture of the same scene pin the wave phase and stay reproducible.
        self.elapsed += delta;
        // The jitter phase advances once per presented frame, which is what a temporal resolve accumulates
        // over. Counted here rather than inside the renderer so a capture of the same sequence is
        // reproducible -- see `DeferredFrame::jitter`.
        self.frame_ordinal = self.frame_ordinal.wrapping_add(1);

        // Before the frame, so a page this camera wants is composed in time to be sampled by it. The compose
        // and reduce passes are their own submission, so this costs a submit on the frames that miss and
        // nothing at all on the frames that do not.
        self.stage_pages();
        let Some(active) = &mut self.active else {
            return Ok(());
        };

        // The simulation advances at its fixed rate however fast frames come: the accumulator turns
        // frame time into whole ticks, and presentation may interpolate but never advance. Standing
        // orders are host-side input generation — the same shape a lobby's network session would be.
        if let Some(simulation) = &mut active.simulation {
            let extent = self.terrain.world_extent();
            let ticks = simulation.accumulator.frame(f64::from(delta));
            for _ in 0..ticks {
                let orders = demo_orders(&simulation.kernel, extent);
                simulation
                    .kernel
                    .advance(&orders)
                    .map_err(|error| error.to_string())?;
                let Some(units) = simulation
                    .kernel
                    .subsystem(UNITS)
                    .and_then(|subsystem| subsystem.as_any().downcast_ref::<Units>())
                else {
                    return Err("the simulation lost its units".to_owned());
                };
                // The tick that just finished becomes the one being interpolated *towards*, and the
                // one before it is what presentation draws from. Snapshotting inside the loop rather
                // than after it keeps that true when a slow frame advances several ticks at once.
                simulation.previous = std::mem::take(&mut simulation.current);
                simulation.current = unit_poses(units);
            }

            // Rebuilt every frame, not only on the frames a tick landed on: the interpolation is the
            // whole point, and it changes between two frames that share a tick. The batch is a
            // handful of instances, so the cost is a small upload rather than a rebuild of anything.
            #[expect(
                clippy::cast_possible_truncation,
                reason = "an interpolation fraction is in [0, 1) and narrows exactly enough"
            )]
            let alpha = simulation.accumulator.alpha() as f32;
            let instances = unit_instances(
                &simulation.previous,
                &simulation.current,
                &mut simulation.headings,
                &self.terrain,
                alpha,
                delta,
            );
            active.models.truncate(active.static_models);
            if !instances.is_empty() {
                let batch = ModelBatch::new(
                    &active.context,
                    &simulation.scout,
                    &instances,
                    active.surface.material_layout(),
                )
                .map_err(|error| error.to_string())?;
                active.models.push(batch);
            }
            // The formation, drawn where it is going rather than where it is. Built from the
            // current tick alone -- a slot does not move between two ticks, so there is nothing to
            // interpolate and interpolating it would only make the plate shiver.
            let slots = slot_instances(&simulation.current, &self.terrain);
            if !slots.is_empty() {
                let batch = ModelBatch::new(
                    &active.context,
                    &simulation.marker,
                    &slots,
                    active.surface.material_layout(),
                )
                .map_err(|error| error.to_string())?;
                active.models.push(batch);
            }
        }

        let (width, height) = active.surface.size();
        // Aimed every frame rather than when the hour key fires, which is the difference between a sky
        // that tracks a scrubbed day cycle and one that jumps when a key is released. It writes sixteen
        // bytes to a uniform; the expensive half -- finding the image's own sun -- was done once, at
        // upload.
        let sky = active.sky.as_mut().filter(|_| self.sky_enabled);
        let environment = match sky {
            Some(sky) => {
                sky.aim_at(&active.context, self.environment.sun_direction());
                // Not automatic. The renderer is handed the image and the environment separately, and
                // this is the line that makes the ground agree with what is behind it -- the ambient
                // and the fog colour both come off the picture from here on.
                self.environment.under_sky(sky.lighting())
            }
            None => self.environment,
        };
        // `in_environment` re-derives the light, so the time-of-day keys move the sun rather than only
        // recolouring it. At the default environment this is the frame the references were captured
        // from, which is what keeps a key press the only difference between the window and them.
        let frame = DeferredFrame::new(self.camera.pose(), width, height)
            .in_environment(environment)
            .at_time(self.elapsed)
            .at_jitter(self.frame_ordinal);
        active
            .surface
            .render(
                &active.context,
                &active.terrain_renderer,
                &active.models,
                &active.water,
                active.sky.as_ref().filter(|_| self.sky_enabled),
                frame,
            )
            .map_err(|error| error.to_string())?;

        // After the frame, so the breakdown read back describes one that has been submitted. It blocks on
        // the GPU, which is why it runs about once a second rather than here every time.
        self.report_timings(delta);
        Ok(())
    }
}

/// The tallest point on a terrain, in world units.
///
/// The page residency map wants the terrain's bounding box, and the *top* of it has to come from the
/// heightfield: a box that stops below the ground would project every page as though it were flat, ranking a
/// hilltop under the camera as though it were far away.
fn highest_elevation(terrain: &Terrain) -> f32 {
    terrain
        .elevations()
        .iter()
        .copied()
        .max()
        .map_or(0.0, |peak| f32::from(peak) * terrain.vertical_scale())
}

fn cross(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn normalise(vector: [f32; 3]) -> [f32; 3] {
    let length = (vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]).sqrt();
    if length <= f32::EPSILON {
        return [1.0, 0.0, 0.0];
    }
    vector.map(|component| component / length)
}

/// Maps a physical key to a change of display settings, or `None` if it is not one.
fn display_change(code: KeyCode, current: DisplaySettings) -> Option<DisplaySettings> {
    Some(match code {
        // All three, in ascending cost. Cycling rather than toggling because the point of the key is to
        // compare them on a moving camera, and what an edge does *as the camera moves* is the whole
        // subject -- no still capture reports it, and the temporal tier least of all.
        KeyCode::KeyT => current.with_antialiasing(match current.antialiasing {
            Antialiasing::None => Antialiasing::Fxaa,
            Antialiasing::Fxaa => Antialiasing::Taa,
            Antialiasing::Taa => Antialiasing::None,
        }),
        // Stepped from the *sanitised* scale rather than the stored one, so a press at either end of
        // the range moves back into it instead of walking a value that is already clamped.
        KeyCode::BracketLeft => {
            current.at_scale((current.scale() - RESOLUTION_SCALE_STEP).max(MIN_RESOLUTION_SCALE))
        }
        KeyCode::BracketRight => {
            current.at_scale((current.scale() + RESOLUTION_SCALE_STEP).min(MAX_RESOLUTION_SCALE))
        }
        _ => return None,
    })
}

/// Prints the environment in force, and enough of what it derived to tell a key press worked.
///
/// The sun's elevation is included because it is the figure that explains the frame: a cloud shadow at
/// a high sun lands nearly under its cloud and at a low one is thrown hundreds of units sideways, so an
/// elevation is the difference between a dapple that looks wrong and one that is merely unfamiliar.
fn report_environment(environment: &Environment, name: &str) {
    let weather = environment.weather;
    eprintln!(
        "environment: {name} at {:04.1}h, sun elevation {:.1} degrees, \
         overcast {:.2}, wetness {:.2}, snow {:.2}, cloud coverage {:.2}, fog {:.5}",
        environment.hour(),
        environment.sun_elevation().to_degrees(),
        weather.overcast,
        weather.wetness,
        weather.snow,
        environment.clouds.coverage,
        environment.fog.density,
    );
}

/// Prints the settings in force and the size they produced.
fn report_display(surface: &SurfaceRenderer) {
    let display = surface.display();
    let (render_width, render_height) = surface.render_size();
    let (output_width, output_height) = surface.size();
    eprintln!(
        "display: scale {:.2} ({render_width}x{render_height} into {output_width}x{output_height}), \
         antialiasing {:?}",
        display.scale(),
        display.antialiasing
    );
}

/// Maps a physical key to a camera action.
fn action_for(code: KeyCode) -> Option<Action> {
    Some(match code {
        KeyCode::KeyA | KeyCode::ArrowLeft => Action::PanWest,
        KeyCode::KeyD | KeyCode::ArrowRight => Action::PanEast,
        KeyCode::KeyS | KeyCode::ArrowDown => Action::PanSouth,
        KeyCode::KeyW | KeyCode::ArrowUp => Action::PanNorth,
        KeyCode::KeyQ => Action::RotateLeft,
        KeyCode::KeyE => Action::RotateRight,
        KeyCode::KeyR => Action::Reset,
        KeyCode::KeyF => Action::ResetRotation,
        _ => return None,
    })
}
