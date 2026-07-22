// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Interactive free-flight presentation for immutable staged terrain.
//!
//! The directional light is a project-authored, viewer-only preview until bounded MAP
//! `LightingData` decoding supplies the source-authored terrain lights. Detail streaming derives
//! nested, depth-capped 16/32-texel screen-space footprints over the stable 8-texel background.
//! Each tier independently cancels stale bakes and overlaps resident replacements, so presentation
//! never performs CPU texture composition synchronously or dilutes foreground detail at the horizon.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, OwnedDisplayHandle};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::terrain::{
    TerrainDetailPatch, TerrainDetailRequest, TerrainMipLevel, generate_srgb_mips,
};
use crate::viewer::{ViewerError, create_depth, nonzero_size};
use crate::{RenderError, StagedTerrain, StagedWater, WaterAppearance};

const WINDOW_WIDTH: u32 = 1_280;
const WINDOW_HEIGHT: u32 = 800;
const CAMERA_UNIFORM_BYTES: u64 = 128;
const MAX_FRAME_SECONDS: f32 = 0.1;
const CAMERA_VERTICAL_FOV: f32 = std::f32::consts::PI / 3.0;
const TERRAIN_CELL_WORLD_SIZE: f32 = 10.0;
const DETAIL_SCREEN_OVERSAMPLE: f32 = 1.5;
const DETAIL_TRANSITION_SECONDS: f32 = 0.12;

/// Opens a perspective terrain viewer with keyboard flight and right-drag mouse look.
///
/// W/S move forward/back, A/D strafe, Space/Ctrl move vertically, Shift boosts speed, right mouse
/// drag looks around, the wheel moves along the view direction, R resets, and Escape closes.
///
/// # Errors
///
/// Returns a structured window, surface, adapter, device, or terrain-resource failure.
pub fn run_terrain_viewer(
    terrain: StagedTerrain,
    water: StagedWater,
    water_appearance: WaterAppearance,
    title: String,
) -> Result<(), ViewerError> {
    let event_loop = EventLoop::new().map_err(ViewerError::EventLoop)?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let display = event_loop.owned_display_handle();
    let mut application =
        TerrainViewerApplication::new(terrain, water, water_appearance, title, display)?;
    event_loop
        .run_app(&mut application)
        .map_err(ViewerError::EventLoop)?;
    application.error.map_or(Ok(()), Err)
}

struct TerrainViewerApplication {
    terrain: Arc<StagedTerrain>,
    water: StagedWater,
    water_appearance: WaterAppearance,
    title: String,
    display: OwnedDisplayHandle,
    window: Option<Arc<Window>>,
    gpu: Option<TerrainViewerGpu>,
    camera: TerrainCamera,
    initial_camera: TerrainCamera,
    detail_requests: Vec<TerrainDetailRequest>,
    detail_workers: Vec<DetailWorker>,
    input: TerrainInput,
    right_drag: bool,
    cursor: Option<PhysicalPosition<f64>>,
    previous_frame: Instant,
    presentation_seconds: f32,
    error: Option<ViewerError>,
}

type DetailResult = (
    TerrainDetailRequest,
    Result<TerrainDetailPatch, crate::TerrainError>,
);

struct DetailWorker {
    requests: Sender<(u64, TerrainDetailRequest)>,
    results: Receiver<DetailResult>,
    requested: TerrainDetailRequest,
    generation: Arc<AtomicU64>,
}

impl DetailWorker {
    fn new(
        terrain: Arc<StagedTerrain>,
        initial_request: TerrainDetailRequest,
    ) -> Result<Self, std::io::Error> {
        let (request_sender, request_receiver) = mpsc::channel();
        let (result_sender, result_receiver) = mpsc::channel();
        let generation = Arc::new(AtomicU64::new(0));
        let worker_generation = generation.clone();
        thread::Builder::new()
            .name("cic-terrain-detail".into())
            .spawn(move || {
                while let Ok((mut request_generation, mut request)) = request_receiver.recv() {
                    while let Ok(newer) = request_receiver.try_recv() {
                        (request_generation, request) = newer;
                    }
                    let result = terrain.detail_patch_controlled(request, || {
                        worker_generation.load(Ordering::Acquire) != request_generation
                    });
                    match result {
                        Ok(Some(patch)) => {
                            if result_sender.send((request, Ok(patch))).is_err() {
                                break;
                            }
                        }
                        Ok(None) => {}
                        Err(error) => {
                            if result_sender.send((request, Err(error))).is_err() {
                                break;
                            }
                        }
                    }
                }
            })?;
        Ok(Self {
            requests: request_sender,
            results: result_receiver,
            requested: initial_request,
            generation,
        })
    }

    fn request(&mut self, request: TerrainDetailRequest) {
        if self.requested.covers(request) {
            return;
        }
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        if self.requests.send((generation, request)).is_ok() {
            self.requested = request;
        }
    }

    fn take_latest(&self) -> Option<DetailResult> {
        let mut latest = None;
        loop {
            match self.results.try_recv() {
                Ok(result) if result.0 == self.requested => latest = Some(result),
                Ok(_) => {}
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return latest,
            }
        }
    }
}

impl TerrainViewerApplication {
    fn new(
        terrain: StagedTerrain,
        water: StagedWater,
        water_appearance: WaterAppearance,
        title: String,
        display: OwnedDisplayHandle,
    ) -> Result<Self, ViewerError> {
        let terrain = Arc::new(terrain);
        let camera = TerrainCamera::for_terrain(&terrain);
        let detail_requests = camera.detail_requests(&terrain, [WINDOW_WIDTH, WINDOW_HEIGHT])?;
        let detail_workers = detail_requests
            .iter()
            .copied()
            .map(|request| DetailWorker::new(terrain.clone(), request))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            terrain,
            water,
            water_appearance,
            title,
            display,
            window: None,
            gpu: None,
            camera,
            initial_camera: camera,
            detail_requests,
            detail_workers,
            input: TerrainInput::default(),
            right_drag: false,
            cursor: None,
            previous_frame: Instant::now(),
            presentation_seconds: 0.0,
            error: None,
        })
    }

    fn initialize(&mut self, event_loop: &ActiveEventLoop) -> Result<(), ViewerError> {
        let title = format!(
            "{} | WASD fly, Space/Ctrl vertical, Shift boost, RMB look, wheel move, R reset, Esc close",
            self.title
        );
        let attributes = Window::default_attributes()
            .with_title(title)
            .with_inner_size(LogicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT));
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .map_err(ViewerError::Window)?,
        );
        let details = self
            .detail_requests
            .iter()
            .copied()
            .map(|request| self.terrain.detail_patch(request))
            .collect::<Result<Vec<_>, _>>()?;
        let gpu = pollster::block_on(TerrainViewerGpu::new(
            window.clone(),
            self.display.clone(),
            &self.terrain,
            &details,
            &self.water,
            &self.water_appearance,
        ))?;
        self.window = Some(window);
        self.gpu = Some(gpu);
        self.previous_frame = Instant::now();
        Ok(())
    }

    fn refresh_detail(&mut self) -> Result<(), ViewerError> {
        for (index, worker) in self.detail_workers.iter().enumerate() {
            if let Some((request, detail)) = worker.take_latest() {
                let detail = detail?;
                if let Some(gpu) = &mut self.gpu {
                    gpu.update_detail(index, &detail, self.presentation_seconds)?;
                }
                self.detail_requests[index] = request;
            }
        }
        if let Some(window) = &self.window {
            let size = nonzero_size(window.inner_size());
            let requests = self
                .camera
                .detail_requests(&self.terrain, [size.width, size.height])?;
            for (worker, request) in self.detail_workers.iter_mut().zip(requests) {
                worker.request(request);
            }
        }
        Ok(())
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: ViewerError) {
        self.error = Some(error);
        event_loop.exit();
    }
}

