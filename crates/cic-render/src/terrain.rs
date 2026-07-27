//! Terrain rendering: heights and layer weights as writable GPU textures.
//!
//! Nothing about a terrain's shape or surface lives in a vertex buffer. Elevations go into an
//! `R16Uint` texture — the container's own `u16` bytes, copied without conversion — and layer
//! weights into an `R8Unorm` array. The grid itself is procedural in the vertex shader.
//!
//! `R16Uint` rather than `R16Unorm`: the normalized 16-bit formats need an optional device
//! feature, while the integer form is baseline. Nothing is lost, because heights are only ever
//! loaded at exact texel coordinates and never filtered.
//!
//! The reason is future work that is expensive to retrofit and nearly free to design for:
//! a faction that grades roads across the map is editing layer weights, and terrain deformation is
//! editing heights. Both are texture writes here. Had the mesh been baked on the CPU, both would be
//! a remesh plus a buffer upload on every edit.
//!
//! # Layer surfaces are tiled in *world* space
//!
//! Each layer carries an optional albedo image, and the whole set uploads as one array so a fragment
//! can blend up to eight of them without a bind group change. The coordinate they are sampled at is
//! world position divided by the layer's own detail scale — not the terrain's normalized `uv`.
//!
//! That distinction is the difference between a terrain that reads as ground and one that reads as a
//! stretched photograph. Normalized coordinates fit exactly one copy of the image across the entire
//! map, so a 512-pixel grass texture on a two-kilometre map resolves at about four metres per texel
//! and is pure blur at any tactical zoom. A world-space divisor fixes the repeat at a real size — a
//! layer that repeats every thirty-two units looks the same on a small map and a large one, which is
//! also what makes an authored value portable between them.
//!
//! The palette colour survives as a *multiplier* over the sampled texel rather than being replaced by
//! it. A layer with no image multiplies white and comes out exactly as before, so a terrain authored
//! against flat colours renders unchanged, and a greyscale detail texture can be recoloured per map
//! without a second copy of the image.

use cic_assets::Terrain;

use crate::RenderError;
use crate::gpu::{CAPTURE_FORMAT, DEPTH_FORMAT, GpuContext};
use crate::texture::{TextureArray, TextureImage, array_sampler};

/// Largest layer count the forward pass blends, matching `MAX_LAYERS` in the shader.
pub const MAX_LAYERS: usize = 8;

/// Roughness a layer takes when its material does not state one. Terrain is a rough dielectric.
pub const DEFAULT_LAYER_ROUGHNESS: f32 = 0.88;

/// World units one repeat of a layer's albedo covers when its material does not state a scale.
///
/// Roughly the width of a road: fine enough that the repeat is not obvious at a tactical zoom, coarse
/// enough that a strategic view is not sampling the deep end of the mip chain everywhere.
pub const DEFAULT_DETAIL_SCALE: f32 = 32.0;

/// Byte size of the uniform block, which must match the shader's `Uniforms` exactly.
///
/// A mat4x4 (64), five vec4<f32> (80), one vec4<u32> (16), eight vec4<f32> of palette (128), and
/// eight vec4<f32> of per-layer detail parameters (128).
const UNIFORM_BYTES: usize = 64 + 80 + 16 + 128 + 128;

/// A directional light, in the terms the forward pass consumes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DirectionalLight {
    /// Unit direction *toward* the light, in world space.
    pub direction: [f32; 3],
    /// Ambient term, applied regardless of incidence.
    pub ambient: [f32; 3],
    /// Diffuse term, scaled by surface incidence.
    pub diffuse: [f32; 3],
}

impl Default for DirectionalLight {
    /// A late-afternoon sun: high enough to light the ground, low enough that slopes read.
    ///
    /// Two things here are deliberate. The light is not straight overhead, because a vertical light
    /// gives a heightfield near-uniform incidence and flattens it into one colour -- the single
    /// easiest way to make correct terrain look broken. And the ambient term is low: a large ambient
    /// lifts shadowed slopes almost to lit ones, which produces the same flatness by a different
    /// route. Skylight belongs in an ambient-occlusion pass, not in a constant.
    fn default() -> Self {
        Self {
            direction: normalize([-0.45, -0.30, 0.84]),
            ambient: [0.11, 0.13, 0.18],
            diffuse: [1.02, 0.96, 0.86],
        }
    }
}

