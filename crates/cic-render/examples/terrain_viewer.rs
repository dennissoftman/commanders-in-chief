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

use cic_assets::model::{Model, ModelMaterial, ModelPrimitive, ModelVertex};
use cic_assets::{MapPackage, PackageLimits, Terrain, TerrainLayer};
use cic_camera::{RtsCamera, RtsCameraProfile};
use cic_render::terrain::LayerColour;
use cic_render::{
    Action, DeferredFrame, GpuContext, InputState, ModelBatch, ModelInstance, SurfaceRenderer,
    TerrainGround, TerrainRenderer,
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
                    uv: [(corner & 1) as f32, ((corner >> 1) & 1) as f32],
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
                base_color_texture: None,
                blended: false,
            },
            ModelMaterial {
                name: "roof".to_owned(),
                base_color: [0.36, 0.20, 0.16, 1.0],
                metallic: 0.0,
                roughness: 0.65,
                base_color_texture: None,
                blended: false,
            },
        ],
        has_skin: false,
        has_animation: false,
    }
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

fn palette() -> Vec<LayerColour> {
    vec![
        LayerColour([0.74, 0.68, 0.50]),
        LayerColour([0.30, 0.42, 0.22]),
        LayerColour([0.48, 0.46, 0.43]),
    ]
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

        let terrain_renderer = match TerrainRenderer::new(&context, &self.terrain, &palette()) {
            Ok(renderer) => renderer,
            Err(error) => return self.fail(event_loop, error.to_string()),
        };
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