impl ApplicationHandler for TerrainViewerApplication {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none()
            && let Err(error) = self.initialize(event_loop)
        {
            self.fail(event_loop, error);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self
            .window
            .as_ref()
            .is_none_or(|window| window.id() != window_id)
        {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.resize(size);
                }
            }
            WindowEvent::Focused(false) => {
                self.input = TerrainInput::default();
                self.right_drag = false;
                self.cursor = None;
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                let pressed = event.state == ElementState::Pressed;
                self.input.set(code, pressed);
                if pressed && !event.repeat {
                    match code {
                        KeyCode::Escape => event_loop.exit(),
                        KeyCode::KeyR => self.camera = self.initial_camera,
                        _ => {}
                    }
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Right,
                ..
            } => {
                self.right_drag = state == ElementState::Pressed;
                self.cursor = None;
            }
            WindowEvent::CursorMoved { position, .. } => {
                if self.right_drag
                    && let Some(previous) = self.cursor
                {
                    #[allow(clippy::cast_possible_truncation)]
                    self.camera.rotate(
                        (position.x - previous.x) as f32,
                        (position.y - previous.y) as f32,
                    );
                }
                self.cursor = Some(position);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let amount = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(position) => {
                        #[allow(clippy::cast_possible_truncation)]
                        let y = position.y as f32;
                        y / 80.0
                    }
                };
                self.camera.dolly(amount);
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let seconds = now
                    .duration_since(self.previous_frame)
                    .as_secs_f32()
                    .min(MAX_FRAME_SECONDS);
                self.previous_frame = now;
                self.presentation_seconds += seconds;
                self.camera.update(self.input, seconds);
                let result = self.refresh_detail().and_then(|()| {
                    self.gpu.as_mut().map_or(Ok(()), |gpu| {
                        gpu.render(self.camera, self.presentation_seconds)
                    })
                });
                if let Err(error) = result {
                    self.fail(event_loop, error);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct TerrainInput(u8);

impl TerrainInput {
    const FORWARD: u8 = 1 << 0;
    const BACKWARD: u8 = 1 << 1;
    const LEFT: u8 = 1 << 2;
    const RIGHT: u8 = 1 << 3;
    const UP: u8 = 1 << 4;
    const DOWN: u8 = 1 << 5;
    const BOOST: u8 = 1 << 6;

    fn set(&mut self, code: KeyCode, pressed: bool) {
        let mask = match code {
            KeyCode::KeyW => Self::FORWARD,
            KeyCode::KeyS => Self::BACKWARD,
            KeyCode::KeyA => Self::LEFT,
            KeyCode::KeyD => Self::RIGHT,
            KeyCode::Space => Self::UP,
            KeyCode::ControlLeft | KeyCode::ControlRight => Self::DOWN,
            KeyCode::ShiftLeft | KeyCode::ShiftRight => Self::BOOST,
            _ => return,
        };
        if pressed {
            self.0 |= mask;
        } else {
            self.0 &= !mask;
        }
    }

    const fn active(self, mask: u8) -> bool {
        self.0 & mask != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct TerrainCamera {
    position: [f32; 3],
    yaw: f32,
    pitch: f32,
    move_speed: f32,
    far_plane: f32,
}

impl TerrainCamera {
    fn for_terrain(terrain: &StagedTerrain) -> Self {
        let (minimum, maximum) = terrain.bounds();
        let center = [
            (minimum[0] + maximum[0]) * 0.5,
            (minimum[1] + maximum[1]) * 0.5,
            (minimum[2] + maximum[2]) * 0.5,
        ];
        let horizontal_span = (maximum[0] - minimum[0])
            .max(maximum[1] - minimum[1])
            .max(100.0);
        let distance = horizontal_span * 0.85;
        let position = [
            center[0] - distance * 0.65,
            center[1] - distance * 0.65,
            maximum[2] + distance * 0.55,
        ];
        let direction = subtract(center, position);
        let horizontal = direction[0].hypot(direction[1]);
        Self {
            position,
            yaw: direction[1].atan2(direction[0]),
            pitch: direction[2].atan2(horizontal),
            move_speed: (horizontal_span * 0.35).max(50.0),
            far_plane: (horizontal_span * 20.0).max(10_000.0),
        }
    }

    fn forward(self) -> [f32; 3] {
        let pitch_cosine = self.pitch.cos();
        [
            pitch_cosine * self.yaw.cos(),
            pitch_cosine * self.yaw.sin(),
            self.pitch.sin(),
        ]
    }

    fn update(&mut self, input: TerrainInput, seconds: f32) {
        let forward = self.forward();
        let right = normalize([forward[1], -forward[0], 0.0]);
        let mut movement = [0.0; 3];
        if input.active(TerrainInput::FORWARD) {
            add_scaled(&mut movement, forward, 1.0);
        }
        if input.active(TerrainInput::BACKWARD) {
            add_scaled(&mut movement, forward, -1.0);
        }
        if input.active(TerrainInput::RIGHT) {
            add_scaled(&mut movement, right, 1.0);
        }
        if input.active(TerrainInput::LEFT) {
            add_scaled(&mut movement, right, -1.0);
        }
        if input.active(TerrainInput::UP) {
            movement[2] += 1.0;
        }
        if input.active(TerrainInput::DOWN) {
            movement[2] -= 1.0;
        }
        let length = dot(movement, movement).sqrt();
        if length > f32::EPSILON {
            let multiplier = if input.active(TerrainInput::BOOST) {
                4.0
            } else {
                1.0
            };
            add_scaled(
                &mut self.position,
                movement,
                self.move_speed * multiplier * seconds / length,
            );
        }
    }

    fn rotate(&mut self, delta_x: f32, delta_y: f32) {
        self.yaw -= delta_x * 0.004;
        self.pitch = (self.pitch - delta_y * 0.004).clamp(-1.48, 1.48);
    }

    fn dolly(&mut self, amount: f32) {
        let forward = self.forward();
        add_scaled(&mut self.position, forward, amount * self.move_speed * 0.2);
    }

    #[allow(clippy::cast_precision_loss)]
    fn detail_requests(
        self,
        terrain: &StagedTerrain,
        viewport: [u32; 2],
    ) -> Result<Vec<TerrainDetailRequest>, crate::TerrainError> {
        let aspect = viewport[0] as f32 / viewport[1].max(1) as f32;
        let terrain_bounds = terrain.bounds();
        let Some((full_minimum, full_maximum)) =
            self.viewport_ground_bounds(terrain_bounds, aspect)
        else {
            return Ok(Vec::new());
        };
        let fallback = [
            (full_minimum[0] + full_maximum[0]) * 0.5,
            (full_minimum[1] + full_maximum[1]) * 0.5,
        ];
        let projection_scale =
            viewport[1].max(1) as f32 * TERRAIN_CELL_WORLD_SIZE * DETAIL_SCREEN_OVERSAMPLE
                / (2.0 * (CAMERA_VERTICAL_FOV * 0.5).tan());
        let mut requests = Vec::with_capacity(2);
        for (pixels_per_cell, outer_screen_pixels) in [(16, 8.0_f32), (32, 16.0)] {
            let maximum_distance = projection_scale / outer_screen_pixels;
            let (minimum, maximum) = self
                .viewport_ground_bounds_limited(terrain_bounds, aspect, maximum_distance)
                .unwrap_or((fallback, fallback));
            requests.push(terrain.detail_request_at_density(minimum, maximum, pixels_per_cell)?);
        }
        Ok(requests)
    }

    fn viewport_ground_bounds(
        self,
        terrain_bounds: ([f32; 3], [f32; 3]),
        aspect: f32,
    ) -> Option<([f32; 2], [f32; 2])> {
        self.viewport_ground_bounds_limited(terrain_bounds, aspect, self.far_plane)
    }

    fn viewport_ground_bounds_limited(
        self,
        terrain_bounds: ([f32; 3], [f32; 3]),
        aspect: f32,
        maximum_distance: f32,
    ) -> Option<([f32; 2], [f32; 2])> {
        let (terrain_minimum, terrain_maximum) = terrain_bounds;
        let forward = self.forward();
        let right = normalize(cross(forward, [0.0, 0.0, 1.0]));
        let camera_up = cross(right, forward);
        let tangent = (CAMERA_VERTICAL_FOV * 0.5).tan();
        let mut footprint_minimum = [f32::INFINITY; 2];
        let mut footprint_maximum = [f32::NEG_INFINITY; 2];
        let mut found = false;
        let direction_for = |x: f32, y: f32| {
            let mut direction = forward;
            add_scaled(&mut direction, right, x * tangent * aspect);
            add_scaled(&mut direction, camera_up, y * tangent);
            direction
        };
        let maximum_distance = maximum_distance.min(self.far_plane);
        for x in [-1.0, 0.0, 1.0] {
            let lower = direction_for(x, -1.0);
            let upper = direction_for(x, 1.0);
            for y in [-1.0, -0.5, 0.0, 0.5, 1.0] {
                let direction = direction_for(x, y);
                let direction = normalize(direction);
                if direction[2].abs() <= f32::EPSILON {
                    continue;
                }
                for height in [terrain_minimum[2], terrain_maximum[2]] {
                    let distance = (height - self.position[2]) / direction[2];
                    if !distance.is_finite() || distance <= 0.0 {
                        continue;
                    }
                    let distance = distance.min(maximum_distance);
                    for axis in 0..2 {
                        let coordinate = self.position[axis] + direction[axis] * distance;
                        footprint_minimum[axis] = footprint_minimum[axis].min(coordinate);
                        footprint_maximum[axis] = footprint_maximum[axis].max(coordinate);
                    }
                    found = true;
                }
            }
            let vertical_delta = upper[2] - lower[2];
            if vertical_delta.abs() > f32::EPSILON {
                let horizon_ratio = -lower[2] / vertical_delta;
                if (0.0..=1.0).contains(&horizon_ratio) {
                    let horizon = normalize([
                        lower[0] + (upper[0] - lower[0]) * horizon_ratio,
                        lower[1] + (upper[1] - lower[1]) * horizon_ratio,
                        0.0,
                    ]);
                    for axis in 0..2 {
                        let coordinate = self.position[axis] + horizon[axis] * maximum_distance;
                        footprint_minimum[axis] = footprint_minimum[axis].min(coordinate);
                        footprint_maximum[axis] = footprint_maximum[axis].max(coordinate);
                    }
                    found = true;
                }
            }
        }
        if !found {
            return None;
        }
        let minimum = [
            footprint_minimum[0].max(terrain_minimum[0]),
            footprint_minimum[1].max(terrain_minimum[1]),
        ];
        let maximum = [
            footprint_maximum[0].min(terrain_maximum[0]),
            footprint_maximum[1].min(terrain_maximum[1]),
        ];
        (minimum[0] <= maximum[0] && minimum[1] <= maximum[1]).then_some((minimum, maximum))
    }

    fn view_projection(self, aspect: f32) -> [[f32; 4]; 4] {
        multiply_matrix(
            perspective(CAMERA_VERTICAL_FOV, aspect, 1.0, self.far_plane),
            look_to(self.position, self.forward(), [0.0, 0.0, 1.0]),
        )
    }
}

struct TerrainViewerGpu {
    _instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    edge_pipeline: wgpu::RenderPipeline,
    detail_pipeline: wgpu::RenderPipeline,
    detail_edge_pipeline: wgpu::RenderPipeline,
    lighting_pipeline: wgpu::RenderPipeline,
    composite_pipeline: wgpu::RenderPipeline,
    water_pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    lighting_layout: wgpu::BindGroupLayout,
    composite_layout: wgpu::BindGroupLayout,
    water_layout: wgpu::BindGroupLayout,
    _texture: wgpu::Texture,
    _edge_texture: wgpu::Texture,
    sampler: wgpu::Sampler,
    camera_uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    edge_bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    edge_index_buffer: Option<wgpu::Buffer>,
    index_count: u32,
    edge_index_count: u32,
    details: Vec<TerrainDetailTierGpu>,
    water: Option<WaterGpu>,
    water_appearance: WaterAppearanceGpu,
    deferred: DeferredTargets,
    config: wgpu::SurfaceConfiguration,
    window: Arc<Window>,
}

struct WaterGpu {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
}

struct WaterAppearanceGpu {
    _caustics: wgpu::Texture,
    caustic_view: wgpu::TextureView,
    caustic_sampler: wgpu::Sampler,
    frame_count: u32,
    frames_per_second: u32,
    deep_opacity: f32,
    opaque_depth: f32,
}

impl WaterAppearanceGpu {
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        appearance: &WaterAppearance,
    ) -> Result<Self, ViewerError> {
        let fallback = vec![0_u8];
        let (width, height, frame_count, frames_per_second, frames): (_, _, _, _, &[Vec<u8>]) =
            match appearance.caustics() {
                Some(sequence) => (
                    sequence.width(),
                    sequence.height(),
                    u32::try_from(sequence.frames().len())
                        .map_err(|_| RenderError::TextureTooLarge)?,
                    sequence.frames_per_second(),
                    sequence.frames(),
                ),
                None => (1, 1, 1, 1, std::slice::from_ref(&fallback)),
            };
        let mip_level_count = width.max(height).ilog2() + 1;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cic-render water caustic array"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: frame_count,
            },
            mip_level_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (layer, frame) in frames.iter().enumerate() {
            let layer = u32::try_from(layer).map_err(|_| RenderError::TextureTooLarge)?;
            let mut level_width = width;
            let mut level_height = height;
            let mut level = frame.clone();
            for mip_level in 0..mip_level_count {
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: layer,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    &level,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(level_width),
                        rows_per_image: Some(level_height),
                    },
                    wgpu::Extent3d {
                        width: level_width,
                        height: level_height,
                        depth_or_array_layers: 1,
                    },
                );
                if mip_level + 1 < mip_level_count {
                    let (next_width, next_height, next) =
                        gray_mip(level_width, level_height, &level)?;
                    level_width = next_width;
                    level_height = next_height;
                    level = next;
                }
            }
        }
        let caustic_view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("cic-render water caustic array view"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let caustic_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("cic-render water caustic sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 4,
            ..Default::default()
        });
        Ok(Self {
            _caustics: texture,
            caustic_view,
            caustic_sampler,
            frame_count,
            frames_per_second,
            deep_opacity: appearance.deep_opacity(),
            opaque_depth: appearance.opaque_depth(),
        })
    }
}

fn gray_mip(width: u32, height: u32, source: &[u8]) -> Result<(u32, u32, Vec<u8>), RenderError> {
    let target_width = (width / 2).max(1);
    let target_height = (height / 2).max(1);
    let target_len = usize::try_from(u64::from(target_width) * u64::from(target_height))
        .map_err(|_| RenderError::TextureTooLarge)?;
    let mut target = vec![0_u8; target_len];
    for target_y in 0..target_height {
        let row_start = target_y * height / target_height;
        let row_end = (target_y + 1) * height / target_height;
        for target_x in 0..target_width {
            let column_start = target_x * width / target_width;
            let column_end = (target_x + 1) * width / target_width;
            let mut sum = 0_u32;
            let mut count = 0_u32;
            for source_y in row_start..row_end {
                for source_x in column_start..column_end {
                    let index = usize::try_from(
                        u64::from(source_y) * u64::from(width) + u64::from(source_x),
                    )
                    .map_err(|_| RenderError::TextureTooLarge)?;
                    sum = sum.saturating_add(u32::from(source[index]));
                    count += 1;
                }
            }
            let target_index = usize::try_from(
                u64::from(target_y) * u64::from(target_width) + u64::from(target_x),
            )
            .map_err(|_| RenderError::TextureTooLarge)?;
            target[target_index] = u8::try_from((sum + count / 2) / count)
                .expect("averaged caustic luminance fits u8");
        }
    }
    Ok((target_width, target_height, target))
}

struct DeferredTargets {
    _albedo: wgpu::Texture,
    _normal: wgpu::Texture,
    _world: wgpu::Texture,
    _scene: wgpu::Texture,
    depth: wgpu::Texture,
    albedo_view: wgpu::TextureView,
    normal_view: wgpu::TextureView,
    world_view: wgpu::TextureView,
    scene_view: wgpu::TextureView,
    lighting_bind_group: wgpu::BindGroup,
    composite_bind_group: wgpu::BindGroup,
    water_bind_group: wgpu::BindGroup,
}

struct TerrainDetailGpu {
    _texture: wgpu::Texture,
    _edge_texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    edge_bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    edge_index_buffer: Option<wgpu::Buffer>,
    index_count: u32,
    edge_index_count: u32,
    fade_uv: [f32; 2],
    camera_uniform: wgpu::Buffer,
}

struct TerrainDetailTierGpu {
    current: TerrainDetailGpu,
    previous: Option<TerrainDetailGpu>,
    transition_started: f32,
}

fn upload_mipmapped_terrain_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    width: u32,
    height: u32,
    base_rgba: &[u8],
    mips: &[TerrainMipLevel],
) -> Result<wgpu::Texture, ViewerError> {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: u32::try_from(mips.len())
            .map_err(|_| RenderError::TextureTooLarge)?
            .checked_add(1)
            .ok_or(RenderError::TextureTooLarge)?,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    write_texture_mip(queue, &texture, 0, width, height, base_rgba)?;
    for (index, mip) in mips.iter().enumerate() {
        let level = u32::try_from(index)
            .map_err(|_| RenderError::TextureTooLarge)?
            .checked_add(1)
            .ok_or(RenderError::TextureTooLarge)?;
        write_texture_mip(queue, &texture, level, mip.width, mip.height, &mip.rgba)?;
    }
    Ok(texture)
}

