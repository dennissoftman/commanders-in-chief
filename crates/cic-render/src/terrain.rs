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

use std::ops::Range;

use cic_assets::Terrain;
use cic_assets::texture::TextureAsset;

use crate::RenderError;
use crate::culling::{CHUNK_CELLS, ChunkGrid};
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
/// A mat4x4 (64), five vec4<f32> (80), one vec4<u32> (16), eight vec4<f32> of palette (128), eight
/// vec4<f32> of per-layer detail parameters (128), one vec4<f32> of wind and the two scene times (16),
/// one vec4<f32> of the projection jitter (16), the previous frame's view-projection (64), and one
/// vec4<u32> of virtual-texture state (16).
///
/// Everything a geometry pass needs to write a motion vector is in the middle three of the last four
/// entries, and the whole tail is **appended** rather than inserted because `model_gbuffer.wgsl` and
/// `terrain_forward.wgsl` bind this same buffer through structs declaring only a prefix of it. That holds
/// exactly as long as anything they do not read comes after what they do. The same rule already governs the
/// scene block in [`crate::deferred`], for the same reason.
const UNIFORM_BYTES: usize = 64 + 80 + 16 + 128 + 128 + 16 + 16 + 64 + 16;

/// Wind, scene time, and the previous frame's, as a geometry pass needs them.
///
/// Separate from [`crate::environment::Weather`] because a vertex shader wants the numbers a displacement
/// is computed from and nothing else, and because this has to reach a pass that binds the *terrain*
/// uniform rather than the scene one. See [`crate::scenery`] for what reads it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Animation {
    /// World-space wind vector, in world units per second.
    pub wind: [f32; 2],
    /// Scene time in seconds.
    ///
    /// A frame parameter, never a clock reading — the same constraint [`crate::DeferredFrame::time`]
    /// carries, and for the same reason: a capture of a moving surface is only a usable regression
    /// reference if the same inputs produce the same image.
    pub time: f32,
    /// The previous frame's scene time.
    ///
    /// What makes a motion vector correct for swaying geometry, and it costs nothing to provide: the
    /// displacement is a pure function of time, so evaluating it at the previous time returns exactly
    /// where the vertex was. No per-vertex history buffer, and no way for the motion vector to disagree
    /// with what the geometry did.
    ///
    /// Equal to [`Self::time`] for the first frame of a sequence, which reports no motion — correct,
    /// since there is no previous frame to have moved from.
    pub previous_time: f32,
}

/// What a geometry pass needs to say where each fragment was last frame.
///
/// # Why the jitter travels with it
///
/// A motion vector has to be the *unjittered* screen displacement. The rasterized position is jittered by
/// construction — that is the whole mechanism — so the vector has to have the jitter removed from it, or
/// the temporal resolve would sample its history at a position offset by the very sub-pixel shake it is
/// there to average out, and the accumulation would never converge.
///
/// Removing it needs no second matrix, though. A sub-pixel jitter is a translation in clip space
/// proportional to `w`, so `clip.xy - jitter * clip.w` recovers the unjittered position exactly. That is
/// why this carries two floats rather than a second view-projection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Motion {
    /// The previous frame's *unjittered* view-projection, column-major.
    pub previous_view_projection: [[f32; 4]; 4],
    /// This frame's sub-pixel jitter as a clip-space offset — pixels converted to normalized units.
    pub jitter: [f32; 2],
}

impl Motion {
    /// No jitter and no movement: the previous view is this one.
    ///
    /// What every pass that does not resolve temporally uses, and what makes the motion target read as
    /// exactly zero — which is the honest answer for a frame with no predecessor, rather than a small
    /// wrong one.
    #[must_use]
    pub const fn still(view_projection: &[[f32; 4]; 4]) -> Self {
        Self {
            previous_view_projection: *view_projection,
            jitter: [0.0, 0.0],
        }
    }
}

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
    /// The layer's detail texture, tiled across the map. See [`LayerAlbedo`].
    pub albedo: LayerAlbedo,
}

