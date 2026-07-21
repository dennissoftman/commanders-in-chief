//! Renderer boundary and deterministic headless capture support.

mod model;
mod viewer;

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::sync::mpsc;
use std::time::Duration;

use cic_formats::W3dStaticMesh;
use sha2::{Digest, Sha256};

pub use model::{AnimatedModel, StagedModel};
pub use viewer::{ViewerError, run_model_viewer};

const CAPTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const BYTES_PER_PIXEL: u32 = 4;
const MAX_CAPTURE_DIMENSION: u32 = 4_096;
const MAX_CAPTURE_BUFFER_BYTES: u64 = 64 * 1_024 * 1_024;

/// Immutable renderer upload data copied from one validated W3D mesh in stable file order.
#[derive(Debug, Clone, PartialEq)]
pub struct StagedMesh {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    indices: Vec<u32>,
}

impl StagedMesh {
    /// Copies renderer-neutral geometry without retaining parser or filesystem state.
    #[must_use]
    pub fn from_w3d(mesh: &W3dStaticMesh) -> Self {
        let positions = mesh
            .vertices()
            .iter()
            .map(|value| [value.x(), value.y(), value.z()])
            .collect();
        let normals = mesh
            .normals()
            .iter()
            .map(|value| [value.x(), value.y(), value.z()])
            .collect();
        let indices = mesh
            .triangles()
            .iter()
            .flat_map(|triangle| triangle.vertex_indices())
            .collect();
        Self {
            positions,
            normals,
            indices,
        }
    }

    #[must_use]
    pub fn positions(&self) -> &[[f32; 3]] {
        &self.positions
    }

    #[must_use]
    pub fn normals(&self) -> &[[f32; 3]] {
        &self.normals
    }

    #[must_use]
    pub fn indices(&self) -> &[u32] {
        &self.indices
    }
}

/// Explicit column-major pose supplied by the caller for one diagnostic frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    columns: [[f32; 4]; 4],
}

impl Pose {
    pub const IDENTITY: Self = Self {
        columns: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };

    /// Returns a finite clip-space translation pose.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::NonFinitePose`] when either component is not finite.
    pub fn translation(x: f32, y: f32) -> Result<Self, RenderError> {
        if !x.is_finite() || !y.is_finite() {
            return Err(RenderError::NonFinitePose);
        }
        let mut pose = Self::IDENTITY;
        pose.columns[3][0] = x;
        pose.columns[3][1] = y;
        Ok(pose)
    }

    fn bytes(self) -> [u8; 64] {
        let mut bytes = [0; 64];
        for (index, value) in self.columns.into_iter().flatten().enumerate() {
            let offset = index * 4;
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }
}

/// One tightly packed RGBA8 headless frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capture {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl Capture {
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    #[must_use]
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// Returns a stable lowercase SHA-256 digest of the tightly packed RGBA bytes.
    #[must_use]
    pub fn sha256(&self) -> String {
        format!("{:x}", Sha256::digest(&self.rgba))
    }

    /// Encodes a portable binary PPM for local visual inspection. Alpha is omitted.
    #[must_use]
    pub fn ppm(&self) -> Vec<u8> {
        let mut bytes = format!("P6\n{} {}\n255\n", self.width, self.height).into_bytes();
        bytes.reserve(self.rgba.len() / 4 * 3);
        for pixel in self.rgba.chunks_exact(4) {
            bytes.extend_from_slice(&pixel[..3]);
        }
        bytes
    }
}

/// Headless GPU renderer with no window, filesystem, clock, or simulation ownership.
pub struct HeadlessRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    synthetic_pipeline: wgpu::RenderPipeline,
    model_pipeline: wgpu::RenderPipeline,
    pose_buffer: wgpu::Buffer,
    pose_bind_group: wgpu::BindGroup,
    adapter: wgpu::AdapterInfo,
}