fn write_texture_mip(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    mip_level: u32,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<(), RenderError> {
    let expected = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|texels| texels.checked_mul(4))
        .and_then(|bytes| usize::try_from(bytes).ok())
        .ok_or(RenderError::TextureTooLarge)?;
    if rgba.len() != expected {
        return Err(RenderError::InvalidTexture);
    }
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width.checked_mul(4).ok_or(RenderError::TextureTooLarge)?),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    Ok(())
}

impl TerrainDetailGpu {
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        patch: &TerrainDetailPatch,
    ) -> Result<Self, ViewerError> {
        let camera_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cic-render terrain detail camera"),
            size: CAMERA_UNIFORM_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let texture = upload_mipmapped_terrain_texture(
            device,
            queue,
            "cic-render terrain detail texture",
            patch.texture_width(),
            patch.texture_height(),
            patch.texture_rgba(),
            patch.texture_mips(),
        )?;
        let edge_texture = upload_mipmapped_terrain_texture(
            device,
            queue,
            "cic-render terrain detail edge texture",
            patch.texture_width(),
            patch.texture_height(),
            patch.edge_texture_rgba(),
            patch.edge_texture_mips(),
        )?;
        let bind_group = detail_bind_group(
            device,
            layout,
            &texture,
            sampler,
            &camera_uniform,
            "cic-render terrain detail bind group",
        );
        let edge_bind_group = detail_bind_group(
            device,
            layout,
            &edge_texture,
            sampler,
            &camera_uniform,
            "cic-render terrain detail edge bind group",
        );
        let vertices = patch.vertex_bytes();
        let vertex_buffer = upload_buffer(
            device,
            queue,
            "cic-render terrain detail vertices",
            &vertices,
            wgpu::BufferUsages::VERTEX,
        )?;
        let indices = patch.index_bytes();
        let index_buffer = upload_buffer(
            device,
            queue,
            "cic-render terrain detail indices",
            &indices,
            wgpu::BufferUsages::INDEX,
        )?;
        let edge_index_count = patch.edge_index_count()?;
        let edge_index_buffer = if edge_index_count == 0 {
            None
        } else {
            let bytes = patch.edge_index_bytes();
            Some(upload_buffer(
                device,
                queue,
                "cic-render terrain detail edge indices",
                &bytes,
                wgpu::BufferUsages::INDEX,
            )?)
        };
        Ok(Self {
            _texture: texture,
            _edge_texture: edge_texture,
            bind_group,
            edge_bind_group,
            vertex_buffer,
            index_buffer,
            edge_index_buffer,
            index_count: patch.index_count()?,
            edge_index_count,
            fade_uv: patch.fade_uv(),
            camera_uniform,
        })
    }
}

