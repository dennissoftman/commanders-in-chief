// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only
//
// Presentation integration follows the public winit 0.30 ApplicationHandler and wgpu 30 surface
// APIs. No upstream implementation source was copied or translated.

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::sync::Arc;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, OwnedDisplayHandle};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::{AnimatedModel, RenderError};

const WINDOW_WIDTH: u32 = 960;
const WINDOW_HEIGHT: u32 = 720;
const ROTATION_RADIANS_PER_SECOND: f64 = 0.45;

/// Opens a medium-sized interactive viewer and runs until the window closes.
///
/// The model is framed orthographically from a 45-degree elevated camera and rotates about W3D's
/// Z-up axis. Left/Right select clips and Escape closes the viewer.
///
/// # Errors
///
/// Returns a structured window, surface, adapter, device, or animation staging failure.
pub fn run_model_viewer(model: AnimatedModel, title: String) -> Result<(), ViewerError> {
    let event_loop = EventLoop::new().map_err(ViewerError::EventLoop)?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let display = event_loop.owned_display_handle();
    let mut application = ViewerApplication::new(model, title, display);
    event_loop
        .run_app(&mut application)
        .map_err(ViewerError::EventLoop)?;
    application.error.map_or(Ok(()), Err)
}

struct ViewerApplication {
    model: AnimatedModel,
    base_title: String,
    display: OwnedDisplayHandle,
    window: Option<Arc<Window>>,
    gpu: Option<ViewerGpu>,
    active_animation: Option<usize>,
    animation_started: Instant,
    viewer_started: Instant,
    error: Option<ViewerError>,
}

impl ViewerApplication {
    fn new(model: AnimatedModel, base_title: String, display: OwnedDisplayHandle) -> Self {
        let active_animation = (model.animation_count() > 0).then_some(0);
        Self {
            model,
            base_title,
            display,
            window: None,
            gpu: None,
            active_animation,
            animation_started: Instant::now(),
            viewer_started: Instant::now(),
            error: None,
        }
    }

    fn initialize(&mut self, event_loop: &ActiveEventLoop) -> Result<(), ViewerError> {
        let attributes = Window::default_attributes()
            .with_title(self.window_title())
            .with_inner_size(LogicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT));
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .map_err(ViewerError::Window)?,
        );
        let gpu = pollster::block_on(ViewerGpu::new(
            window.clone(),
            self.display.clone(),
            &self.model,
        ))?;
        self.window = Some(window);
        self.gpu = Some(gpu);
        Ok(())
    }

    fn window_title(&self) -> String {
        match self.active_animation {
            Some(index) => {
                let name = self
                    .model
                    .animation_name(index)
                    .unwrap_or_else(|| "unnamed".to_owned());
                format!(
                    "{} — animation {}/{}: {} — Left/Right switch, Esc closes",
                    self.base_title,
                    index + 1,
                    self.model.animation_count(),
                    name
                )
            }
            None => format!(
                "{} — bind pose (no animations) — Esc closes",
                self.base_title
            ),
        }
    }

    fn switch_animation(&mut self, direction: i32) {
        let count = self.model.animation_count();
        if count == 0 {
            return;
        }
        let current = self.active_animation.unwrap_or(0);
        let next = if direction < 0 {
            current.checked_sub(1).unwrap_or(count - 1)
        } else {
            (current + 1) % count
        };
        self.active_animation = Some(next);
        self.animation_started = Instant::now();
        if let Some(window) = &self.window {
            window.set_title(&self.window_title());
        }
    }

    fn animation_frame(&self) -> u32 {
        let Some(index) = self.active_animation else {
            return 0;
        };
        let frame_rate = self.model.animation_frame_rate(index).unwrap_or(0);
        let frame_count = self.model.animation_frame_count(index).unwrap_or(0);
        if frame_rate == 0 || frame_count == 0 {
            return 0;
        }
        let elapsed = self.animation_started.elapsed().as_secs_f64();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let frame = (elapsed * f64::from(frame_rate)).floor() as u64;
        u32::try_from(frame % u64::from(frame_count)).expect("frame is modulo a u32 count")
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: ViewerError) {
        self.error = Some(error);
        event_loop.exit();
    }
}

