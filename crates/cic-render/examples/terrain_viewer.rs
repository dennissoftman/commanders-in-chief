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
//! | `Esc` | Quit |

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
use cic_render::{
    Action, DeferredFrame, GpuContext, InputState, LayerMaterial, ModelBatch, ModelInstance,
    SurfaceRenderer, TerrainGround, TerrainRenderer, TextureImage,
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

/// Everything created once a window exists.
struct Active {
    window: Arc<Window>,
    context: GpuContext,
    terrain_renderer: TerrainRenderer,
    surface: SurfaceRenderer,
    models: Vec<ModelBatch>,
}

struct Viewer {
    terrain: Terrain,
    camera: RtsCamera,
    input: InputState,
    active: Option<Active>,
    last_frame: Option<Instant>,
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
            failure: None,
        }
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
        ) {
            Ok(surface) => surface,
            Err(error) => return self.fail(event_loop, error.to_string()),
        };
        eprintln!("surface: {:?} at {:?}", surface.format(), surface.size());

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

        self.active = Some(Active {
            window,
            context,
            terrain_renderer,
            surface,
            models,
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

        let (width, height) = active.surface.size();
        let frame = DeferredFrame::new(self.camera.pose(), width, height);
        active
            .surface
            .render(
                &active.context,
                &active.terrain_renderer,
                &active.models,
                frame,
            )
            .map_err(|error| error.to_string())
    }
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