fn detail_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    texture: &wgpu::Texture,
    sampler: &wgpu::Sampler,
    camera_uniform: &wgpu::Buffer,
    label: &str,
) -> wgpu::BindGroup {
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: camera_uniform.as_entire_binding(),
            },
        ],
    })
}

fn upload_buffer(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    bytes: &[u8],
    usage: wgpu::BufferUsages,
) -> Result<wgpu::Buffer, RenderError> {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: u64::try_from(bytes.len()).map_err(|_| RenderError::GeometryTooLarge)?,
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&buffer, 0, bytes);
    Ok(buffer)
}

impl TerrainViewerGpu {
    #[allow(clippy::too_many_lines)]
    async fn new(
        window: Arc<Window>,
        display: OwnedDisplayHandle,
        terrain: &StagedTerrain,
        details: &[TerrainDetailPatch],
        water: &StagedWater,
        water_appearance: &WaterAppearance,
    ) -> Result<Self, ViewerError> {
        let descriptor = wgpu::InstanceDescriptor::new_with_display_handle(Box::new(display));
        let instance = wgpu::Instance::new(descriptor);
        let surface = instance
            .create_surface(window.clone())
            .map_err(ViewerError::CreateSurface)?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .map_err(RenderError::RequestAdapter)?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("cic-render terrain viewer device"),
                ..Default::default()
            })
            .await
            .map_err(RenderError::RequestDevice)?;
        let size = nonzero_size(window.inner_size());
        let mut config = surface
            .get_default_config(&adapter, size.width, size.height)
            .ok_or(ViewerError::UnsupportedSurface)?;
        config.present_mode = wgpu::PresentMode::Fifo;
        surface.configure(&device, &config);

        let layout = create_terrain_layout(&device);
        let lighting_layout = create_lighting_layout(&device);
        let composite_layout = create_composite_layout(&device);
        let water_layout = create_water_layout(&device);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cic-render terrain viewer shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("terrain_viewer.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cic-render terrain viewer pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = create_terrain_pipeline(
            &device,
            &shader,
            &pipeline_layout,
            "cic-render terrain viewer pipeline",
            "fragment_main",
            None,
            true,
        );
        let edge_pipeline = create_terrain_pipeline(
            &device,
            &shader,
            &pipeline_layout,
            "cic-render terrain viewer edge pipeline",
            "fragment_main",
            Some(wgpu::BlendState::ALPHA_BLENDING),
            false,
        );
        let detail_pipeline = create_terrain_pipeline(
            &device,
            &shader,
            &pipeline_layout,
            "cic-render terrain viewer detail pipeline",
            "detail_fragment_main",
            Some(wgpu::BlendState::ALPHA_BLENDING),
            false,
        );
        let detail_edge_pipeline = create_terrain_pipeline(
            &device,
            &shader,
            &pipeline_layout,
            "cic-render terrain viewer detail edge pipeline",
            "detail_fragment_main",
            Some(wgpu::BlendState::ALPHA_BLENDING),
            false,
        );
        let deferred_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cic-render deferred resolve shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("terrain_deferred.wgsl").into()),
        });
        let lighting_pipeline = create_fullscreen_pipeline(
            &device,
            &deferred_shader,
            &[&lighting_layout],
            "lighting_fragment",
            wgpu::TextureFormat::Rgba16Float,
            "cic-render deferred lighting pipeline",
        );
        let composite_pipeline = create_fullscreen_pipeline(
            &device,
            &deferred_shader,
            &[&lighting_layout, &composite_layout],
            "composite_fragment",
            config.format,
            "cic-render deferred composite pipeline",
        );
        let water_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cic-render modern water shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("water_viewer.wgsl").into()),
        });
        let water_pipeline =
            create_water_pipeline(&device, &water_shader, &water_layout, config.format);

        let texture_mips = generate_srgb_mips(
            terrain.texture_width(),
            terrain.texture_height(),
            terrain.texture_rgba(),
        )?;
        let texture = upload_mipmapped_terrain_texture(
            &device,
            &queue,
            "cic-render terrain viewer texture",
            terrain.texture_width(),
            terrain.texture_height(),
            terrain.texture_rgba(),
            &texture_mips,
        )?;
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("cic-render terrain viewer sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 16,
            ..Default::default()
        });
        let camera_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cic-render terrain viewer camera"),
            size: CAMERA_UNIFORM_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cic-render terrain viewer bind group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: camera_uniform.as_entire_binding(),
                },
            ],
        });
        let edge_texture_mips = generate_srgb_mips(
            terrain.texture_width(),
            terrain.texture_height(),
            terrain.edge_texture_rgba(),
        )?;
        let edge_texture = upload_mipmapped_terrain_texture(
            &device,
            &queue,
            "cic-render terrain viewer edge texture",
            terrain.texture_width(),
            terrain.texture_height(),
            terrain.edge_texture_rgba(),
            &edge_texture_mips,
        )?;
        let edge_view = edge_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let edge_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cic-render terrain viewer edge bind group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&edge_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: camera_uniform.as_entire_binding(),
                },
            ],
        });
        let vertices = terrain.world_vertex_bytes();
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cic-render terrain viewer vertices"),
            size: u64::try_from(vertices.len()).map_err(|_| RenderError::GeometryTooLarge)?,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&vertex_buffer, 0, &vertices);
        let indices = terrain.index_bytes();
        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cic-render terrain viewer indices"),
            size: u64::try_from(indices.len()).map_err(|_| RenderError::GeometryTooLarge)?,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&index_buffer, 0, &indices);
        let index_count =
            u32::try_from(terrain.indices().len()).map_err(|_| RenderError::GeometryTooLarge)?;
        let edge_index_count = u32::try_from(terrain.edge_indices().len())
            .map_err(|_| RenderError::GeometryTooLarge)?;
        let edge_index_buffer = if edge_index_count == 0 {
            None
        } else {
            let bytes = terrain.edge_index_bytes();
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("cic-render terrain viewer edge indices"),
                size: u64::try_from(bytes.len()).map_err(|_| RenderError::GeometryTooLarge)?,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&buffer, 0, &bytes);
            Some(buffer)
        };
        let details = details
            .iter()
            .map(|detail| {
                Ok(TerrainDetailTierGpu {
                    current: TerrainDetailGpu::new(&device, &queue, &layout, &sampler, detail)?,
                    previous: None,
                    transition_started: 0.0,
                })
            })
            .collect::<Result<Vec<_>, ViewerError>>()?;
        let water = if water.indices().is_empty() {
            None
        } else {
            Some(WaterGpu {
                vertex_buffer: upload_buffer(
                    &device,
                    &queue,
                    "cic-render water vertices",
                    &water.vertex_bytes(),
                    wgpu::BufferUsages::VERTEX,
                )?,
                index_buffer: upload_buffer(
                    &device,
                    &queue,
                    "cic-render water indices",
                    &water.index_bytes(),
                    wgpu::BufferUsages::INDEX,
                )?,
                index_count: u32::try_from(water.indices().len())
                    .map_err(|_| RenderError::GeometryTooLarge)?,
            })
        };
        let water_appearance = WaterAppearanceGpu::new(&device, &queue, water_appearance)?;
        let deferred = DeferredTargets::new(
            &device,
            size,
            &lighting_layout,
            &composite_layout,
            &water_layout,
            &camera_uniform,
            &water_appearance,
        );
        Ok(Self {
            _instance: instance,
            surface,
            device,
            queue,
            pipeline,
            edge_pipeline,
            detail_pipeline,
            detail_edge_pipeline,
            lighting_pipeline,
            composite_pipeline,
            water_pipeline,
            layout,
            lighting_layout,
            composite_layout,
            water_layout,
            _texture: texture,
            _edge_texture: edge_texture,
            sampler,
            camera_uniform,
            bind_group,
            edge_bind_group,
            vertex_buffer,
            index_buffer,
            edge_index_buffer,
            index_count,
            edge_index_count,
            details,
            water,
            water_appearance,
            deferred,
            config,
            window,
        })
    }

    fn update_detail(
        &mut self,
        index: usize,
        detail: &TerrainDetailPatch,
        presentation_seconds: f32,
    ) -> Result<(), ViewerError> {
        let replacement = TerrainDetailGpu::new(
            &self.device,
            &self.queue,
            &self.layout,
            &self.sampler,
            detail,
        )?;
        let target = self
            .details
            .get_mut(index)
            .ok_or(RenderError::InvalidTexture)?;
        target.previous = Some(std::mem::replace(&mut target.current, replacement));
        target.transition_started = presentation_seconds;
        Ok(())
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
        self.deferred = DeferredTargets::new(
            &self.device,
            size,
            &self.lighting_layout,
            &self.composite_layout,
            &self.water_layout,
            &self.camera_uniform,
            &self.water_appearance,
        );
    }

    #[allow(clippy::cast_precision_loss, clippy::too_many_lines)]
    fn render(
        &mut self,
        camera: TerrainCamera,
        presentation_seconds: f32,
    ) -> Result<(), ViewerError> {
        if self.window.inner_size().width == 0 || self.window.inner_size().height == 0 {
            return Ok(());
        }
        let (surface_texture, suboptimal) = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture) => (texture, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(texture) => (texture, true),
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Lost => return Err(ViewerError::SurfaceLost),
            wgpu::CurrentSurfaceTexture::Validation => return Err(ViewerError::SurfaceValidation),
        };
        #[allow(clippy::cast_precision_loss)]
        let aspect = self.config.width as f32 / self.config.height as f32;
        #[allow(clippy::cast_precision_loss)]
        let viewport = [self.config.width as f32, self.config.height as f32];
        let matrix = camera.view_projection(aspect);
        let caustic_animation = [
            self.water_appearance.frame_count as f32,
            self.water_appearance.frames_per_second as f32,
        ];
        let water_material = [
            self.water_appearance.deep_opacity,
            self.water_appearance.opaque_depth,
            0.58,
            0.06,
        ];
        self.queue.write_buffer(
            &self.camera_uniform,
            0,
            &camera_bytes(
                matrix,
                camera.position,
                presentation_seconds,
                viewport,
                [0.0; 2],
                caustic_animation,
                water_material,
            ),
        );
        for tier in &mut self.details {
            let transition = tier.previous.as_ref().map_or(1.0, |_| {
                ((presentation_seconds - tier.transition_started) / DETAIL_TRANSITION_SECONDS)
                    .clamp(0.0, 1.0)
            });
            if transition >= 1.0 {
                tier.previous = None;
            }
            if let Some(previous) = &tier.previous {
                self.queue.write_buffer(
                    &previous.camera_uniform,
                    0,
                    &camera_bytes(
                        matrix,
                        camera.position,
                        presentation_seconds,
                        viewport,
                        previous.fade_uv,
                        [1.0, 0.0],
                        water_material,
                    ),
                );
            }
            self.queue.write_buffer(
                &tier.current.camera_uniform,
                0,
                &camera_bytes(
                    matrix,
                    camera.position,
                    presentation_seconds,
                    viewport,
                    tier.current.fade_uv,
                    [transition, 0.0],
                    water_material,
                ),
            );
        }
        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let depth_view = self
            .deferred
            .depth
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("cic-render terrain viewer encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cic-render terrain G-buffer pass"),
                color_attachments: &[
                    Some(clear_attachment(
                        &self.deferred.albedo_view,
                        wgpu::Color::TRANSPARENT,
                    )),
                    Some(clear_attachment(
                        &self.deferred.normal_view,
                        wgpu::Color::TRANSPARENT,
                    )),
                    Some(clear_attachment(
                        &self.deferred.world_view,
                        wgpu::Color::TRANSPARENT,
                    )),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.index_count, 0, 0..1);
            if let Some(edge_index_buffer) = &self.edge_index_buffer {
                pass.set_pipeline(&self.edge_pipeline);
                pass.set_bind_group(0, &self.edge_bind_group, &[]);
                pass.set_index_buffer(edge_index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.edge_index_count, 0, 0..1);
            }
            for tier in &self.details {
                for detail in tier.previous.iter().chain(std::iter::once(&tier.current)) {
                    pass.set_pipeline(&self.detail_pipeline);
                    pass.set_bind_group(0, &detail.bind_group, &[]);
                    pass.set_vertex_buffer(0, detail.vertex_buffer.slice(..));
                    pass.set_index_buffer(detail.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..detail.index_count, 0, 0..1);
                    if let Some(edge_index_buffer) = &detail.edge_index_buffer {
                        pass.set_pipeline(&self.detail_edge_pipeline);
                        pass.set_bind_group(0, &detail.edge_bind_group, &[]);
                        pass.set_index_buffer(
                            edge_index_buffer.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        pass.draw_indexed(0..detail.edge_index_count, 0, 0..1);
                    }
                }
            }
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cic-render deferred lighting pass"),
                color_attachments: &[Some(clear_attachment(
                    &self.deferred.scene_view,
                    wgpu::Color::BLACK,
                ))],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.lighting_pipeline);
            pass.set_bind_group(0, &self.deferred.lighting_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cic-render scene composite pass"),
                color_attachments: &[Some(clear_attachment(&view, wgpu::Color::BLACK))],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.composite_pipeline);
            pass.set_bind_group(0, &self.deferred.lighting_bind_group, &[]);
            pass.set_bind_group(1, &self.deferred.composite_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        if let Some(water) = &self.water {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cic-render forward water pass"),
                color_attachments: &[Some(load_attachment(&view))],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.water_pipeline);
            pass.set_bind_group(0, &self.deferred.water_bind_group, &[]);
            pass.set_vertex_buffer(0, water.vertex_buffer.slice(..));
            pass.set_index_buffer(water.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..water.index_count, 0, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        self.window.pre_present_notify();
        self.queue.present(surface_texture);
        if suboptimal {
            self.surface.configure(&self.device, &self.config);
        }
        Ok(())
    }
}