/// What a terrain layer's surface is, and therefore how it reaches the GPU.
///
/// # Why an enum here and an override table for models
///
/// A model's images are read through three material slots and by several existing callers, so widening
/// the image type there would have made every reader branch — and the branch is a decode. A terrain
/// layer's albedo is read in exactly one place, [`TerrainRenderer::with_materials`], and written by
/// exactly one kind of caller: whoever decides what a layer looks like. Naming the three possibilities
/// makes the mixed case unrepresentable per layer, which is what the array actually needs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum LayerAlbedo {
    /// No texture: the layer renders as its flat palette colour.
    #[default]
    None,
    /// An RGBA8 image, resampled and mipped at upload.
    ///
    /// What procedural and placeholder content uses, and what a layer whose texture has not been
    /// converted still uses.
    Image(TextureImage),
    /// A block-compressed texture with its mip chain already in it.
    ///
    /// The fast path: these blocks reach the texture unit unchanged. See
    /// [`cic_assets::resolve_terrain_textures`] for where one comes from.
    Blocks(TextureAsset),
}

impl LayerAlbedo {
    /// Whether this layer has a texture at all.
    #[must_use]
    pub const fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    /// The compressed texture, when that is what this is.
    #[must_use]
    pub const fn blocks(&self) -> Option<&TextureAsset> {
        match self {
            Self::Blocks(asset) => Some(asset),
            _ => None,
        }
    }

    /// This layer's surface as an RGBA8 image, decoding a compressed one if that is what it holds.
    ///
    /// `None` only for [`Self::None`], which the caller fills with an opaque-white slice so the layer's
    /// palette colour multiplies through unchanged.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::InvalidTexture`] when a decoded texture's dimensions and byte length
    /// disagree, which a `TextureAsset` already refuses at construction.
    pub fn to_image(&self) -> Result<Option<TextureImage>, RenderError> {
        match self {
            Self::None => Ok(None),
            Self::Image(image) => Ok(Some(image.clone())),
            Self::Blocks(asset) => {
                TextureImage::new(asset.width(), asset.height(), asset.decode()).map(Some)
            }
        }
    }
}