impl DirectionalLight {
    /// A daylight rig for a pass that computes ambient occlusion.
    ///
    /// The ambient term is markedly higher than [`Self::default`], and that is the point of having
    /// occlusion at all: a constant ambient is a stand-in for skylight, and the reason it normally has
    /// to be kept low is that nothing removes it from creases and hollows where the sky cannot
    /// actually reach. With an occlusion term doing that removal, a realistic amount of skylight
    /// becomes affordable — so unlit faces read as shadowed rather than as black holes.
    #[must_use]
    pub fn daylight_with_occlusion() -> Self {
        Self {
            direction: normalize([-0.45, -0.30, 0.84]),
            ambient: [0.30, 0.34, 0.42],
            diffuse: [1.05, 0.98, 0.86],
        }
    }
}

/// One layer's flat colour, for a caller with no textures to supply.
///
/// A shorthand for a [`LayerMaterial`] with default roughness and detail scale and no image.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerColour(pub [f32; 3]);

/// How one terrain layer's surface is shaded.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerMaterial {
    /// Multiplied into the sampled albedo. `[1.0; 3]` leaves a textured layer as authored; a layer
    /// with no image renders exactly this colour.
    pub colour: [f32; 3],
    /// Surface roughness in `0..=1`, blended across layers by the same weights as the colour.
    pub roughness: f32,
    /// World units one repeat of `albedo` covers. Larger is coarser.
    pub detail_scale: f32,
    /// The layer's albedo image, tiled across the map, or `None` for a flat colour.
    pub albedo: Option<TextureImage>,
}

impl Default for LayerMaterial {
    fn default() -> Self {
        Self {
            colour: [0.5, 0.5, 0.5],
            roughness: DEFAULT_LAYER_ROUGHNESS,
            detail_scale: DEFAULT_DETAIL_SCALE,
            albedo: None,
        }
    }
}

impl LayerMaterial {
    /// A flat-coloured layer with no image.
    #[must_use]
    pub fn colour(colour: [f32; 3]) -> Self {
        Self {
            colour,
            ..Self::default()
        }
    }

    /// Returns the material with an albedo image tiled at a world-space scale.
    #[must_use]
    pub fn with_albedo(mut self, albedo: TextureImage, detail_scale: f32) -> Self {
        self.albedo = Some(albedo);
        self.detail_scale = detail_scale;
        self
    }

    /// Returns the material with a stated roughness.
    #[must_use]
    pub const fn with_roughness(mut self, roughness: f32) -> Self {
        self.roughness = roughness;
        self
    }
}

impl From<LayerColour> for LayerMaterial {
    fn from(colour: LayerColour) -> Self {
        Self::colour(colour.0)
    }
}

/// A terrain uploaded to the GPU, with its pipeline.
#[derive(Debug)]
pub struct TerrainRenderer {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    bind_group_layout: wgpu::BindGroupLayout,
    uniform_buffer: wgpu::Buffer,
    height_texture: wgpu::Texture,
    weight_texture: wgpu::Texture,
    albedo: TextureArray,
    width: u32,
    height: u32,
    horizontal_scale: f32,
    height_scale: f32,
    layer_count: u32,
    /// Per layer: `rgb` colour multiplier, `w` roughness.
    palette: [[f32; 4]; MAX_LAYERS],
    /// Per layer: world units per albedo repeat.
    detail_scale: [f32; MAX_LAYERS],
    height_range: f32,
}

impl TerrainRenderer {
    /// Uploads a terrain with flat layer colours and builds the forward pipeline.
    ///
    /// Equivalent to [`Self::with_materials`] over layers that carry no image.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::TooManyLayers`] when the terrain declares more layers than the forward
    /// pass blends.
    pub fn new(
        context: &GpuContext,
        terrain: &Terrain,
        palette: &[LayerColour],
    ) -> Result<Self, RenderError> {
        let materials: Vec<LayerMaterial> =
            palette.iter().copied().map(LayerMaterial::from).collect();
        Self::with_materials(context, terrain, &materials)
    }