fn create_terrain_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cic-render terrain viewer layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(CAMERA_UNIFORM_BYTES),
                },
                count: None,
            },
        ],
    })
}

fn create_terrain_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    label: &str,
    fragment_entry: &str,
    blend: Option<wgpu::BlendState>,
    depth_write: bool,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vertex_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: 20,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x3,
                        offset: 0,
                        shader_location: 0,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: 12,
                        shader_location: 1,
                    },
                ],
            })],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment_entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[
                Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    blend,
                    write_mask: wgpu::ColorWrites::ALL,
                }),
                Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba16Float,
                    blend,
                    write_mask: wgpu::ColorWrites::ALL,
                }),
                Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba16Float,
                    blend,
                    write_mask: wgpu::ColorWrites::ALL,
                }),
            ],
        }),
        primitive: wgpu::PrimitiveState {
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: Some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(depth_write),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn create_lighting_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cic-render deferred lighting layout"),
        entries: &[
            texture_layout_entry(0, true),
            texture_layout_entry(1, false),
            texture_layout_entry(2, false),
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(CAMERA_UNIFORM_BYTES),
                },
                count: None,
            },
        ],
    })
}

fn create_composite_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cic-render deferred composite layout"),
        entries: &[texture_layout_entry(0, false)],
    })
}