impl Default for LayerMaterial {
    fn default() -> Self {
        Self {
            colour: [0.5, 0.5, 0.5],
            roughness: DEFAULT_LAYER_ROUGHNESS,
            detail_scale: DEFAULT_DETAIL_SCALE,
            albedo: LayerAlbedo::None,
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

    /// Returns the material with an RGBA8 albedo image tiled at a world-space scale.
    #[must_use]
    pub fn with_albedo(mut self, albedo: TextureImage, detail_scale: f32) -> Self {
        self.albedo = LayerAlbedo::Image(albedo);
        self.detail_scale = detail_scale;
        self
    }

    /// Returns the material with a block-compressed albedo tiled at a world-space scale.
    ///
    /// The layer then takes the compressed upload path, provided every *other* textured layer does too —
    /// one array cannot hold two formats. See [`TerrainRenderer::with_materials`].
    #[must_use]
    pub fn with_compressed_albedo(mut self, albedo: TextureAsset, detail_scale: f32) -> Self {
        self.albedo = LayerAlbedo::Blocks(albedo);
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
    /// Retained so the page-compose pass can bind the same view and samplers this group does. See
    /// [`TerrainBindings`].
    height_view: wgpu::TextureView,
    weight_view: wgpu::TextureView,
    weight_sampler: wgpu::Sampler,
    albedo_sampler: wgpu::Sampler,
    /// One-by-one stand-ins for the virtual-texture bindings, held so the group can be rebuilt back to
    /// them.
    ///
    /// The layout declares those bindings whether or not a cache exists, because it is shared by six
    /// pipelines and a layout that changed with the cache would mean rebuilding all of them. Every binding a
    /// layout declares has to be filled, so the alternative to a placeholder is a second layout — which is a
    /// second thing to keep in step, for a case that renders identically either way.
    placeholders: VirtualPlaceholders,
    /// Whether the G-buffer samples composed pages. False until [`Self::attach_pages`] is called.
    virtual_pages: bool,
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
    /// The chunk decomposition the deferred passes cull and draw against.
    ///
    /// Built once from the terrain's own dimensions and elevation range. It survives a height write
    /// because its boxes span the whole elevation range rather than each chunk's own — see [`ChunkGrid`].
    chunks: ChunkGrid,
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
        // number and cannot pair a weight with another layer's surface.
        let slice_count = terrain.layers().len().max(1);
        let default = LayerMaterial::default();
        let layer_albedo: Vec<&LayerAlbedo> = (0..slice_count)
            .map(|index| &materials.get(index).unwrap_or(&default).albedo)
            .collect();
        let albedo = upload_layer_albedo(context, &layer_albedo)?;

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cic-render terrain uniforms"),
            size: UNIFORM_BYTES as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bindings = build_bindings(
            device,
            context.queue(),
            &uniform_buffer,
            &height_texture,
            &weight_texture,
            &albedo,
        );
        let pipeline = build_render_pipeline(device, &bindings.layout);

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
            bind_group: bindings.bind_group,
            bind_group_layout: bindings.layout,
            weight_view: bindings.weight_view,
            weight_sampler: bindings.weight_sampler,
            albedo_sampler: bindings.albedo_sampler,
            height_view: bindings.height_view,
            placeholders: bindings.placeholders,
            virtual_pages: false,
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
            chunks: ChunkGrid::new(terrain),
        })
    }

    /// Returns the chunk decomposition the deferred passes cull against.
    #[must_use]
    pub const fn chunks(&self) -> &ChunkGrid {
        &self.chunks
    }

    /// Vertices in one chunk's worth of grid.
    ///
    /// Every chunk submits the same count, including the partial ones at the terrain's far edge: their
    /// out-of-range vertices are collapsed to a degenerate triangle in the vertex shader rather than
    /// clamped, because clamping would smear the last row of cells across the missing ones. A uniform
    /// count is what lets a run of adjacent chunks draw as one instanced call.
    #[must_use]
    pub const fn chunk_vertex_count() -> u32 {
        CHUNK_CELLS * CHUNK_CELLS * 6
    }

    /// Records a terrain draw for each run of adjacent visible chunks.
    ///
    /// The instance index *is* the chunk index, which is what keeps this free of any new binding: the
    /// vertex shader turns it into a chunk origin using the counts already in the terrain uniform. Runs
    /// are drawn as single instanced calls, so a camera looking at a contiguous patch of map — which is
    /// every camera — costs a handful of draws rather than one per chunk.
    ///
    /// The pipeline and bind group are the caller's to set, because this is used by the G-buffer pass and
    /// by four shadow cascades that each bind their own cascade group first.
    pub fn draw_chunks(&self, pass: &mut wgpu::RenderPass<'_>, runs: &[Range<u32>]) {
        let vertices = Self::chunk_vertex_count();
        for run in runs {
            pass.draw(0..vertices, run.clone());
        }
    }