    /// Uploads a terrain with full layer materials and builds the forward pipeline.
    ///
    /// `materials` is matched to the terrain's layers positionally; a layer past the end of the slice
    /// takes [`LayerMaterial::default`], so a caller may supply materials for only the layers it cares
    /// about.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::TooManyLayers`] when the terrain declares more layers than the pass
    /// blends, or a structured texture error when the albedo images exceed their bounds.
    pub fn with_materials(
        context: &GpuContext,
        terrain: &Terrain,
        materials: &[LayerMaterial],
    ) -> Result<Self, RenderError> {
        if terrain.layers().len() > MAX_LAYERS {
            return Err(RenderError::TooManyLayers {
                actual: terrain.layers().len(),
                maximum: MAX_LAYERS,
            });
        }
        let device = context.device();
        let queue = context.queue();
        let width = terrain.width();
        let height = terrain.height();

        let height_texture = upload_heights(device, queue, terrain);
        let weight_texture = upload_weights(device, queue, terrain)?;

        // One slice per weight layer, in the same order, so the shader indexes both with the same
        // number and cannot pair a weight with another layer's surface. A layer with no image takes
        // an opaque-white slice, which multiplies its colour through unchanged.
        let slice_count = terrain.layers().len().max(1);
        let default = LayerMaterial::default();
        let slices: Vec<TextureImage> = (0..slice_count)
            .map(|index| {
                materials
                    .get(index)
                    .unwrap_or(&default)
                    .albedo
                    .clone()
                    .unwrap_or_else(|| TextureImage::solid(1, 1, [u8::MAX; 4]))
            })
            .collect();
        let albedo = TextureArray::new(context, "cic-render terrain layer albedo", &slices)?;

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cic-render terrain uniforms"),
            size: UNIFORM_BYTES as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let (layout, bind_group) = build_bindings(
            device,
            &uniform_buffer,
            &height_texture,
            &weight_texture,
            &albedo,
        );
        let pipeline = build_render_pipeline(device, &layout);

        let mut palette = [[0.5f32, 0.5, 0.5, DEFAULT_LAYER_ROUGHNESS]; MAX_LAYERS];
        let mut detail_scale = [DEFAULT_DETAIL_SCALE; MAX_LAYERS];
        for (index, slot) in palette.iter_mut().enumerate() {
            let material = materials.get(index).unwrap_or(&default);
            *slot = [
                material.colour[0],
                material.colour[1],
                material.colour[2],
                material.roughness,
            ];
            // A zero or negative scale would divide the sampling coordinate to infinity, so it is
            // clamped here rather than guarded in the shader on every fragment.
            detail_scale[index] = if material.detail_scale.is_finite() {
                material.detail_scale.max(1.0e-3)
            } else {
                DEFAULT_DETAIL_SCALE
            };
        }

        Ok(Self {
            pipeline,
            bind_group,
            bind_group_layout: layout,
            uniform_buffer,
            height_texture,
            weight_texture,
            albedo,
            width,
            height,
            horizontal_scale: terrain.horizontal_scale(),
            // An integer texture read returns the stored elevation itself, so the vertical scale
            // applies directly with no normalization to undo.
            height_scale: terrain.vertical_scale(),
            layer_count: u32::try_from(terrain.layers().len()).unwrap_or(0),
            palette,
            detail_scale,
            // Kept so the shadow fit can size each cascade's reach toward the light without the
            // caller having to know or remember to supply it.
            height_range: terrain
                .elevations()
                .iter()
                .copied()
                .max()
                .map_or(0.0, |peak| f32::from(peak) * terrain.vertical_scale()),
        })
    }

    /// Returns the layer albedo array, for reporting what was actually uploaded.
    #[must_use]
    pub const fn layer_albedo(&self) -> &TextureArray {
        &self.albedo
    }