fn create_water_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cic-render forward water layout"),
        entries: &[
            texture_layout_entry(0, false),
            texture_layout_entry(1, false),
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(CAMERA_UNIFORM_BYTES),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

fn texture_layout_entry(binding: u32, filterable: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn create_fullscreen_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layouts: &[&wgpu::BindGroupLayout],
    fragment_entry: &str,
    format: wgpu::TextureFormat,
    label: &str,
) -> wgpu::RenderPipeline {
    let optional_layouts = layouts.iter().copied().map(Some).collect::<Vec<_>>();
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &optional_layouts,
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("fullscreen_vertex"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment_entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn create_water_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    water_layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("cic-render forward water pipeline layout"),
        bind_group_layouts: &[Some(water_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("cic-render forward water pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("water_vertex"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: 12,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 0,
                }],
            })],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("water_fragment"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

impl DeferredTargets {
    fn new(
        device: &wgpu::Device,
        size: PhysicalSize<u32>,
        lighting_layout: &wgpu::BindGroupLayout,
        composite_layout: &wgpu::BindGroupLayout,
        water_layout: &wgpu::BindGroupLayout,
        camera_uniform: &wgpu::Buffer,
        water_appearance: &WaterAppearanceGpu,
    ) -> Self {
        let albedo = render_texture(
            device,
            size,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            "G-buffer albedo",
        );
        let normal = render_texture(
            device,
            size,
            wgpu::TextureFormat::Rgba16Float,
            "G-buffer normal",
        );
        let world = render_texture(
            device,
            size,
            wgpu::TextureFormat::Rgba16Float,
            "G-buffer world",
        );
        let scene = render_texture(
            device,
            size,
            wgpu::TextureFormat::Rgba16Float,
            "lit scene color",
        );
        let depth = create_depth(device, size);
        let albedo_view = albedo.create_view(&wgpu::TextureViewDescriptor::default());
        let normal_view = normal.create_view(&wgpu::TextureViewDescriptor::default());
        let world_view = world.create_view(&wgpu::TextureViewDescriptor::default());
        let scene_view = scene.create_view(&wgpu::TextureViewDescriptor::default());
        let lighting_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cic-render deferred lighting bind group"),
            layout: lighting_layout,
            entries: &[
                texture_binding(0, &albedo_view),
                texture_binding(1, &normal_view),
                texture_binding(2, &world_view),
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: camera_uniform.as_entire_binding(),
                },
            ],
        });
        let composite_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cic-render deferred composite bind group"),
            layout: composite_layout,
            entries: &[texture_binding(0, &scene_view)],
        });
        let water_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cic-render forward water bind group"),
            layout: water_layout,
            entries: &[
                texture_binding(0, &scene_view),
                texture_binding(1, &world_view),
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: camera_uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&water_appearance.caustic_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&water_appearance.caustic_sampler),
                },
            ],
        });
        Self {
            _albedo: albedo,
            _normal: normal,
            _world: world,
            _scene: scene,
            depth,
            albedo_view,
            normal_view,
            world_view,
            scene_view,
            lighting_bind_group,
            composite_bind_group,
            water_bind_group,
        }
    }
}

