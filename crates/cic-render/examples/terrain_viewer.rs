//! An interactive terrain viewer.
//!
//! ```text
//! cargo run -p cic-render --example terrain_viewer --release
//! cargo run -p cic-render --example terrain_viewer --release -- path/to/map.cicmap
//! ```
//!
//! With no argument it generates a terrain, so the viewer runs before any content exists.
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
//! | `T` | Toggle antialiasing |
//! | `[` `]` | Step the resolution scale |
//! | `P` | Toggle the per-pass GPU timing printout |
//! | `Esc` | Quit |
//!
//! The last two are here because antialiasing is the one rendering change a still capture reports
//! badly. Its whole subject is what an edge does *as the camera moves*, and a resolution scale trades
//! frame rate for sampling rate — neither of which a headless test can show anyone. Both print the
//! settings and the size they took effect at, so a screenshot of the terminal says what the window is
//! showing.

// The generator clamps before converting and its inputs are bounded constants, so the width casts
// below cannot lose anything.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use std::error::Error;
use std::sync::Arc;
use std::time::Instant;

use cic_assets::model::{Model, ModelImage, ModelMaterial, ModelPrimitive, ModelVertex};
use cic_assets::{MapPackage, PackageLimits, Terrain, TerrainLayer};
use cic_camera::{RtsCamera, RtsCameraProfile};
use cic_render::display::{MAX_RESOLUTION_SCALE, MIN_RESOLUTION_SCALE};
use cic_render::{
    Action, Antialiasing, DeferredFrame, DisplaySettings, GpuContext, InputState, LayerMaterial,
    ModelBatch, ModelInstance, SurfaceRenderer, TerrainGround, TerrainRenderer, TextureImage,
    WaterBody, WaterSurface,
};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

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
    let terrain = if let Some(path) = std::env::args().nth(1) {
        load_package(&path)?
    } else {
        eprintln!("no map given; generating a terrain");
        generated_terrain()
    };
    eprintln!(
        "terrain {}x{} samples, {:?} world units, peak {:.0}",
        terrain.width(),
        terrain.height(),
        terrain.world_extent(),
        terrain
            .elevations()
            .iter()
            .copied()
            .max()
            .map_or(0.0, |peak| f32::from(peak) * terrain.vertical_scale()),
    );

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = Viewer::new(terrain);
    event_loop.run_app(&mut app)?;
    if let Some(error) = app.failure {
        return Err(error.into());
    }
    Ok(())
}

