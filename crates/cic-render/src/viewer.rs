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

use crate::model::{BlendMode, ModelFraming};
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
    let mut application = ViewerApplication::new(model, title, display)?;
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
    framing: ModelFraming,
    animation_started: Instant,
    viewer_started: Instant,
    error: Option<ViewerError>,
}

impl ViewerApplication {
    fn new(
        model: AnimatedModel,
        base_title: String,
        display: OwnedDisplayHandle,
    ) -> Result<Self, ViewerError> {
        let active_animation = (model.animation_count() > 0).then_some(0);
        let framing = model.framing(active_animation)?;
        Ok(Self {
            model,
            base_title,
            display,
            window: None,
            gpu: None,
            active_animation,
            framing,
            animation_started: Instant::now(),
            viewer_started: Instant::now(),
            error: None,
        })
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

    fn switch_animation(&mut self, direction: i32) -> Result<(), ViewerError> {
        let count = self.model.animation_count();
        if count == 0 {
            return Ok(());
        }
        let current = self.active_animation.unwrap_or(0);
        let next = if direction < 0 {
            current.checked_sub(1).unwrap_or(count - 1)
        } else {
            (current + 1) % count
        };
        self.active_animation = Some(next);
        self.framing = self.model.framing(self.active_animation)?;
        self.animation_started = Instant::now();
        if let Some(window) = &self.window {
            window.set_title(&self.window_title());
        }
        Ok(())
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
                    PhysicalKey::Code(KeyCode::ArrowLeft) => {
                        if let Err(error) = self.switch_animation(-1) {
                            self.fail(event_loop, error);
                        }
                    }
                    PhysicalKey::Code(KeyCode::ArrowRight) => {
                        if let Err(error) = self.switch_animation(1) {
                            self.fail(event_loop, error);
                        }
                    }
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
                    gpu.render(
                        &self.model,
                        self.active_animation,
                        frame,
                        rotation,
                        self.framing,
                    )
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
    opaque_pipeline: wgpu::RenderPipeline,
    alpha_pipeline: wgpu::RenderPipeline,
    additive_pipeline: wgpu::RenderPipeline,
    resources: GpuResourceManager,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
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
            source: wgpu::ShaderSource::Wgsl(include_str!("viewer.wgsl").into()),
        });
        let material_layout = create_material_layout(&device);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cic-render viewer pipeline layout"),
            bind_group_layouts: &[Some(&material_layout)],
            immediate_size: 0,
        });
        let opaque_pipeline = create_pipeline(
            &device,
            &shader,
            &pipeline_layout,
            config.format,
            "cic-render opaque pipeline",
            None,
            true,
        );
        let alpha_pipeline = create_pipeline(
            &device,
            &shader,
            &pipeline_layout,
            config.format,
            "cic-render alpha pipeline",
            Some(wgpu::BlendState::ALPHA_BLENDING),
            false,
        );
        let additive_pipeline = create_pipeline(
            &device,
            &shader,
            &pipeline_layout,
            config.format,
            "cic-render additive pipeline",
            Some(additive_blend()),
            false,
        );
        let resources = GpuResourceManager::new(&device, &queue, model, &material_layout)?;
        let vertex_size = u64::try_from(
            model
                .vertex_count()
                .checked_mul(36)
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
            opaque_pipeline,
            alpha_pipeline,
            additive_pipeline,
            resources,
            vertex_buffer,
            index_buffer,
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
        framing: ModelFraming,
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
        let vertices = model.frame_vertex_bytes(animation, frame, rotation, aspect, framing)?;
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
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            for draw in model.draws() {
                let material = self
                    .resources
                    .materials
                    .get(draw.material)
                    .ok_or(RenderError::InvalidMaterial)?;
                let pipeline = match material.blend {
                    BlendMode::Opaque => &self.opaque_pipeline,
                    BlendMode::Alpha => &self.alpha_pipeline,
                    BlendMode::Additive => &self.additive_pipeline,
                };
                let end = draw
                    .first_index
                    .checked_add(draw.index_count)
                    .ok_or(RenderError::GeometryTooLarge)?;
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &material.bind_group, &[]);
                pass.draw_indexed(draw.first_index..end, 0, 0..1);
            }
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

struct GpuTexture {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

struct GpuMaterial {
    _sampler: wgpu::Sampler,
    _uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    blend: BlendMode,
}

struct GpuResourceManager {
    _textures: Vec<GpuTexture>,
    materials: Vec<GpuMaterial>,
}

impl GpuResourceManager {
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        model: &AnimatedModel,
        layout: &wgpu::BindGroupLayout,
    ) -> Result<Self, RenderError> {
        let mut textures = Vec::with_capacity(model.texture_resources().images().len());
        for image in model.texture_resources().images() {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("cic-render managed texture"),
                size: wgpu::Extent3d {
                    width: image.width(),
                    height: image.height(),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                image.rgba(),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(
                        image
                            .width()
                            .checked_mul(4)
                            .ok_or(RenderError::TextureTooLarge)?,
                    ),
                    rows_per_image: Some(image.height()),
                },
                wgpu::Extent3d {
                    width: image.width(),
                    height: image.height(),
                    depth_or_array_layers: 1,
                },
            );
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            textures.push(GpuTexture {
                _texture: texture,
                view,
            });
        }
        let mut materials = Vec::with_capacity(model.materials().len());
        for material in model.materials() {
            let texture = textures
                .get(material.texture.index())
                .ok_or(RenderError::InvalidMaterial)?;
            let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("cic-render managed sampler"),
                address_mode_u: address_mode(material.clamp_u),
                address_mode_v: address_mode(material.clamp_v),
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Nearest,
                ..Default::default()
            });
            let uniform = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("cic-render material uniform"),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let cutoff = if material.alpha_test { 0.5_f32 } else { 0.0 };
            let mut uniform_bytes = [0; 16];
            uniform_bytes[..4].copy_from_slice(&cutoff.to_le_bytes());
            queue.write_buffer(&uniform, 0, &uniform_bytes);
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("cic-render managed material"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&texture.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: uniform.as_entire_binding(),
                    },
                ],
            });
            materials.push(GpuMaterial {
                _sampler: sampler,
                _uniform: uniform,
                bind_group,
                blend: material.blend,
            });
        }
        Ok(Self {
            _textures: textures,
            materials,
        })
    }
}

fn create_material_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cic-render material layout"),
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
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(16),
                },
                count: None,
            },
        ],
    })
}

#[allow(clippy::too_many_arguments)]
fn create_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
    label: &'static str,
    blend: Option<wgpu::BlendState>,
    depth_write_enabled: bool,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vertex_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: 36,
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
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: 28,
                        shader_location: 2,
                    },
                ],
            })],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fragment_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(depth_write_enabled),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn additive_blend() -> wgpu::BlendState {
    wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
    }
}

fn address_mode(clamp: bool) -> wgpu::AddressMode {
    if clamp {
        wgpu::AddressMode::ClampToEdge
    } else {
        wgpu::AddressMode::Repeat
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