fn render_texture(
    device: &wgpu::Device,
    size: PhysicalSize<u32>,
    format: wgpu::TextureFormat,
    label: &str,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size.width,
            height: size.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

fn texture_binding(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    }
}

fn clear_attachment(
    view: &wgpu::TextureView,
    color: wgpu::Color,
) -> wgpu::RenderPassColorAttachment<'_> {
    wgpu::RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: None,
        ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(color),
            store: wgpu::StoreOp::Store,
        },
    }
}

fn load_attachment(view: &wgpu::TextureView) -> wgpu::RenderPassColorAttachment<'_> {
    wgpu::RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: None,
        ops: wgpu::Operations {
            load: wgpu::LoadOp::Load,
            store: wgpu::StoreOp::Store,
        },
    }
}

fn perspective(field_of_view: f32, aspect: f32, near: f32, far: f32) -> [[f32; 4]; 4] {
    let focal = 1.0 / (field_of_view * 0.5).tan();
    [
        [focal / aspect, 0.0, 0.0, 0.0],
        [0.0, focal, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, near * far / (near - far), 0.0],
    ]
}

fn look_to(position: [f32; 3], forward: [f32; 3], up: [f32; 3]) -> [[f32; 4]; 4] {
    let forward = normalize(forward);
    let right = normalize(cross(forward, up));
    let camera_up = cross(right, forward);
    [
        [right[0], camera_up[0], -forward[0], 0.0],
        [right[1], camera_up[1], -forward[1], 0.0],
        [right[2], camera_up[2], -forward[2], 0.0],
        [
            -dot(right, position),
            -dot(camera_up, position),
            dot(forward, position),
            1.0,
        ],
    ]
}