impl HeadlessRenderer {
    /// Requests a native adapter and an empty-feature device. A software/fallback adapter is tried
    /// when no default adapter is available.
    ///
    /// # Errors
    ///
    /// Returns a structured adapter or device error when neither a native nor fallback device can
    /// satisfy the empty-feature request.
    #[allow(clippy::too_many_lines)]
    pub async fn new() -> Result<Self, RenderError> {
        let instance = wgpu::Instance::default();
        let mut options = wgpu::RequestAdapterOptions::default();
        let adapter = if let Ok(adapter) = instance.request_adapter(&options).await {
            adapter
        } else {
            options.force_fallback_adapter = true;
            instance
                .request_adapter(&options)
                .await
                .map_err(RenderError::RequestAdapter)?
        };
        let descriptor = wgpu::DeviceDescriptor {
            label: Some("cic-render headless device"),
            ..Default::default()
        };
        let (device, queue) = adapter
            .request_device(&descriptor)
            .await
            .map_err(RenderError::RequestDevice)?;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cic-render synthetic triangle shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let pose_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cic-render pose uniform"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cic-render pose layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(64),
                },
                count: None,
            }],
        });
        let pose_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cic-render pose bind group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: pose_buffer.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cic-render synthetic pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let synthetic_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cic-render synthetic pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: CAPTURE_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let model_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cic-render model shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("model.wgsl").into()),
        });
        let model_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cic-render model pipeline"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &model_shader,
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
                module: &model_shader,
                entry_point: Some("fragment_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: CAPTURE_FORMAT,
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
        Ok(Self {
            device,
            queue,
            synthetic_pipeline,
            model_pipeline,
            pose_buffer,
            pose_bind_group,
            adapter: adapter.get_info(),
        })
    }

    #[must_use]
    pub fn adapter_info(&self) -> &wgpu::AdapterInfo {
        &self.adapter
    }

    /// Renders one explicitly posed synthetic frame and returns tightly packed RGBA8 pixels.
    ///
    /// # Errors
    ///
    /// Returns a structured error for empty or overflowing dimensions, GPU submission/readback
    /// failures, or a readback callback that does not arrive within the bounded wait.
    #[allow(clippy::too_many_lines)]
    pub fn capture_triangle(
        &self,
        width: u32,
        height: u32,
        pose: Pose,
    ) -> Result<Capture, RenderError> {
        let (unpadded_row, padded_row, buffer_size) = capture_layout(width, height)?;
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cic-render headless target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: CAPTURE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cic-render readback"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&self.pose_buffer, 0, &pose.bytes());
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("cic-render headless encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cic-render synthetic pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.synthetic_pipeline);
            pass.set_bind_group(0, &self.pose_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row),
                    rows_per_image: Some(height),
                },
            },
            texture.size(),
        );
        self.finish_capture(encoder, &readback, width, height, unpadded_row, padded_row)
    }

    /// Renders one composed W3D bind pose with deterministic geometry order, orthographic framing,
    /// vertex-material color approximation, and a depth buffer.
    ///
    /// # Errors
    ///
    /// Returns a structured error for invalid capture dimensions, excessive GPU buffer sizes,
    /// device submission/readback failures, or index counts outside the draw API limit.
    #[allow(clippy::too_many_lines)]
    pub fn capture_model(
        &self,
        width: u32,
        height: u32,
        model: &StagedModel,
    ) -> Result<Capture, RenderError> {
        let (unpadded_row, padded_row, buffer_size) = capture_layout(width, height)?;
        let index_count =
            u32::try_from(model.index_count()).map_err(|_| RenderError::GeometryTooLarge)?;
        let (vertex_bytes, index_bytes) = model.gpu_bytes();
        let vertex_size =
            u64::try_from(vertex_bytes.len()).map_err(|_| RenderError::GeometryTooLarge)?;
        let index_size =
            u64::try_from(index_bytes.len()).map_err(|_| RenderError::GeometryTooLarge)?;
        let vertex_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cic-render model vertices"),
            size: vertex_size,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let index_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cic-render model indices"),
            size: index_size,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&vertex_buffer, 0, &vertex_bytes);
        self.queue.write_buffer(&index_buffer, 0, &index_bytes);
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cic-render model target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: CAPTURE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let depth = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cic-render model depth"),
            size: texture.size(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cic-render model readback"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("cic-render model encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cic-render model pass"),
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
            pass.set_pipeline(&self.model_pipeline);
            pass.set_vertex_buffer(0, vertex_buffer.slice(..));
            pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..index_count, 0, 0..1);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row),
                    rows_per_image: Some(height),
                },
            },
            texture.size(),
        );
        self.finish_capture(encoder, &readback, width, height, unpadded_row, padded_row)
    }

    fn finish_capture(
        &self,
        encoder: wgpu::CommandEncoder,
        readback: &wgpu::Buffer,
        width: u32,
        height: u32,
        unpadded_row: u32,
        padded_row: u32,
    ) -> Result<Capture, RenderError> {
        let submission = self.queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        let (sender, receiver) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(Duration::from_secs(10)),
            })
            .map_err(RenderError::Poll)?;
        receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| RenderError::MapCallbackTimeout)?
            .map_err(RenderError::MapBuffer)?;
        let mapped = slice.get_mapped_range().map_err(RenderError::MapRange)?;
        let output_len = usize::try_from(u64::from(unpadded_row) * u64::from(height))
            .map_err(|_| RenderError::CaptureTooLarge)?;
        let mut rgba = Vec::with_capacity(output_len);
        let padded_row = usize::try_from(padded_row).map_err(|_| RenderError::CaptureTooLarge)?;
        let unpadded_row =
            usize::try_from(unpadded_row).map_err(|_| RenderError::CaptureTooLarge)?;
        for row in mapped.chunks_exact(padded_row) {
            rgba.extend_from_slice(&row[..unpadded_row]);
        }
        drop(mapped);
        readback.unmap();
        Ok(Capture {
            width,
            height,
            rgba,
        })
    }
}