    /// Points the G-buffer at a page cache, so it samples composed pages instead of blending layers.
    ///
    /// # What changes and what does not
    ///
    /// The bind group is rebuilt against the cache's tables and pages, and a flag in the uniform tells the
    /// shader they are real. Nothing else moves: the layout, the pipelines, and the forward pass are
    /// untouched, and the forward pass never reads a page at all — it draws terrain alone in one pass, which
    /// is the case a cache has nothing to offer.
    ///
    /// **The direct blend stays in the shader as the fallback**, and that is not belt-and-braces. A cache is
    /// allowed to run out of slots — the residency map treats it as a normal condition — so a fragment over
    /// ground whose page was evicted has to have an answer. Making the frame depend on the cache having won
    /// would turn a memory budget into a correctness requirement.
    ///
    /// Views are cloned rather than borrowed, so the cache and the terrain do not have to outlive each other
    /// in a particular order. `wgpu` handles are reference-counted, so this shares the textures rather than
    /// copying them.
    pub fn attach_pages(&mut self, context: &GpuContext, cache: &crate::TerrainPageCache) {
        self.bind_group = build_bind_group(
            context.device(),
            &self.bind_group_layout,
            &self.uniform_buffer,
            &self.height_view,
            &self.weight_view,
            &self.weight_sampler,
            self.albedo.view(),
            &self.albedo_sampler,
            &VirtualViews {
                fine: cache.table_view(0).unwrap_or(&self.placeholders.table),
                coarse: cache.table_view(1).unwrap_or(&self.placeholders.table),
                pages: cache.page_view(),
            },
        );
        self.set_virtual_flag(context, true);
    }

    /// Stops the G-buffer sampling pages, restoring the direct blend.
    ///
    /// The counterpart to [`Self::attach_pages`], and it exists because a caller may want the comparison: the
    /// two paths compute the same surface, so rendering both is how anyone checks that they do.
    pub fn detach_pages(&mut self, context: &GpuContext) {
        self.bind_group = build_bind_group(
            context.device(),
            &self.bind_group_layout,
            &self.uniform_buffer,
            &self.height_view,
            &self.weight_view,
            &self.weight_sampler,
            self.albedo.view(),
            &self.albedo_sampler,
            &VirtualViews {
                fine: &self.placeholders.table,
                coarse: &self.placeholders.table,
                pages: &self.placeholders.pages,
            },
        );
        self.set_virtual_flag(context, false);
    }

    /// Records the switch and pushes it to the GPU at once.
    ///
    /// Written immediately rather than left for the next [`Self::set_frame`], and the difference is not
    /// cosmetic: attaching a cache and then rendering without an intervening frame upload would draw the
    /// direct blend while every accessor said otherwise. The first draft did exactly that, and the test that
    /// caught it reported the whole frame as byte-identical to the unattached one.
    ///
    /// A sixteen-byte write at the end of the block rather than a rebuild of it, because the rest of the
    /// block is a *frame's* worth of state and this is not part of a frame.
    fn set_virtual_flag(&mut self, context: &GpuContext, enabled: bool) {
        self.virtual_pages = enabled;
        let mut bytes = Vec::with_capacity(16);
        for value in [u32::from(enabled), 0, 0, 0] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        context
            .queue()
            .write_buffer(&self.uniform_buffer, (UNIFORM_BYTES - 16) as u64, &bytes);
    }

    /// Whether the G-buffer is sampling composed pages.
    #[must_use]
    pub const fn samples_pages(&self) -> bool {
        self.virtual_pages
    }

    /// Returns the terrain's sample dimensions minus one: the number of *cells* along each axis.
    ///
    /// What a page cache is decomposed in. Cells rather than samples because a page covers ground rather
    /// than grid points, and the two differ by one along each axis — which is exactly the off-by-one a
    /// virtual texture would express as a seam along the far edge of the map.
    #[must_use]
    pub const fn cell_size(&self) -> (u32, u32) {
        (
            if self.width > 1 { self.width - 1 } else { 1 },
            if self.height > 1 { self.height - 1 } else { 1 },
        )
    }

    /// Returns the per-frame uniform buffer.
    ///
    /// Exposed so a pass outside this module can bind the *same* buffer rather than build a second one from
    /// the same figures. The compose pass in [`crate::terrain_page`] does, which is what stops it
    /// disagreeing with the G-buffer about the terrain it is composing.
    #[must_use]
    pub const fn uniform_buffer(&self) -> &wgpu::Buffer {
        &self.uniform_buffer
    }