    /// Returns the terrain's bind group, so another pipeline can draw the same terrain.
    ///
    /// The G-buffer and shadow-depth passes need exactly these bindings — the uniforms, the height
    /// texture, and the layer weights — so they share this group rather than duplicating it.
    #[must_use]
    pub const fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }

    /// Returns the layout the bind group was built against, for creating compatible pipelines.
    #[must_use]
    pub const fn bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.bind_group_layout
    }

    /// Returns the world-space elevation of the terrain's highest sample.
    ///
    /// Used to size how far a shadow cascade reaches toward the light, since the tallest thing in the
    /// scene bounds how far away a caster can be.
    #[must_use]
    pub const fn height_range(&self) -> f32 {
        self.height_range
    }

    /// Returns the number of vertices a full terrain draw submits.
    #[must_use]
    pub const fn vertex_count(&self) -> u32 {
        let cells_x = self.width.saturating_sub(1);
        let cells_y = self.height.saturating_sub(1);
        cells_x * cells_y * 6
    }

    /// Overwrites a rectangular region of one layer's weights.
    ///
    /// This is the operation road grading and terrain painting are built on: appearance changes
    /// without touching geometry.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::LayerOutOfRange`] for an unknown layer, or
    /// [`RenderError::RegionOutOfRange`] when the region leaves the terrain or the supplied weights
    /// do not match its area.
    pub fn write_layer_region(
        &self,
        context: &GpuContext,
        layer: u32,
        origin: [u32; 2],
        size: [u32; 2],
        weights: &[u8],
    ) -> Result<(), RenderError> {
        if layer >= self.layer_count.max(1) {
            return Err(RenderError::LayerOutOfRange {
                layer,
                layers: self.layer_count,
            });
        }
        let end_x = origin[0].checked_add(size[0]);
        let end_y = origin[1].checked_add(size[1]);
        let within = end_x.is_some_and(|value| value <= self.width)
            && end_y.is_some_and(|value| value <= self.height);
        let expected = (size[0] as usize).checked_mul(size[1] as usize);
        if !within || size[0] == 0 || size[1] == 0 || expected != Some(weights.len()) {
            return Err(RenderError::RegionOutOfRange {
                origin,
                size,
                terrain: [self.width, self.height],
            });
        }
        context.queue().write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.weight_texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: origin[0],
                    y: origin[1],
                    z: layer,
                },
                aspect: wgpu::TextureAspect::All,
            },
            weights,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size[0]),
                rows_per_image: Some(size[1]),
            },
            wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
        );
        Ok(())
    }

    /// Overwrites a rectangular region of elevations.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::RegionOutOfRange`] when the region leaves the terrain or the supplied
    /// elevations do not match its area.
    pub fn write_height_region(
        &self,
        context: &GpuContext,
        origin: [u32; 2],
        size: [u32; 2],
        elevations: &[u16],
    ) -> Result<(), RenderError> {
        let end_x = origin[0].checked_add(size[0]);
        let end_y = origin[1].checked_add(size[1]);
        let within = end_x.is_some_and(|value| value <= self.width)
            && end_y.is_some_and(|value| value <= self.height);
        let expected = (size[0] as usize).checked_mul(size[1] as usize);
        if !within || size[0] == 0 || size[1] == 0 || expected != Some(elevations.len()) {
            return Err(RenderError::RegionOutOfRange {
                origin,
                size,
                terrain: [self.width, self.height],
            });
        }
        let mut bytes = Vec::with_capacity(elevations.len() * 2);
        for elevation in elevations {
            bytes.extend_from_slice(&elevation.to_le_bytes());
        }
        context.queue().write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.height_texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: origin[0],
                    y: origin[1],
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size[0] * 2),
                rows_per_image: Some(size[1]),
            },
            wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
        );
        Ok(())
    }

    /// Uploads the per-frame uniform block.
    pub fn set_frame(
        &self,
        context: &GpuContext,
        view_projection: &[[f32; 4]; 4],
        camera_position: [f32; 3],
        light: DirectionalLight,
    ) {
        let mut bytes = Vec::with_capacity(UNIFORM_BYTES);
        for column in view_projection {
            for value in column {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        let push_vec4 = |values: [f32; 4], target: &mut Vec<u8>| {
            for value in values {
                target.extend_from_slice(&value.to_le_bytes());
            }
        };
        let [cx, cy, cz] = camera_position;
        push_vec4([cx, cy, cz, 0.0], &mut bytes);
        let [lx, ly, lz] = normalize(light.direction);
        push_vec4([lx, ly, lz, 0.0], &mut bytes);
        let [ar, ag, ab] = light.ambient;
        push_vec4([ar, ag, ab, 0.0], &mut bytes);
        let [dr, dg, db] = light.diffuse;
        push_vec4([dr, dg, db, 0.0], &mut bytes);
        // Terrain dimensions are bounded by the asset limits at 8,192, well inside exact f32 range.
        #[allow(clippy::cast_precision_loss)]
        push_vec4(
            [
                self.horizontal_scale,
                self.height_scale,
                self.width as f32,
                self.height as f32,
            ],
            &mut bytes,
        );
        for value in [self.layer_count, 0, 0, 0] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for entry in self.palette {
            push_vec4(entry, &mut bytes);
        }
        for scale in self.detail_scale {
            push_vec4([scale, 0.0, 0.0, 0.0], &mut bytes);
        }
        debug_assert_eq!(bytes.len(), UNIFORM_BYTES, "uniform block size drifted");
        context
            .queue()
            .write_buffer(&self.uniform_buffer, 0, &bytes);
    }

    /// Records the terrain draw into an existing render pass.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..self.vertex_count(), 0..1);
    }
}