fn multiply_matrix(left: [[f32; 4]; 4], right: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut result = [[0.0; 4]; 4];
    for column in 0..4 {
        for row in 0..4 {
            result[column][row] = (0..4)
                .map(|index| left[index][row] * right[column][index])
                .sum();
        }
    }
    result
}

fn camera_bytes(
    matrix: [[f32; 4]; 4],
    position: [f32; 3],
    time: f32,
    viewport: [f32; 2],
    detail_fade_uv: [f32; 2],
    caustic_animation: [f32; 2],
    water_material: [f32; 4],
) -> [u8; 128] {
    let mut bytes = [0; 128];
    let values = matrix
        .into_iter()
        .flatten()
        .chain([position[0], position[1], position[2], time])
        .chain([
            viewport[0],
            viewport[1],
            1.0 / viewport[0],
            1.0 / viewport[1],
        ])
        .chain([
            detail_fade_uv[0],
            detail_fade_uv[1],
            caustic_animation[0],
            caustic_animation[1],
        ])
        .chain(water_material);
    for (index, value) in values.enumerate() {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn subtract(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn add_scaled(target: &mut [f32; 3], direction: [f32; 3], scale: f32) {
    for axis in 0..3 {
        target[axis] += direction[axis] * scale;
    }
}

fn dot(left: [f32; 3], right: [f32; 3]) -> f32 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn cross(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn normalize(value: [f32; 3]) -> [f32; 3] {
    let length = dot(value, value).sqrt().max(f32::EPSILON);
    [value[0] / length, value[1] / length, value[2] / length]
}

#[cfg(test)]
mod tests {
    use super::{TerrainCamera, TerrainInput, gray_mip, look_to, multiply_matrix, perspective};

    #[test]
    fn caustic_mips_average_odd_linear_frames_without_dropping_edges() {
        let (width, height, mip) = gray_mip(3, 2, &[0, 30, 90, 120, 150, 210]).expect("gray mip");
        assert_eq!((width, height), (1, 1));
        assert_eq!(mip, [100]);
    }

    #[test]
    fn perspective_view_matrix_is_finite_and_movement_uses_explicit_delta() {
        let mut camera = TerrainCamera {
            position: [10.0, 20.0, 30.0],
            yaw: 0.25,
            pitch: -0.5,
            move_speed: 100.0,
            far_plane: 10_000.0,
        };
        let matrix = multiply_matrix(
            perspective(1.0, 16.0 / 9.0, 1.0, camera.far_plane),
            look_to(camera.position, camera.forward(), [0.0, 0.0, 1.0]),
        );
        assert!(matrix.into_iter().flatten().all(f32::is_finite));
        let before = camera.position;
        let mut input = TerrainInput::default();
        input.set(winit::keyboard::KeyCode::KeyW, true);
        camera.update(input, 0.5);
        let distance = camera
            .position
            .into_iter()
            .zip(before)
            .map(|(left, right)| (left - right).powi(2))
            .sum::<f32>()
            .sqrt();
        assert!((distance - 50.0).abs() < 0.001);

        let focus_camera = TerrainCamera {
            position: [10.0, 20.0, 30.0],
            yaw: 0.0,
            pitch: -std::f32::consts::FRAC_PI_4,
            move_speed: 100.0,
            far_plane: 10_000.0,
        };
        for pitch in [-0.000_001, 0.0, 0.000_001] {
            let horizon_camera = TerrainCamera {
                pitch,
                ..focus_camera
            };
            let (minimum, maximum) = horizon_camera
                .viewport_ground_bounds(
                    ([-1_000.0, -1_000.0, 0.0], [1_000.0, 1_000.0, 100.0]),
                    16.0 / 9.0,
                )
                .expect("near-horizon frustum intersects terrain bounds");
            assert!(minimum.into_iter().chain(maximum).all(f32::is_finite));
            assert!(minimum[0] >= -1_000.0 && minimum[1] >= -1_000.0);
            assert!(maximum[0] <= 1_000.0 && maximum[1] <= 1_000.0);
            assert!((maximum[0] - 1_000.0).abs() < 0.001);
        }
    }

    #[test]
    fn shallow_view_detail_footprint_is_capped_before_the_horizon() {
        let camera = TerrainCamera {
            position: [0.0, 0.0, 200.0],
            yaw: 0.0,
            pitch: -0.1,
            move_speed: 100.0,
            far_plane: 10_000.0,
        };
        let terrain = ([-2_000.0, -2_000.0, 0.0], [2_000.0, 2_000.0, 100.0]);
        let (_, full_maximum) = camera
            .viewport_ground_bounds(terrain, 16.0 / 9.0)
            .expect("shallow frustum reaches terrain");
        let (limited_minimum, limited_maximum) = camera
            .viewport_ground_bounds_limited(terrain, 16.0 / 9.0, 650.0)
            .expect("foreground frustum reaches terrain");

        assert!(full_maximum[0] > limited_maximum[0] + 500.0);
        assert!(
            limited_minimum
                .into_iter()
                .chain(limited_maximum)
                .all(|value| { value.is_finite() && (-650.001..=650.001).contains(&value) })
        );
    }
}