    /// Returns the layer weight array's view.
    #[must_use]
    pub const fn weight_view(&self) -> &wgpu::TextureView {
        &self.weight_view
    }

    /// Returns the sampler the layer weights are read through: clamped, because they are a per-map field in
    /// normalized coordinates.
    #[must_use]
    pub const fn weight_sampler(&self) -> &wgpu::Sampler {
        &self.weight_sampler
    }

    /// Returns the sampler the layer albedo is read through: repeating, because it tiles in world space.
    #[must_use]
    pub const fn albedo_sampler(&self) -> &wgpu::Sampler {
        &self.albedo_sampler
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

    /// Uploads the per-frame uniform block, with no wind and at time zero.
    ///
    /// What the forward path uses: it draws terrain alone, and terrain does not move. The animated form
    /// is [`Self::set_frame_animated`], and keeping the unanimated one as its own call is what lets every
    /// forward capture stay byte-identical to what it was before anything swayed.
    pub fn set_frame(
        &self,
        context: &GpuContext,
        view_projection: &[[f32; 4]; 4],
        camera_position: [f32; 3],
        light: DirectionalLight,
    ) {
        self.set_frame_animated(
            context,
            view_projection,
            camera_position,
            light,
            Animation::default(),
            Motion::still(view_projection),
        );
    }

    /// Uploads the per-frame uniform block, including everything an animated or reprojected pass reads.
    #[allow(clippy::too_many_arguments)]
    pub fn set_frame_animated(
        &self,
        context: &GpuContext,
        view_projection: &[[f32; 4]; 4],
        camera_position: [f32; 3],
        light: DirectionalLight,
        animation: Animation,
        motion: Motion,
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
        // `y` and `z` carry the chunk decomposition, which the G-buffer and shadow entry points need to
        // turn an instance index into a chunk origin. The forward pass shares this block byte for byte and
        // ignores both, drawing the whole grid as it always has.
        let (chunks_x, _) = self.chunks.counts();
        for value in [self.layer_count, chunks_x, CHUNK_CELLS, 0] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for entry in self.palette {
            push_vec4(entry, &mut bytes);
        }
        for scale in self.detail_scale {
            push_vec4([scale, 0.0, 0.0, 0.0], &mut bytes);
        }
        // Last in the block, and these three must stay last. See UNIFORM_BYTES.
        push_vec4(
            [
                animation.wind[0],
                animation.wind[1],
                animation.time,
                animation.previous_time,
            ],
            &mut bytes,
        );
        push_vec4([motion.jitter[0], motion.jitter[1], 0.0, 0.0], &mut bytes);
        for column in &motion.previous_view_projection {
            for value in column {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        // Whether the G-buffer reads composed pages instead of blending layers itself. A flag rather than an
        // inference the shader could make, because "the page table says nothing is resident" and "there is no
        // cache attached" want the same answer in the fragment and arrive by different routes — one is a
        // texel and the other is a bind group full of one-by-one placeholders.
        for value in [u32::from(self.virtual_pages), 0, 0, 0] {
            bytes.extend_from_slice(&value.to_le_bytes());
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
            // The virtual-texture bindings: one page table per level, then the physical pages. Declared
            // whether or not a cache is attached, and filled with one-by-one placeholders when it is not —
            // see `TerrainRenderer::attach_pages` for why that beats a second layout.
            //
            // Integer tables rather than float: an entry is a physical layer index, and a normalized read
            // would have to be scaled back to one, which is a division that can round to the wrong layer.
            page_table_entry(6),
            page_table_entry(7),
            wgpu::BindGroupLayoutEntry {
                binding: 8,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
        ],
    })
}

/// One page-table binding: a single unsigned integer per page, loaded rather than filtered.
const fn page_table_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Uint,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// The three virtual-texture views a terrain bind group needs, real or placeholder.
struct VirtualViews<'a> {
    fine: &'a wgpu::TextureView,
    coarse: &'a wgpu::TextureView,
    pages: &'a wgpu::TextureView,
}

/// One-by-one textures standing in for the virtual-texture bindings when no cache is attached.
///
/// The table reads as zero, which the shader takes as "not resident" — so a terrain with no cache falls
/// through to the direct blend by the same path a cache miss does, rather than by a second one.
#[derive(Debug)]
struct VirtualPlaceholders {
    table: wgpu::TextureView,
    pages: wgpu::TextureView,
}

impl VirtualPlaceholders {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let table = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cic-render empty page table"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        // Written explicitly rather than left to the zero-initialisation the API promises, because "the
        // shader reads zero here" is the whole behaviour and a promise is a worse place to keep it than a
        // four-byte upload.
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &table,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &0u32.to_le_bytes(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let pages = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cic-render empty pages"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: crate::terrain_page::PAGE_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        Self {
            table: table.create_view(&wgpu::TextureViewDescriptor::default()),
            pages: pages.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            }),
        }
    }
}