/// Builds the terrain bind group layout.
///
/// Fixed rather than derived from a shader, so a binding that drifts out of agreement with the Rust
/// side fails at pipeline creation rather than rendering something wrong. Every terrain pipeline —
/// forward, G-buffer, and the four shadow cascades — is built against this one layout and binds this
/// one group, which is what keeps them from disagreeing about the terrain they are drawing.
fn build_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cic-render terrain layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                // Read in the vertex stage for displacement and for the normal differences.
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Uint,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 5,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

/// Builds the terrain bind group and the layout it was made against.
fn build_bindings(
    device: &wgpu::Device,
    uniform_buffer: &wgpu::Buffer,
    height_texture: &wgpu::Texture,
    weight_texture: &wgpu::Texture,
    albedo: &TextureArray,
) -> (wgpu::BindGroupLayout, wgpu::BindGroup) {
    // Two samplers, because the two arrays want opposite behaviour. Weights are a per-map field
    // addressed in normalized coordinates and must clamp at the edge; albedo is a detail texture
    // addressed in world units and must repeat, with the mip chain filtered between levels.
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("cic-render terrain weight sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let albedo_sampler = array_sampler(device, "cic-render terrain albedo sampler");
    let layout = build_bind_group_layout(device);

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("cic-render terrain bindings"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(
                    &height_texture.create_view(&wgpu::TextureViewDescriptor::default()),
                ),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&weight_texture.create_view(
                    &wgpu::TextureViewDescriptor {
                        dimension: Some(wgpu::TextureViewDimension::D2Array),
                        ..Default::default()
                    },
                )),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(albedo.view()),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::Sampler(&albedo_sampler),
            },
        ],
    });
    (layout, bind_group)
}

/// Builds the forward render pipeline against a bind group layout.
fn build_render_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("cic-render terrain forward shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("terrain_forward.wgsl").into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("cic-render terrain pipeline layout"),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("cic-render terrain forward pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vertex_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            // Terrain is viewed from above but a low camera can see a far slope's back face, and
            // culling it would punch holes in ridgelines. Two-sided, with the shader flipping
            // any normal that faces away.
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
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
        multiview_mask: None,
        cache: None,
    })
}

/// Uploads elevations into an integer height texture.
fn upload_heights(device: &wgpu::Device, queue: &wgpu::Queue, terrain: &Terrain) -> wgpu::Texture {
    let width = terrain.width();
    let height = terrain.height();
    let height_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("cic-render terrain heights"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        // `R16Uint` rather than `R16Unorm`: the normalized form needs an optional device
        // feature, while the integer form is baseline. Elevations are only ever loaded, not
        // filtered, so integer sampling costs nothing -- and it makes the upload a straight
        // copy of the container's own `u16` bytes. `COPY_DST` keeps a later edit a partial
        // write rather than a full re-upload.
        format: wgpu::TextureFormat::R16Uint,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let mut height_bytes = Vec::with_capacity(terrain.elevations().len() * 2);
    for elevation in terrain.elevations() {
        height_bytes.extend_from_slice(&elevation.to_le_bytes());
    }
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &height_texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &height_bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 2),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    height_texture
}

/// Uploads per-layer weights into an array texture.
///
/// # Errors
///
/// Returns [`RenderError::TooManyLayers`] when the layer count does not fit an array texture.
fn upload_weights(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    terrain: &Terrain,
) -> Result<wgpu::Texture, RenderError> {
    let width = terrain.width();
    let height = terrain.height();
    // An array texture must have at least one layer even when the terrain has none, because the
    // bind group layout is fixed. An unpainted terrain gets one fully-zero layer and the shader's
    // neutral fallback covers it.
    let layer_count = terrain.layers().len().max(1);
    let weight_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("cic-render terrain layer weights"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: u32::try_from(layer_count).map_err(|_| {
                RenderError::TooManyLayers {
                    actual: layer_count,
                    maximum: MAX_LAYERS,
                }
            })?,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let empty = vec![0u8; (width as usize) * (height as usize)];
    for index in 0..layer_count {
        let weights = terrain
            .layers()
            .get(index)
            .map_or(empty.as_slice(), |layer| layer.weights.as_slice());
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &weight_texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: 0,
                    z: u32::try_from(index).unwrap_or(0),
                },
                aspect: wgpu::TextureAspect::All,
            },
            weights,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }
    Ok(weight_texture)
}

fn normalize(vector: [f32; 3]) -> [f32; 3] {
    let length = (vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]).sqrt();
    if length > 0.0 && length.is_finite() {
        [vector[0] / length, vector[1] / length, vector[2] / length]
    } else {
        [0.0, 0.0, 1.0]
    }
}