impl ApplicationHandler for ViewerApplication {
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
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && !event.repeat =>
            {
                match event.physical_key {
                    PhysicalKey::Code(KeyCode::ArrowLeft) => self.switch_animation(-1),
                    PhysicalKey::Code(KeyCode::ArrowRight) => self.switch_animation(1),
                    PhysicalKey::Code(KeyCode::Escape) => event_loop.exit(),
                    _ => {}
                }
            }
            WindowEvent::RedrawRequested => {
                let frame = self.animation_frame();
                let rotation =
                    self.viewer_started.elapsed().as_secs_f64() * ROTATION_RADIANS_PER_SECOND;
                #[allow(clippy::cast_possible_truncation)]
                let rotation = rotation as f32;
                let result = self.gpu.as_mut().map_or(Ok(()), |gpu| {
                    gpu.render(&self.model, self.active_animation, frame, rotation)
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

struct ViewerGpu {
    _instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    depth: wgpu::Texture,
    config: wgpu::SurfaceConfiguration,
    window: Arc<Window>,
}

impl ViewerGpu {
    #[allow(clippy::too_many_lines)]
    async fn new(
        window: Arc<Window>,
        display: OwnedDisplayHandle,
        model: &AnimatedModel,
    ) -> Result<Self, ViewerError> {
        let descriptor = wgpu::InstanceDescriptor::new_with_display_handle(Box::new(display));
        let instance = wgpu::Instance::new(descriptor);
        let surface = instance
            .create_surface(window.clone())
            .map_err(ViewerError::CreateSurface)?;
        let options = wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
            ..Default::default()
        };
        let adapter = instance
            .request_adapter(&options)
            .await
            .map_err(RenderError::RequestAdapter)?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("cic-render viewer device"),
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
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cic-render viewer shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("model.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cic-render viewer pipeline"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 28,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 12,
                            shader_location: 1,
                        },
                    ],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
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
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let vertex_size = u64::try_from(
            model
                .vertex_count()
                .checked_mul(28)
                .ok_or(RenderError::GeometryTooLarge)?,
        )
        .map_err(|_| RenderError::GeometryTooLarge)?;
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cic-render viewer vertices"),
            size: vertex_size,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let index_bytes = model.index_bytes();
        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cic-render viewer indices"),
            size: u64::try_from(index_bytes.len()).map_err(|_| RenderError::GeometryTooLarge)?,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&index_buffer, 0, &index_bytes);
        let depth = create_depth(&device, size);
        Ok(Self {
            _instance: instance,
            surface,
            device,
            queue,
            pipeline,
            vertex_buffer,
            index_buffer,
            index_count: u32::try_from(model.index_count())
                .map_err(|_| RenderError::GeometryTooLarge)?,
            depth,
            config,
            window,
        })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
        self.depth = create_depth(&self.device, size);
    }

    fn render(
        &mut self,
        model: &AnimatedModel,
        animation: Option<usize>,
        frame: u32,
        rotation: f32,
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
        let vertices = model.frame_vertex_bytes(animation, frame, rotation, aspect)?;
        self.queue.write_buffer(&self.vertex_buffer, 0, &vertices);
        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let depth_view = self
            .depth
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("cic-render viewer encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cic-render viewer pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.015,
                            g: 0.02,
                            b: 0.03,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.index_count, 0, 0..1);
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

fn nonzero_size(size: PhysicalSize<u32>) -> PhysicalSize<u32> {
    PhysicalSize::new(size.width.max(1), size.height.max(1))
}

fn create_depth(device: &wgpu::Device, size: PhysicalSize<u32>) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("cic-render viewer depth"),
        size: wgpu::Extent3d {
            width: size.width.max(1),
            height: size.height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    })
}

/// One interactive viewer initialization or presentation failure.
#[derive(Debug)]
pub enum ViewerError {
    Render(RenderError),
    EventLoop(winit::error::EventLoopError),
    Window(winit::error::OsError),
    CreateSurface(wgpu::CreateSurfaceError),
    UnsupportedSurface,
    SurfaceLost,
    SurfaceValidation,
}

impl From<RenderError> for ViewerError {
    fn from(error: RenderError) -> Self {
        Self::Render(error)
    }
}

impl Display for ViewerError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Render(error) => write!(formatter, "rendering interactive W3D: {error}"),
            Self::EventLoop(error) => write!(formatter, "running viewer event loop: {error}"),
            Self::Window(error) => write!(formatter, "creating viewer window: {error}"),
            Self::CreateSurface(error) => write!(formatter, "creating GPU surface: {error}"),
            Self::UnsupportedSurface => {
                formatter.write_str("graphics adapter cannot present to the viewer surface")
            }
            Self::SurfaceLost => formatter.write_str("viewer GPU surface was lost"),
            Self::SurfaceValidation => {
                formatter.write_str("viewer GPU surface reported a validation error")
            }
        }
    }
}

impl Error for ViewerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Render(error) => Some(error),
            Self::EventLoop(error) => Some(error),
            Self::Window(error) => Some(error),
            Self::CreateSurface(error) => Some(error),
            _ => None,
        }
    }
}