/// Builds the terrain bind group and the layout it was made against.
fn build_bindings(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    uniform_buffer: &wgpu::Buffer,
    height_texture: &wgpu::Texture,
    weight_texture: &wgpu::Texture,
    albedo: &TextureArray,
) -> TerrainBindings {
    // Two samplers, because the two arrays want opposite behaviour. Weights are a per-map field
    // addressed in normalized coordinates and must clamp at the edge; albedo is a detail texture
    // addressed in world units and must repeat, with the mip chain filtered between levels.
    //
    // The clamping one serves the composed pages too, and *that* is why it filters between mip levels: a page
    // carries a chain, the G-buffer picks a level from screen-space derivatives, and a nearest mip filter
    // would step visibly between levels as the camera moved. It changes nothing about the weights, which have
    // one level for a sampler to choose from.
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("cic-render terrain weight sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    });
    let albedo_sampler = array_sampler(device, "cic-render terrain albedo sampler");
    let layout = build_bind_group_layout(device);
    // Named rather than created inline in the entries below, because the compose pass in
    // [`crate::terrain_page`] binds these *same* views and samplers. A second set built from the same
    // descriptors would work and would also be a second thing to keep in step.
    let height_view = height_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let weight_view = weight_texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });

    let placeholders = VirtualPlaceholders::new(device, queue);
    let bind_group = build_bind_group(
        device,
        &layout,
        uniform_buffer,
        &height_view,
        &weight_view,
        &sampler,
        albedo.view(),
        &albedo_sampler,
        &VirtualViews {
            fine: &placeholders.table,
            coarse: &placeholders.table,
            pages: &placeholders.pages,
        },
    );

    TerrainBindings {
        layout,
        bind_group,
        height_view,
        weight_view,
        weight_sampler: sampler,
        albedo_sampler,
        placeholders,
    }
}

/// Assembles the terrain bind group from the resources it names.
///
/// Its own function because the group is built twice — once with placeholders and again when a page cache
/// is attached — and a nine-entry list written out twice is a list that drifts.
#[allow(clippy::too_many_arguments)]
fn build_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform_buffer: &wgpu::Buffer,
    height_view: &wgpu::TextureView,
    weight_view: &wgpu::TextureView,
    weight_sampler: &wgpu::Sampler,
    albedo_view: &wgpu::TextureView,
    albedo_sampler: &wgpu::Sampler,
    virtual_views: &VirtualViews<'_>,
) -> wgpu::BindGroup {
    fn view(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
        wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(view),
        }
    }
    fn sampler(binding: u32, sampler: &wgpu::Sampler) -> wgpu::BindGroupEntry<'_> {
        wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::Sampler(sampler),
        }
    }
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("cic-render terrain bindings"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            },
            view(1, height_view),
            view(2, weight_view),
            sampler(3, weight_sampler),
            view(4, albedo_view),
            sampler(5, albedo_sampler),
            view(6, virtual_views.fine),
            view(7, virtual_views.coarse),
            view(8, virtual_views.pages),
        ],
    })
}