fn capture_layout(width: u32, height: u32) -> Result<(u32, u32, u64), RenderError> {
    if width == 0 || height == 0 {
        return Err(RenderError::EmptyCapture);
    }
    if width > MAX_CAPTURE_DIMENSION || height > MAX_CAPTURE_DIMENSION {
        return Err(RenderError::CaptureTooLarge);
    }
    let unpadded_row = width
        .checked_mul(BYTES_PER_PIXEL)
        .ok_or(RenderError::CaptureTooLarge)?;
    let padded_row = unpadded_row
        .checked_add(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT - 1)
        .map(|value| value / wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        .and_then(|value| value.checked_mul(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT))
        .ok_or(RenderError::CaptureTooLarge)?;
    let buffer_size = u64::from(padded_row)
        .checked_mul(u64::from(height))
        .ok_or(RenderError::CaptureTooLarge)?;
    if buffer_size > MAX_CAPTURE_BUFFER_BYTES {
        return Err(RenderError::CaptureTooLarge);
    }
    Ok((unpadded_row, padded_row, buffer_size))
}

/// A bounded renderer initialization, pose, submission, or readback failure.
#[derive(Debug)]
pub enum RenderError {
    RequestAdapter(wgpu::RequestAdapterError),
    RequestDevice(wgpu::RequestDeviceError),
    Poll(wgpu::PollError),
    MapBuffer(wgpu::BufferAsyncError),
    MapRange(wgpu::MapRangeError),
    MapCallbackTimeout,
    NonFinitePose,
    EmptyCapture,
    CaptureTooLarge,
    EmptyModel,
    GeometryTooLarge,
    InvalidHierarchy,
    GeometryOutsideLimits,
    InvalidAnimation,
}

impl Display for RenderError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::RequestAdapter(error) => {
                write!(formatter, "requesting a graphics adapter: {error}")
            }
            Self::RequestDevice(error) => {
                write!(formatter, "requesting a graphics device: {error}")
            }
            Self::Poll(error) => write!(formatter, "waiting for headless rendering: {error}"),
            Self::MapBuffer(error) => write!(formatter, "mapping headless capture: {error}"),
            Self::MapRange(error) => write!(formatter, "reading headless capture: {error}"),
            Self::MapCallbackTimeout => formatter.write_str("headless capture callback timed out"),
            Self::NonFinitePose => formatter.write_str("pose translation must be finite"),
            Self::EmptyCapture => formatter.write_str("capture dimensions must be positive"),
            Self::CaptureTooLarge => formatter.write_str("capture byte size exceeds limits"),
            Self::EmptyModel => formatter.write_str("model contains no renderable triangles"),
            Self::GeometryTooLarge => formatter.write_str("model geometry exceeds renderer limits"),
            Self::InvalidHierarchy => formatter.write_str("model hierarchy references are invalid"),
            Self::GeometryOutsideLimits => {
                formatter.write_str("transformed model geometry is non-finite or outside limits")
            }
            Self::InvalidAnimation => formatter.write_str("animation clip index is invalid"),
        }
    }
}

impl Error for RenderError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::RequestAdapter(error) => Some(error),
            Self::RequestDevice(error) => Some(error),
            Self::Poll(error) => Some(error),
            Self::MapBuffer(error) => Some(error),
            Self::MapRange(error) => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Pose, StagedMesh};
    use cic_formats::{W3dLimits, W3dMeshLimits, decode_static_mesh, parse_w3d};

    #[test]
    fn stages_validated_w3d_geometry_in_file_order() {
        let hex = include_str!("../../cic-formats/tests/fixtures/static-mesh.w3d.hex");
        let digits = hex
            .bytes()
            .filter(u8::is_ascii_hexdigit)
            .collect::<Vec<_>>();
        let bytes = digits
            .chunks_exact(2)
            .map(|pair| {
                let pair = std::str::from_utf8(pair).expect("ASCII fixture");
                u8::from_str_radix(pair, 16).expect("valid fixture")
            })
            .collect::<Vec<_>>();
        let file =
            parse_w3d(&bytes, "static-mesh.w3d", W3dLimits::default()).expect("valid W3D fixture");
        let mesh = decode_static_mesh(&file.chunks()[0], W3dMeshLimits::default())
            .expect("valid static mesh");

        let staged = StagedMesh::from_w3d(&mesh);

        assert_eq!(staged.positions().len(), 3);
        assert_eq!(staged.normals().len(), 3);
        assert_eq!(staged.indices(), &[0, 1, 2]);
    }

    #[test]
    fn pose_rejects_non_finite_translation() {
        assert!(Pose::translation(f32::NAN, 0.0).is_err());
        assert!(Pose::translation(0.0, f32::INFINITY).is_err());
    }
}