/// Opens a map package and returns its terrain.
fn load_package(path: &str) -> Result<Terrain, Box<dyn Error>> {
    let bytes = std::fs::read(path)?;
    let package = MapPackage::open(&bytes, PackageLimits::default())?;
    eprintln!("loaded {} from {path}", package.scenario().name);
    Ok(package.terrain().clone())
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
        .map(|(index, (normal, corners))| ModelPrimitive {
            vertices: corners
                .into_iter()
                .enumerate()
                .map(|(corner, position)| ModelVertex {
                    position,
                    normal,
                    uv: quad_uv(corner),
                })
                .collect(),
            indices: vec![0, 1, 2, 0, 2, 3],
            material: Some(usize::from(index == 0)),
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
                blended: false,
            },
            ModelMaterial {
                name: "roof".to_owned(),
                base_color: [0.36, 0.20, 0.16, 1.0],
                metallic: 0.0,
                roughness: 0.65,
                base_color_texture: Some(1),
                blended: false,
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
        width: SIZE,
        height: SIZE,
        rgba,
    }
}

/// Terrain layer surfaces, one per weight layer, at the world scale each tiles at.
fn layer_materials() -> Vec<LayerMaterial> {
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

/// A water table just above the terrain's low point, spanning the whole map.
///
/// Derived from the heightfield rather than authored, so the viewer floods a loaded `.cicmap` and the
/// generated terrain alike. The shoreline needs no outline: the water pass clips the surface wherever
/// the bed rises through it, so one rectangle at one elevation fills every basin the terrain has.
fn water_table(terrain: &Terrain) -> WaterSurface {
    let [extent_x, extent_y] = terrain.world_extent();
    let (low, high) = terrain
        .elevations()
        .iter()
        .fold((u16::MAX, u16::MIN), |(low, high), sample| {
            (low.min(*sample), high.max(*sample))
        });
    let scale = terrain.vertical_scale();
    let floor = f32::from(low) * scale;
    let ceiling = f32::from(high) * scale;
    // An eighth of the way up the terrain's range: high enough to be a lake rather than a puddle, low
    // enough to stay in the basins instead of drowning the map.
    let elevation = floor + (ceiling - floor) * 0.12;
    WaterSurface::new([0.0, 0.0, extent_x, extent_y], elevation)
}

/// Everything created once a window exists.
struct Active {
    window: Arc<Window>,
    context: GpuContext,
    terrain_renderer: TerrainRenderer,
    surface: SurfaceRenderer,
    models: Vec<ModelBatch>,
    water: Vec<WaterBody>,
}

struct Viewer {
    terrain: Terrain,
    camera: RtsCamera,
    input: InputState,
    active: Option<Active>,
    last_frame: Option<Instant>,
    /// Seconds since the first frame, which is what animates the water.
    elapsed: f32,
    /// Held here rather than only inside the surface, so the setting survives the window being rebuilt.
    display: DisplaySettings,
    /// Seconds since the last per-pass breakdown was printed, or `None` when timing is off.
    timing_countdown: Option<f32>,
    failure: Option<String>,
}

impl Viewer {
    fn new(terrain: Terrain) -> Self {
        let [extent_x, extent_y] = terrain.world_extent();
        let camera = RtsCamera::new(
            RtsCameraProfile::default(),
            [extent_x * 0.5, extent_y * 0.35],
            &TerrainGround(&terrain),
        );
        Self {
            terrain,
            camera,
            input: InputState::default(),
            active: None,
            last_frame: None,
            elapsed: 0.0,
            // Starts where the headless captures are, so the first frame in the window is the frame the
            // references were rendered from and pressing a key is the only difference from them.
            display: DisplaySettings::NATIVE,
            timing_countdown: None,
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

        let terrain_renderer =
            match TerrainRenderer::with_materials(&context, &self.terrain, &layer_materials()) {
                Ok(renderer) => renderer,
                Err(error) => return self.fail(event_loop, error.to_string()),
            };
        let albedo = terrain_renderer.layer_albedo();
        eprintln!(
            "terrain layers: {} slices at {:?}, {} mip levels",
            albedo.layer_count(),
            albedo.size(),
            albedo.mip_level_count()
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

        // A scattering of buildings, so the scene has something with a silhouette in it and the
        // shadow pass has a caster that is not terrain.
        let placements = building_placements(&self.terrain);
        let models = match ModelBatch::new(
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
        };

        // One water table across the whole map. The shader clips it wherever the bed rises through
        // it, so a single rectangle fills every depression the heightfield happens to have — which is
        // what makes this work for a loaded map as well as for the generated one.
        let table = water_table(&self.terrain);
        let water = match WaterBody::new(&context, table, surface.water_layout()) {
            Ok(body) => {
                eprintln!(
                    "water: surface at {:.1}, {} vertices",
                    table.elevation,
                    body.vertex_count()
                );
                vec![body]
            }
            Err(error) => return self.fail(event_loop, error.to_string()),
        };

        self.active = Some(Active {
            window,
            context,
            terrain_renderer,
            surface,
            models,
            water,
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
                    if pressed && let Some(display) = display_change(code, self.display) {
                        self.change_display(event_loop, display);
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
        let Some(active) = &mut self.active else {
            return Ok(());
        };
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

        let (width, height) = active.surface.size();
        let frame = DeferredFrame::new(self.camera.pose(), width, height).at_time(self.elapsed);
        active
            .surface
            .render(
                &active.context,
                &active.terrain_renderer,
                &active.models,
                &active.water,
                frame,
            )
            .map_err(|error| error.to_string())?;

        // After the frame, so the breakdown read back describes one that has been submitted. It blocks on
        // the GPU, which is why it runs about once a second rather than here every time.
        self.report_timings(delta);
        Ok(())
    }
}

/// Maps a physical key to a change of display settings, or `None` if it is not one.
fn display_change(code: KeyCode, current: DisplaySettings) -> Option<DisplaySettings> {
    Some(match code {
        KeyCode::KeyT => current.with_antialiasing(match current.antialiasing {
            Antialiasing::None => Antialiasing::Fxaa,
            Antialiasing::Fxaa => Antialiasing::None,
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