/// What [`build_bindings`] produces: the group, its layout, and the resources a second pass may rebind.
struct TerrainBindings {
    layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    height_view: wgpu::TextureView,
    weight_view: wgpu::TextureView,
    weight_sampler: wgpu::Sampler,
    albedo_sampler: wgpu::Sampler,
    placeholders: VirtualPlaceholders,
}

/// Builds the forward render pipeline against a bind group layout.
fn build_render_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("cic-render terrain forward shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/terrain_forward.wgsl").into()),
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
/// Uploads the layer albedo array, block-compressed where every textured layer allows it.
///
/// # Why the choice is per array rather than per layer
///
/// One array holds one format at one size, so this is not a per-layer decision even though the materials
/// state it per layer. The compressed path is taken when **every** layer that has a texture at all has a
/// *compressed* one, and those agree on format, size and mip count. A single RGBA8 layer among them puts
/// the whole array back on the uncompressed path — where a compressed layer is still used, decoded, which
/// is the texture the author intended rather than a placeholder.
///
/// This is the same all-or-nothing rule the model path applies per material slot, and it is the same
/// reason: a compressed array has no resample available, because resampling blocks means decoding,
/// resampling and re-encoding them — the offline tool's work, at load time, for a worse result than
/// converting at the right size.
///
/// A layer with **no** texture is not an obstacle to either path. It takes an opaque-white slice, so its
/// palette colour multiplies through unchanged; on the compressed path that slice is a flat block of white
/// at the array's own size, which is exact in every format here.
fn upload_layer_albedo(
    context: &GpuContext,
    layers: &[&LayerAlbedo],
) -> Result<TextureArray, RenderError> {
    if let Some(slices) = compressed_layer_slices(context, layers) {
        let borrowed: Vec<&TextureAsset> = slices.iter().collect();
        return TextureArray::new_blocks(context, "cic-render terrain layer albedo", &borrowed);
    }
    // A layer with no texture takes an opaque-white slice, which multiplies its colour through unchanged.
    let slices: Vec<TextureImage> = layers
        .iter()
        .map(|albedo| {
            Ok(albedo
                .to_image()?
                .unwrap_or_else(|| TextureImage::solid(1, 1, [u8::MAX; 4])))
        })
        .collect::<Result<_, RenderError>>()?;
    TextureArray::new(context, "cic-render terrain layer albedo", &slices)
}

/// The compressed slices for the whole array, or `None` when the set does not allow the compressed path.
///
/// See [`upload_layer_albedo`] for the rule and why it is what it is.
fn compressed_layer_slices(
    context: &GpuContext,
    layers: &[&LayerAlbedo],
) -> Option<Vec<TextureAsset>> {
    if !context.supports_block_compression() {
        return None;
    }
    // Every textured layer must be compressed, and they must agree. An untextured layer abstains.
    let mut textured = layers.iter().filter(|albedo| !albedo.is_none()).peekable();
    textured.peek()?;
    let mut shape = None;
    for albedo in textured {
        let asset = albedo.blocks()?;
        let this = (
            asset.format(),
            asset.width(),
            asset.height(),
            asset.level_count(),
        );
        if *shape.get_or_insert(this) != this {
            return None;
        }
    }
    let (format, width, height, _) = shape?;

    // The white slice an untextured layer takes, in the array's own format and size. One flat block
    // repeated, so this costs a copy rather than a compression pass.
    let mut blank = None;
    layers
        .iter()
        .map(|albedo| match albedo.blocks() {
            Some(asset) => Some(asset.clone()),
            None => blank
                .get_or_insert_with(|| {
                    TextureAsset::solid(
                        width,
                        height,
                        format,
                        [u8::MAX; 4],
                        cic_assets::TextureLimits::default(),
                    )
                    .ok()
                })
                .clone(),
        })
        .collect()
}

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
