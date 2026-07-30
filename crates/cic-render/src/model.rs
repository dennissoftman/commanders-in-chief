//! Model geometry on the GPU: instanced, physically textured, with per-instance tint and sway.
//!
//! # One draw per model, not per primitive
//!
//! A glTF model arrives as several primitives, each with its own material. The obvious mapping is one
//! draw call per primitive with the material bound as state — which means a bind group change between
//! every one, and a model of twenty primitives costs twenty state transitions per frame.
//!
//! Instead the material *index* is a per-vertex attribute. Every vertex of a primitive shares the same
//! index, so all of a model's primitives concatenate into one vertex and index buffer, materials live
//! in one storage buffer bound once, and the whole model draws in a single instanced call. The cost is
//! four bytes per vertex.
//!
//! # Per-instance tint and sway
//!
//! Instances carry a colour multiplier and a sway parameter set as well as a transform. Recovered
//! equipment that keeps its original silhouette under different markings is a shared mesh with a
//! per-instance colour, so that channel exists from the start rather than being retrofitted later.
//!
//! Sway is per instance for a different and more structural reason. The displacement has to be identical
//! in the G-buffer pass and in every shadow cascade — a cascade that swayed differently would throw a
//! shadow detached from its caster — and the instance buffer is the only per-draw data all five of those
//! passes already bind. The shadow pipelines bind the terrain group and the cascade and nothing else, so
//! anything reaching them through a new bind group would have to be added to each. See
//! [`crate::scenery`] for the sway model itself.
//!
//! # Textures, without a bind group per material
//!
//! The same problem as the material factors, one step harder: a texture is a bound resource, and
//! binding one per material is exactly the per-primitive state change this module exists to avoid.
//! So the model's images upload as the slices of three arrays — base colour, normal, and combined
//! metallic-roughness — and each material stores the *slice* it reads in each. One bind group per
//! model, whatever its material and texture count.
//!
//! Three arrays rather than one because they are not in the same colour space. Base colour is
//! sRGB-encoded and the other two are linear measurements, and one array has one format. See
//! [`crate::texture::ColourSpace`] for what goes wrong when a normal map is decoded as though it were a
//! colour.
//!
//! The arrays' cost is that every slice shares a size, so a model mixing a 1024-pixel hull texture
//! with a 256-pixel decal sheet stores the decal upsampled. That is memory, not quality. See
//! [`crate::texture`] for why the alternative — one atlas per model — was not taken.
//!
//! A material with no texture in a given slot still names slice 0 and still samples it. Sampling is not
//! skipped for it, and the result is discarded by a `select` instead: mip level comes from screen-space
//! derivatives, which are undefined in non-uniform control flow, and the material index is per-vertex
//! and therefore not uniform. Branching would make every textured fragment's mip level undefined.
//!
//! # Two index ranges, not one
//!
//! Two things a material can ask for cannot be expressed per material inside one draw: discarding
//! fragments, and being drawn from both faces. Both are pipeline state. So the indices are written in two
//! runs — a *solid* run and a *cutout* run — and the two draw through different pipelines.
//!
//! They are one split rather than two because the two requests arrive together in practice and each is
//! cheap to grant to the other. Foliage is the case that needs the alpha test and it is also the case that
//! needs both faces, since a leaf card has no interior. And an opaque material that merely asked to be
//! double-sided loses nothing by going down the cutout path: the shader reads a zero cutoff as "never
//! discard", so it costs only the early depth rejection a two-sided surface was never going to get much
//! from anyway. Splitting four ways instead would double the pipeline count to separate cases no content
//! has yet asked for.
//!
//! Splitting the *indices* rather than the vertices is what keeps this free: an index is absolute, so
//! grouping them costs a reorder of a `u32` list at upload and nothing at all per frame.

use std::collections::BTreeSet;

use cic_assets::texture::TextureAsset;
use cic_assets::{AlphaMode, Model, ModelMaterial, ModelTextures};

use crate::RenderError;
use crate::scenery::{SwayProfile, sway_phase};
use crate::texture::{ColourSpace, TextureArray, TextureImage, array_sampler};

/// The two index runs a packed model draws through, and the bytes behind them.
///
/// Named because the tuple has four parts and two of them are the same type: `(vertices, indices, solid,
/// cutout)`, where swapping the last two silently draws the wrong geometry through the wrong pipeline.
type PackedGeometry = (Vec<u8>, Vec<u8>, std::ops::Range<u32>, std::ops::Range<u32>);

/// Bytes per model vertex: position, normal, tangent, texture coordinates, material index, sway weight.
const VERTEX_STRIDE: usize = 3 * 4 + 3 * 4 + 4 * 4 + 2 * 4 + 4 + 4;

/// Bytes per instance: a column-major transform, a tint, and the sway parameters.
const INSTANCE_STRIDE: usize = 64 + 16 + 16;

/// Bytes per material: base colour, the scalar factors, the map slices, and the surface parameters.
const MATERIAL_STRIDE: usize = 16 * 4;

/// The encoded flat tangent-space normal, which is what a material with no normal map samples.
///
/// `(0.5, 0.5, 1.0)` decodes to `(0, 0, 1)` — no perturbation at all. The identity for this slot, the
/// way opaque white is the identity for a colour slot.
const FLAT_NORMAL: [u8; 4] = [128, 128, 255, 255];

/// One placement of a model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelInstance {
    /// Column-major world transform: `transform[column][row]`.
    pub transform: [[f32; 4]; 4],
    /// Multiplied into the material's base colour. `[1.0; 4]` leaves the material as authored.
    pub tint: [f32; 4],
    /// Sway parameters as [`SwayProfile::packed`] writes them: tip fraction, phase, frequency, flutter.
    ///
    /// All zero means the instance does not move, which is what [`Self::placed`] produces. Use
    /// [`Self::planted`] for scenery that should.
    pub sway: [f32; 4],
}

impl Default for ModelInstance {
    fn default() -> Self {
        Self {
            transform: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            tint: [1.0; 4],
            sway: [0.0; 4],
        }
    }
}

impl ModelInstance {
    /// A placement at a world position, rotated about Z and uniformly scaled.
    ///
    /// Uniform scale specifically: the shader transforms normals by the transform's upper 3x3 and
    /// renormalizes, which is exact for rotation and uniform scale and wrong under shear or
    /// non-uniform scale. Offering only what is correct beats offering a general transform that
    /// silently mis-lights.
    #[must_use]
    pub fn placed(position: [f32; 3], rotation_radians: f32, scale: f32) -> Self {
        let (sin, cos) = rotation_radians.sin_cos();
        Self {
            transform: [
                [cos * scale, sin * scale, 0.0, 0.0],
                [-sin * scale, cos * scale, 0.0, 0.0],
                [0.0, 0.0, scale, 0.0],
                [position[0], position[1], position[2], 1.0],
            ],
            tint: [1.0; 4],
            sway: [0.0; 4],
        }
    }

    /// A placement that sways, with its phase derived from where it stands.
    ///
    /// The phase comes from the position rather than from the caller, because a caller who had to supply
    /// one would either pass a constant — and a stand of trees moving in unison is the most obvious tell
    /// there is — or draw a random one, which would make a capture irreproducible. See [`sway_phase`].
    #[must_use]
    pub fn planted(
        position: [f32; 3],
        rotation_radians: f32,
        scale: f32,
        profile: SwayProfile,
    ) -> Self {
        let mut instance = Self::placed(position, rotation_radians, scale);
        instance.sway = profile.packed(sway_phase(position));
        instance
    }

    /// Returns the instance with a tint applied.
    #[must_use]
    pub const fn with_tint(mut self, tint: [f32; 4]) -> Self {
        self.tint = tint;
        self
    }

    /// The largest factor by which this instance's sway can lift a vertex above where it sat still.
    ///
    /// The displacement moves a vertex sideways and then re-projects it onto the sphere of its original
    /// radius about the anchor, so the radius is preserved exactly — but the *height* is not, because a
    /// vertex whose sideways move shortened its distance is scaled back out again. The scale is
    /// `radius / |offset + h|`, and by the triangle inequality that is at most
    /// `radius / (radius - |h|)`; with `|h|` bounded by `tip_fraction * radius` this reduces to
    /// `1 / (1 - tip_fraction)`.
    ///
    /// It matters because a shadow cascade is fitted from the tallest caster in the scene. A cascade
    /// sized to the still height would stop looking just below a swaying tip, and the tip would drop its
    /// shadow only on the frames when the wind lifted it — which reads as flickering rather than as a
    /// fitting error.
    fn sway_headroom(&self) -> f32 {
        let tip_fraction = self.sway[0].clamp(0.0, 0.9);
        1.0 / (1.0 - tip_fraction)
    }
}

/// One model uploaded to the GPU with its instances.
#[derive(Debug)]
pub struct ModelBatch {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    material_group: wgpu::BindGroup,
    base_colour: TextureArray,
    /// Indices belonging to single-sided opaque materials, drawn first and without a fragment stage in
    /// the shadow passes.
    solid: std::ops::Range<u32>,
    /// Indices belonging to materials that cut their own silhouette or want both faces drawn. See the
    /// module note on why those are one range.
    cutout: std::ops::Range<u32>,
    instance_count: u32,
    /// The model's own upper bound corner, retained so a later instance change can recompute the
    /// batch's world height exactly rather than approximating it from the instance origins.
    maximum: Option<[f32; 3]>,
    top: f32,
}

impl ModelBatch {
    /// Returns the bind group layout the material storage buffer and the three arrays are bound through.
    ///
    /// Built once by the renderer and shared: the pipelines are created against it and every batch's
    /// bind group uses it, so the two cannot drift apart.
    #[must_use]
    pub fn material_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        let texture = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2Array,
                multisampled: false,
            },
            count: None,
        };
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cic-render model material layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        // Storage rather than uniform: a uniform array would have to be a fixed size,
                        // and a model's material count is whatever its author gave it.
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // One sampler for all three arrays. They differ in format and in what their bytes mean,
                // and in nothing a sampler decides: all three tile, and all three want trilinear
                // filtering for the same reason.
                texture(3),
                texture(4),
            ],
        })
    }

    /// Uploads a model and its instances, using whatever images the container carried.
    ///
    /// See [`Self::with_textures`] to supply block-compressed sidecars found for those images.
    ///
    /// # Errors
    ///
    /// As [`Self::with_textures`].
    pub fn new(
        context: &crate::GpuContext,
        model: &Model,
        instances: &[ModelInstance],
        material_layout: &wgpu::BindGroupLayout,
    ) -> Result<Self, RenderError> {
        Self::with_textures(
            context,
            model,
            &ModelTextures::default(),
            instances,
            material_layout,
        )
    }

    /// Uploads a model and its instances, preferring block-compressed textures where they were found.
    ///
    /// `textures` is the table [`cic_assets::resolve_model_textures`] built, parallel to
    /// [`Model::images`] by index. An entry present *and* usable for a slot makes that slot a
    /// straight block copy; anything else leaves the slot on the RGBA8 path. See
    /// [`upload_arrays`] for what "usable" means and why the decision is per slot.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::EmptyModel`] when the model has no geometry, or
    /// [`RenderError::ModelTooLarge`] when its vertex or index count exceeds the addressable range.
    pub fn with_textures(
        context: &crate::GpuContext,
        model: &Model,
        textures: &ModelTextures,
        instances: &[ModelInstance],
        material_layout: &wgpu::BindGroupLayout,
    ) -> Result<Self, RenderError> {
        if model.primitives.is_empty() || model.vertex_count() == 0 {
            return Err(RenderError::EmptyModel);
        }

        let (vertices, indices, solid, cutout) = pack_geometry(model)?;

        // Every image in source order, so a material's recorded image index *is* its slice index in each
        // array. Compacting the list to only the images a given slot references would shift every later
        // material onto the wrong picture, which is a wrong answer presented confidently.
        //
        // A model with no images at all still gets a one-slice array holding that slot's identity value —
        // every material then names slice 0 and discards what it reads, which keeps the sampling call
        // unconditional. See the module note on why the branch that looks cheaper is not available.
        let [base_colour, normal, metallic_roughness] = upload_arrays(context, model, textures)?;
        let materials = pack_materials(model, base_colour.layer_count());

        let mut instance_bytes = pack_instances(instances);
        // An empty batch is legal and draws nothing, but a zero-sized buffer is not, so one instance
        // worth of zeroes is allocated and the draw is skipped by `instance_count`.
        if instance_bytes.is_empty() {
            instance_bytes.resize(INSTANCE_STRIDE, 0);
        }

        let device = context.device();
        let vertex_buffer = buffer(
            device,
            "cic-render model vertices",
            &vertices,
            wgpu::BufferUsages::VERTEX,
        );
        let index_buffer = buffer(
            device,
            "cic-render model indices",
            &indices,
            wgpu::BufferUsages::INDEX,
        );
        let instance_buffer = buffer(
            device,
            "cic-render model instances",
            &instance_bytes,
            wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        );
        let material_buffer = buffer(
            device,
            "cic-render model materials",
            &materials,
            wgpu::BufferUsages::STORAGE,
        );
        context.queue().write_buffer(&vertex_buffer, 0, &vertices);
        context.queue().write_buffer(&index_buffer, 0, &indices);
        context
            .queue()
            .write_buffer(&instance_buffer, 0, &instance_bytes);
        context
            .queue()
            .write_buffer(&material_buffer, 0, &materials);

        let sampler = array_sampler(device, "cic-render model texture sampler");
        let material_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cic-render model materials"),
            layout: material_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: material_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(base_colour.view()),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(normal.view()),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(metallic_roughness.view()),
                },
            ],
        });

        Ok(Self {
            vertex_buffer,
            index_buffer,
            instance_buffer,
            material_group,
            base_colour,
            solid,
            cutout,
            instance_count: u32::try_from(instances.len())
                .map_err(|_| RenderError::ModelTooLarge)?,
            maximum: model.bounds().map(|(_, maximum)| maximum),
            top: world_top(model.bounds().map(|(_, maximum)| maximum), instances),
        })
    }

    /// Overwrites the instance data, keeping the geometry.
    ///
    /// The instance count may only shrink or stay the same, because the buffer was sized at upload.
    /// Growing it needs a new batch — which is the honest constraint rather than a silent truncation.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::ModelTooLarge`] when more instances are supplied than the batch was
    /// built with.
    pub fn set_instances(
        &mut self,
        context: &crate::GpuContext,
        instances: &[ModelInstance],
    ) -> Result<(), RenderError> {
        let count = u32::try_from(instances.len()).map_err(|_| RenderError::ModelTooLarge)?;
        if u64::from(count) * INSTANCE_STRIDE as u64 > self.instance_buffer.size() {
            return Err(RenderError::ModelTooLarge);
        }
        let bytes = pack_instances(instances);
        if !bytes.is_empty() {
            context
                .queue()
                .write_buffer(&self.instance_buffer, 0, &bytes);
        }
        self.instance_count = count;
        // Recomputed from the new set alone, not merged with the old high-water mark: removing the
        // tallest instance has to *lower* this, or every cascade keeps reaching toward a caster that
        // is no longer in the scene and spends its resolution on empty space.
        self.top = world_top(self.maximum, instances);
        Ok(())
    }

    /// Returns the highest world-space Z any instance of this batch reaches, sway included.
    ///
    /// A shadow cascade sizes how far it looks toward the light from the tallest thing that can cast,
    /// and a model standing on terrain is taller than the terrain alone. Without this a tall model at a
    /// low sun casts no shadow, because the cascade never looked far enough to record it. See
    /// [`ModelInstance::sway_headroom`] for why a swaying instance reports more than its still height.
    #[must_use]
    pub const fn world_top(&self) -> f32 {
        self.top
    }

    /// Returns how many instances will be drawn.
    #[must_use]
    pub const fn instance_count(&self) -> u32 {
        self.instance_count
    }

    /// Returns how many triangles one instance contains, across both index ranges.
    #[must_use]
    pub const fn triangle_count(&self) -> u32 {
        (self.solid.end - self.solid.start + self.cutout.end - self.cutout.start) / 3
    }

    /// Whether any of this model's materials needs the cutout path — an alpha test, both faces, or both.
    ///
    /// The chain asks before recording the cutout pipelines, so a scene of entirely solid models pays
    /// nothing for a path it does not use.
    #[must_use]
    pub const fn has_cutout(&self) -> bool {
        self.cutout.end > self.cutout.start
    }

    /// Whether any of this model's materials is single-sided and opaque.
    #[must_use]
    pub const fn has_solid(&self) -> bool {
        self.solid.end > self.solid.start
    }

    /// Returns the base-colour array the model's materials index into.
    #[must_use]
    pub const fn base_colour(&self) -> &TextureArray {
        &self.base_colour
    }

    /// Records a draw of this model's solid geometry. See [`Self::draw_cutout`] for the rest.
    ///
    /// `material_group_index` is `Some(2)` for the G-buffer pass and `None` for the shadow pass, which
    /// needs no materials at all when nothing discards. The caller states it rather than this guessing,
    /// because the two passes genuinely bind different groups.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, material_group_index: Option<u32>) {
        self.draw_range(pass, material_group_index, self.solid.clone());
    }

    /// Records a draw of this model's alpha-tested and two-sided geometry.
    ///
    /// Separate from [`Self::draw`] because it needs a pipeline with a fragment stage — including in the
    /// shadow passes, where the solid path deliberately has none. That pipeline reads the materials, so
    /// unlike the solid shadow draw this one always binds them.
    pub fn draw_cutout(&self, pass: &mut wgpu::RenderPass<'_>, material_group_index: Option<u32>) {
        self.draw_range(pass, material_group_index, self.cutout.clone());
    }

    fn draw_range(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        material_group_index: Option<u32>,
        indices: std::ops::Range<u32>,
    ) {
        if self.instance_count == 0 || indices.is_empty() {
            return;
        }
        if let Some(index) = material_group_index {
            pass.set_bind_group(index, &self.material_group, &[]);
        }
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(indices, 0, 0..self.instance_count);
    }
}

/// One of the three arrays a model's materials index into.
struct Slot {
    /// Debug label for the uploaded array.
    label: &'static str,
    /// The space this slot's bytes are in, which decides both the uncompressed format and whether a
    /// compressed texture found for it is the right kind of thing.
    space: ColourSpace,
    /// The value a slice this slot never samples is filled with: the identity for what the slot means.
    identity: [u8; 4],
}

/// The three slots, in the order the bind group binds them.
const SLOTS: [Slot; 3] = [
    Slot {
        label: "cic-render model base colour",
        space: ColourSpace::Srgb,
        // Opaque white, which multiplies a material colour through unchanged.
        identity: [u8::MAX; 4],
    },
    Slot {
        label: "cic-render model normal",
        space: ColourSpace::Linear,
        identity: FLAT_NORMAL,
    },
    Slot {
        label: "cic-render model metallic roughness",
        space: ColourSpace::Linear,
        // White, so a material with no map has its factors multiplied by one rather than by zero.
        identity: [u8::MAX; 4],
    },
];

/// Uploads a model's images as the three arrays its materials index into: base colour, normal, and
/// combined metallic-roughness (which is also where an occlusion map lives, since glTF packs occlusion in
/// the red channel of the same image).
///
/// Every image in source order in each array, so a material's recorded image index *is* its slice index
/// whichever slot it is read through. Compacting a list to only the images that slot references would shift
/// every later material onto the wrong picture, which is a wrong answer presented confidently.
///
/// So the same images are uploaded three times, twice in a linear format. That is the honest cost of glTF's
/// model, where one image may be a colour in one material and data in another: a format is a property of the
/// texture, not of the read. In practice a normal map is never also a base colour, so the duplicate slices
/// are ones no material samples — memory, and nothing else.
///
/// A model with no images at all still gets a one-slice array holding each slot's identity value — every
/// material then names slice 0 and discards what it reads, which keeps the sampling call unconditional. See
/// the module note on why the branch that looks cheaper is not available.
///
/// # How a slot decides to take the compressed path
///
/// Per slot, not per model, and not per image. A slot goes compressed only when *every image that slot is
/// actually read through* has a sidecar, and those sidecars agree on format, size and mip count. Then the
/// slices the slot never samples are filled with a flat block of its identity value, so the whole array is
/// one format at one size — which is what a compressed array requires, having no resample available.
///
/// Any other combination leaves that slot on the RGBA8 path, where a sidecar is still not wasted: its base
/// level is decoded and used, which is better pixels than the placeholder the container carried. The two
/// requirements this rejects are worth naming, because both are ordinary content mistakes rather than
/// corruption:
///
/// - **A slot half converted.** Three of a model's four base colours have sidecars. Mixing a compressed
///   slice with an uncompressed one in a single array is not expressible, so the slot waits until the set
///   is complete.
/// - **Sidecars at different sizes.** A 2048 hull and a 512 decal. The uncompressed path resamples the
///   decal up; this one would have to decode, resample and re-encode it, which is the offline tool's work
///   done at load time to produce a worse image than converting at the right size would have.
///
/// A sidecar whose colour space disagrees with its slot — a `BC7_UNORM_SRGB` base colour found for the
/// normal slot — is not usable *there*, which is the same case as not having one. That is how a model whose
/// base colour is BC7 sRGB and whose normal is BC5 gets both: each slot accepts only what it can read.
///
/// # Errors
///
/// Returns a structured [`RenderError`] when an image is malformed or the arrays exceed their byte budget.
fn upload_arrays(
    context: &crate::GpuContext,
    model: &Model,
    textures: &ModelTextures,
) -> Result<[TextureArray; 3], RenderError> {
    // Which image indices each slot is genuinely read through, across every material. Occlusion joins the
    // metallic-roughness slot because glTF puts it in the same image's red channel.
    let mut referenced: [BTreeSet<usize>; 3] = Default::default();
    for material in &model.materials {
        referenced[0].extend(material.base_color_texture);
        referenced[1].extend(material.normal_texture);
        referenced[2].extend(material.metallic_roughness_texture);
        referenced[2].extend(material.occlusion_texture);
    }

    let mut arrays = Vec::with_capacity(SLOTS.len());
    for (slot, used) in SLOTS.iter().zip(&referenced) {
        arrays.push(
            if let Some(slices) = compressed_slices(context, model, textures, slot, used) {
                let borrowed: Vec<&TextureAsset> = slices.iter().collect();
                TextureArray::new_blocks(context, slot.label, &borrowed)?
            } else {
                let images = decoded_images(model, textures)?;
                TextureArray::new_in(context, slot.label, &images, slot.space, slot.identity)?
            },
        );
    }
    // Three slots in, three arrays out; the `try_into` cannot fail and says so once rather than at each use.
    arrays
        .try_into()
        .map_err(|_: Vec<TextureArray>| RenderError::InvalidTexture)
}

/// The compressed slices for one slot, or `None` when that slot cannot take the compressed path.
///
/// See [`upload_arrays`] for the rules and why they are what they are.
fn compressed_slices(
    context: &crate::GpuContext,
    model: &Model,
    textures: &ModelTextures,
    slot: &Slot,
    used: &BTreeSet<usize>,
) -> Option<Vec<TextureAsset>> {
    if !context.supports_block_compression() || used.is_empty() {
        return None;
    }
    // Every image this slot reads must have a sidecar it can actually read, and they must all agree.
    let mut usable = used.iter().map(|index| {
        textures
            .get(*index)
            .filter(|asset| asset.format().colour_space() == slot.space)
    });
    let first = usable.next().flatten()?;
    let shape = (
        first.format(),
        first.width(),
        first.height(),
        first.level_count(),
    );
    if !usable.all(|asset| {
        asset.is_some_and(|asset| {
            (
                asset.format(),
                asset.width(),
                asset.height(),
                asset.level_count(),
            ) == shape
        })
    }) {
        return None;
    }

    let (format, width, height, _) = shape;
    // The slices this slot never samples still occupy a layer, because a material's image index is its
    // slice index in all three arrays. A flat block of the slot's identity value is what belongs there.
    let filler = TextureAsset::solid(
        width,
        height,
        format,
        slot.identity,
        cic_assets::TextureLimits::default(),
    )
    .ok()?;
    Some(
        (0..model.images.len())
            .map(|index| match textures.get(index) {
                Some(asset) if used.contains(&index) => asset.clone(),
                _ => filler.clone(),
            })
            .collect(),
    )
}

/// A model's images as RGBA8, preferring a sidecar's decoded base level over the container's own pixels.
///
/// The uncompressed path, and the reason a sidecar is never wasted even on a device that cannot sample
/// blocks: the `.dds` holds the authored texture and the container may hold only a placeholder, so
/// decoding the sidecar is both the better picture and the one the author intended.
fn decoded_images(
    model: &Model,
    textures: &ModelTextures,
) -> Result<Vec<TextureImage>, RenderError> {
    model
        .images
        .iter()
        .enumerate()
        .map(|(index, image)| match textures.get(index) {
            Some(asset) => TextureImage::new(asset.width(), asset.height(), asset.decode()),
            None => TextureImage::new(image.width, image.height, image.rgba.clone()),
        })
        .collect()
}

/// The vertex and instance buffer layouts, in the order the pipelines expect them.
///
/// Wrapped in `Some` because a pipeline may leave a vertex buffer slot empty, the same way it may
/// leave a bind group slot empty.
#[must_use]
pub fn buffer_layouts() -> [Option<wgpu::VertexBufferLayout<'static>>; 2] {
    [
        Some(wgpu::VertexBufferLayout {
            array_stride: VERTEX_STRIDE as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 12,
                    shader_location: 1,
                },
                // xyz tangent, w the bitangent's handedness. See `cic_assets::ModelVertex::tangent`.
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 24,
                    shader_location: 2,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 40,
                    shader_location: 3,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Uint32,
                    offset: 48,
                    shader_location: 4,
                },
                // The share of the sway this vertex takes. See `crate::scenery`.
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32,
                    offset: 52,
                    shader_location: 5,
                },
            ],
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: INSTANCE_STRIDE as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                // A 4x4 transform arrives as four separate vec4 attributes; there is no matrix
                // vertex format.
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 0,
                    shader_location: 6,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 16,
                    shader_location: 7,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 32,
                    shader_location: 8,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 48,
                    shader_location: 9,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 64,
                    shader_location: 10,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 80,
                    shader_location: 11,
                },
            ],
        }),
    ]
}

/// Concatenates a model into one vertex and index buffer, with each vertex carrying its material.
///
/// Returns the packed bytes and the two index ranges: solid first, then everything that needs the cutout
/// path. Indices are rebased per primitive, which is what allows the whole model to draw in one call per
/// range.
///
/// # Errors
///
/// Returns [`RenderError::ModelTooLarge`] when a count or a rebased index leaves the addressable range.
fn pack_geometry(model: &Model) -> Result<PackedGeometry, RenderError> {
    let mut vertices: Vec<u8> = Vec::new();
    let mut solid: Vec<u8> = Vec::new();
    let mut cutout: Vec<u8> = Vec::new();
    let mut base_vertex = 0u32;
    let mut solid_count = 0u32;
    let mut cutout_count = 0u32;

    // The height range the per-vertex sway weight is normalized against. The model's own bounds rather
    // than a per-primitive box: a canopy primitive and a trunk primitive must share one ramp, or the
    // canopy's own lowest leaves would be as still as the trunk's foot.
    let (base_z, span_z) = model.bounds().map_or((0.0, 0.0), |(minimum, maximum)| {
        (minimum[2], maximum[2] - minimum[2])
    });

    for primitive in &model.primitives {
        // A primitive with no material takes slot 0 and gets the default below, so a fragment
        // never indexes past the storage buffer.
        let material = u32::try_from(primitive.material.map_or(0, |index| index + 1))
            .map_err(|_| RenderError::ModelTooLarge)?;
        for vertex in &primitive.vertices {
            for value in vertex.position {
                vertices.extend_from_slice(&value.to_le_bytes());
            }
            for value in vertex.normal {
                vertices.extend_from_slice(&value.to_le_bytes());
            }
            for value in vertex.tangent {
                vertices.extend_from_slice(&value.to_le_bytes());
            }
            for value in vertex.uv {
                vertices.extend_from_slice(&value.to_le_bytes());
            }
            vertices.extend_from_slice(&material.to_le_bytes());
            // The raw normalized height, not a curve. The exponent that turns a height ramp into a
            // bending mode shape belongs with the physics in `crate::scenery`, so that a model with an
            // authored weight channel and one without behave identically.
            let sway = if span_z > 0.0 {
                ((vertex.position[2] - base_z) / span_z).clamp(0.0, 1.0)
            } else {
                0.0
            };
            vertices.extend_from_slice(&sway.to_le_bytes());
        }

        let cuts = primitive
            .material
            .and_then(|index| model.materials.get(index))
            // A primitive with no material takes the glTF default, which is opaque and single-sided.
            .is_some_and(|material| {
                !matches!(material.alpha_mode, AlphaMode::Opaque) || material.double_sided
            });
        let (target, count) = if cuts {
            (&mut cutout, &mut cutout_count)
        } else {
            (&mut solid, &mut solid_count)
        };
        // Indices are primitive-local, so each primitive's are offset by the vertices already
        // written. That is what allows one draw call for the whole model.
        for index in &primitive.indices {
            let shifted = index
                .checked_add(base_vertex)
                .ok_or(RenderError::ModelTooLarge)?;
            target.extend_from_slice(&shifted.to_le_bytes());
        }
        *count = count
            .checked_add(
                u32::try_from(primitive.indices.len()).map_err(|_| RenderError::ModelTooLarge)?,
            )
            .ok_or(RenderError::ModelTooLarge)?;
        base_vertex = base_vertex
            .checked_add(
                u32::try_from(primitive.vertices.len()).map_err(|_| RenderError::ModelTooLarge)?,
            )
            .ok_or(RenderError::ModelTooLarge)?;
    }

    let total = solid_count
        .checked_add(cutout_count)
        .ok_or(RenderError::ModelTooLarge)?;
    solid.extend_from_slice(&cutout);
    Ok((vertices, solid, 0..solid_count, solid_count..total))
}

/// Highest world-space Z reached by any instance of a model with the given upper bound corner.
fn world_top(maximum: Option<[f32; 3]>, instances: &[ModelInstance]) -> f32 {
    let Some(maximum) = maximum else {
        return 0.0;
    };
    let mut top = f32::NEG_INFINITY;
    for instance in instances {
        // Only the Z row of the transform matters, applied to the bound's upper corner. The upper
        // corner suffices because the transform allows no shear: rotation about Z leaves Z unchanged
        // and uniform scale cannot make a lower corner the taller one.
        let z = instance.transform[0][2] * maximum[0]
            + instance.transform[1][2] * maximum[1]
            + instance.transform[2][2] * maximum[2]
            + instance.transform[3][2];
        // The anchor is the transform's translation, and the sway acts on the height above it.
        let anchor = instance.transform[3][2];
        let swayed = anchor + (z - anchor).max(0.0) * instance.sway_headroom();
        top = top.max(swayed);
    }
    if top.is_finite() { top } else { 0.0 }
}

/// Packs the instance buffer: each transform column-major, then the tint, then the sway parameters.
fn pack_instances(instances: &[ModelInstance]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(instances.len() * INSTANCE_STRIDE);
    for instance in instances {
        for column in instance.transform {
            for value in column {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        for value in instance.tint {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for value in instance.sway {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    bytes
}

/// Packs the material storage buffer, resolving each material's texture slices.
///
/// Slot 0 is a neutral default for primitives that declare no material; the model's own materials
/// follow it, which is why `pack_geometry` shifts its index by one.
fn pack_materials(model: &Model, slice_count: u32) -> Vec<u8> {
    let mut materials: Vec<u8> = Vec::with_capacity((model.materials.len() + 1) * MATERIAL_STRIDE);
    push_material(&mut materials, &default_material(), slice_count);
    for material in &model.materials {
        push_material(&mut materials, material, slice_count);
    }
    materials
}

/// The material a primitive declaring none takes here: a mid-grey dielectric.
///
/// Deliberately **not** [`ModelMaterial::default`], which is glTF's own default and is a fully rough
/// metal — a surface with no highlight and almost no diffuse term, which renders very nearly black. The
/// spec's choice is a reasonable one for a loader, where an unfinished mesh should look unfinished. It is
/// the wrong one for a renderer, where the common cause of a missing material is a placeholder or a
/// blockout and the useful answer is a plausible surface.
fn default_material() -> ModelMaterial {
    ModelMaterial {
        base_color: [0.8, 0.8, 0.8, 1.0],
        metallic: 0.0,
        roughness: 0.7,
        ..ModelMaterial::default()
    }
}

/// Appends one material record: base colour, the scalar factors, the map slices, and the surface
/// parameters.
///
/// A slice index travels as a float because the record is a `vec4<f32>` either way and a separate
/// integer field would cost 16 more bytes to alignment. Slice counts are bounded by the asset layer's
/// image limit, far below the point where a float stops representing consecutive integers exactly.
fn push_material(bytes: &mut Vec<u8>, material: &ModelMaterial, slice_count: u32) {
    // An index past the images actually decoded is dropped rather than clamped onto some other
    // material's picture, which would be a wrong answer presented confidently.
    let slice = |index: Option<usize>| -> Option<u32> {
        index
            .and_then(|index| u32::try_from(index).ok())
            .filter(|index| *index < slice_count)
    };
    let base = slice(material.base_color_texture);
    let normal = slice(material.normal_texture);
    let metallic_roughness = slice(material.metallic_roughness_texture);

    // Slice indices are bounded by the image limit, well inside exact f32 range.
    #[allow(clippy::cast_precision_loss)]
    let pair = |slot: Option<u32>| -> [f32; 2] {
        [
            slot.unwrap_or(0) as f32,
            f32::from(u8::from(slot.is_some())),
        ]
    };
    let [base_slice, base_present] = pair(base);
    let [normal_slice, normal_present] = pair(normal);
    let [mr_slice, mr_present] = pair(metallic_roughness);

    for value in material.base_color {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for value in [
        material.metallic,
        material.roughness,
        base_slice,
        base_present,
    ] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for value in [normal_slice, normal_present, mr_slice, mr_present] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    // A material that discards reports its cutoff; one that does not reports zero, which the shader
    // reads as "never discard" — so the same fragment stage serves both and the opaque pipeline is
    // simply the one that has none.
    let cutoff = material.alpha_mode.cutoff().unwrap_or(0.0);
    for value in [
        material.normal_scale,
        cutoff,
        emissive_strength(material),
        0.0,
    ] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
}

/// The scalar self-illumination a material contributes, for the G-buffer's coverage channel.
///
/// The channel carries one number and glTF's emissive is a colour, so this is the colour's luminance and
/// the *hue* comes from the material's own albedo in the lighting pass. That approximation is exact for
/// the common case — a lamp whose emissive colour is its base colour — and wrong only for a surface
/// authored to glow a different colour than it reflects, which is rare enough not to justify a second
/// G-buffer channel. See `lighting.wgsl`, where the decode lives.
///
/// The Rec. 709 luminance weights, because that is the primary set the rest of the chain works in.
fn emissive_strength(material: &ModelMaterial) -> f32 {
    let [r, g, b] = material.emissive;
    let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    (luminance * material.emissive_strength).max(0.0)
}

fn buffer(
    device: &wgpu::Device,
    label: &str,
    contents: &[u8],
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        // Buffer sizes must be a multiple of four, and a zero-sized buffer is invalid.
        size: (contents.len().max(4) as u64).next_multiple_of(4),
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

#[cfg(test)]
mod tests {
    // Comparisons are against transform entries the constructor writes directly.
    #![allow(clippy::float_cmp)]

    use super::{
        INSTANCE_STRIDE, MATERIAL_STRIDE, ModelInstance, VERTEX_STRIDE, buffer_layouts,
        default_material, emissive_strength, world_top,
    };
    use crate::scenery::SwayProfile;
    use cic_assets::AlphaMode;

    #[test]
    fn strides_match_the_declared_attributes() {
        // A stride disagreeing with its attributes does not fail validation in any obvious way; it
        // silently reads each vertex from the wrong offset, which looks like corrupt geometry.
        let [vertex, instance] = buffer_layouts();
        let vertex = vertex.expect("a vertex layout");
        let instance = instance.expect("an instance layout");
        assert_eq!(vertex.array_stride, VERTEX_STRIDE as u64);
        assert_eq!(instance.array_stride, INSTANCE_STRIDE as u64);

        // Every attribute, not only the last: the tangent was inserted in the middle of this layout, and
        // an offset that overlaps its neighbour reads half of one attribute and half of another.
        for attribute in vertex.attributes {
            let width = match attribute.format {
                wgpu::VertexFormat::Float32 | wgpu::VertexFormat::Uint32 => 4,
                wgpu::VertexFormat::Float32x2 => 8,
                wgpu::VertexFormat::Float32x3 => 12,
                wgpu::VertexFormat::Float32x4 => 16,
                other => panic!("unexpected vertex format {other:?}"),
            };
            assert!(
                attribute.offset + width <= VERTEX_STRIDE as u64,
                "{attribute:?} does not fit the stride"
            );
        }
        let last = instance.attributes.last().expect("instance attributes");
        assert_eq!(
            last.offset + 16,
            INSTANCE_STRIDE as u64,
            "the sway parameters end the instance record"
        );
    }

    #[test]
    fn no_vertex_attribute_overlaps_another() {
        let [vertex, _] = buffer_layouts();
        let vertex = vertex.expect("a vertex layout");
        let mut spans: Vec<(u64, u64)> = vertex
            .attributes
            .iter()
            .map(|attribute| {
                let width = match attribute.format {
                    wgpu::VertexFormat::Float32 | wgpu::VertexFormat::Uint32 => 4,
                    wgpu::VertexFormat::Float32x2 => 8,
                    wgpu::VertexFormat::Float32x3 => 12,
                    wgpu::VertexFormat::Float32x4 => 16,
                    other => panic!("unexpected vertex format {other:?}"),
                };
                (attribute.offset, attribute.offset + width)
            })
            .collect();
        spans.sort_unstable();
        for pair in spans.windows(2) {
            assert!(
                pair[0].1 <= pair[1].0,
                "attributes at {:?} and {:?} overlap",
                pair[0],
                pair[1]
            );
        }
        // And the whole stride is used, so a byte is not being paid for and left unread.
        assert_eq!(spans.last().expect("spans").1, VERTEX_STRIDE as u64);
    }

    #[test]
    fn shader_locations_are_unique_and_contiguous() {
        // The two buffers share one location namespace, so an overlap binds the wrong data rather
        // than failing to build.
        let [vertex, instance] = buffer_layouts();
        let vertex = vertex.expect("a vertex layout");
        let instance = instance.expect("an instance layout");
        let mut locations: Vec<u32> = vertex
            .attributes
            .iter()
            .chain(instance.attributes.iter())
            .map(|attribute| attribute.shader_location)
            .collect();
        locations.sort_unstable();
        let expected: Vec<u32> = (0..12).collect();
        assert_eq!(locations, expected, "locations 0..12, each used once");
    }

    #[test]
    fn a_placement_positions_rotates_and_scales() {
        let instance = ModelInstance::placed([10.0, 20.0, 30.0], 0.0, 2.0);
        // Translation sits in the fourth column.
        assert_eq!(instance.transform[3], [10.0, 20.0, 30.0, 1.0]);
        // Uniform scale on the diagonal, no rotation.
        assert_eq!(instance.transform[0][0], 2.0);
        assert_eq!(instance.transform[1][1], 2.0);
        assert_eq!(instance.transform[2][2], 2.0);
        assert_eq!(instance.tint, [1.0; 4]);
        assert_eq!(instance.sway, [0.0; 4], "a plain placement does not move");
    }

    #[test]
    fn a_quarter_turn_maps_x_onto_y() {
        let instance = ModelInstance::placed([0.0; 3], core::f32::consts::FRAC_PI_2, 1.0);
        // Column 0 is where the model's +X axis ends up.
        let x_axis = instance.transform[0];
        assert!(x_axis[0].abs() < 1.0e-6, "{x_axis:?}");
        assert!((x_axis[1] - 1.0).abs() < 1.0e-6, "{x_axis:?}");
    }

    #[test]
    fn a_tint_replaces_only_the_colour() {
        let instance =
            ModelInstance::placed([1.0, 2.0, 3.0], 0.5, 1.5).with_tint([0.4, 0.5, 0.6, 1.0]);
        assert_eq!(instance.tint, [0.4, 0.5, 0.6, 1.0]);
        assert_eq!(instance.transform[3], [1.0, 2.0, 3.0, 1.0]);
    }

    #[test]
    fn a_planted_instance_carries_its_profile_and_a_position_derived_phase() {
        let here = ModelInstance::planted([10.0, 0.0, 0.0], 0.0, 1.0, SwayProfile::TREE);
        let there = ModelInstance::planted([40.0, 0.0, 0.0], 0.0, 1.0, SwayProfile::TREE);
        assert_eq!(here.sway[0], SwayProfile::TREE.tip_fraction);
        assert_eq!(here.sway[2], SwayProfile::TREE.frequency);
        assert!(
            (here.sway[1] - there.sway[1]).abs() > 0.05,
            "two positions must not share a phase"
        );
    }

    #[test]
    fn the_material_stride_is_sixteen_byte_aligned() {
        // A storage buffer's array stride must satisfy the element's alignment, and a vec4 needs 16.
        assert_eq!(MATERIAL_STRIDE % 16, 0);
    }

    #[test]
    fn a_rigid_instance_reports_its_still_height_and_a_swaying_one_reports_more() {
        // A cascade is fitted from the tallest caster. Reporting the still height for a swaying tip
        // would drop its shadow only on the frames when the wind lifted it, which reads as flickering
        // rather than as a fitting error.
        let bounds = Some([0.0, 0.0, 10.0]);
        let rigid = [ModelInstance::placed([0.0, 0.0, 5.0], 0.0, 1.0)];
        assert_eq!(world_top(bounds, &rigid), 15.0);

        let swaying = [ModelInstance::planted(
            [0.0, 0.0, 5.0],
            0.0,
            1.0,
            SwayProfile::GRASS,
        )];
        let top = world_top(bounds, &swaying);
        assert!(top > 15.0, "a swaying tip needs headroom, got {top}");
        // And the headroom is bounded rather than open-ended: 1/(1 - 0.22) of the ten units above the
        // anchor, so under thirteen.
        assert!(top < 18.0, "the headroom must stay bounded, got {top}");
    }

    #[test]
    fn no_geometry_reports_a_zero_height_rather_than_an_infinity() {
        assert_eq!(world_top(None, &[ModelInstance::default()]), 0.0);
        assert_eq!(world_top(Some([1.0; 3]), &[]), 0.0);
    }

    #[test]
    fn emissive_is_the_luminance_of_the_factor_and_its_strength() {
        let mut material = default_material();
        assert_eq!(emissive_strength(&material), 0.0, "no glow by default");
        material.emissive = [0.0, 1.0, 0.0];
        // Rec. 709 puts most of the luminance in green.
        assert!((emissive_strength(&material) - 0.7152).abs() < 1.0e-6);
        material.emissive_strength = 4.0;
        assert!((emissive_strength(&material) - 4.0 * 0.7152).abs() < 1.0e-5);
        // A negative factor is not an emitter. The coverage channel encodes emission as anything above
        // one, so a value below zero would read as *less* than full coverage and erase the pixel.
        material.emissive = [-1.0, -1.0, -1.0];
        assert_eq!(emissive_strength(&material), 0.0);
    }

    #[test]
    fn the_default_material_is_opaque_and_untextured() {
        // Every primitive with no material of its own resolves to this, so a fragment never indexes
        // past the storage buffer and never discards for want of a cutoff.
        let material = default_material();
        assert_eq!(material.alpha_mode, AlphaMode::Opaque);
        assert_eq!(material.alpha_mode.cutoff(), None);
        assert_eq!(material.base_color_texture, None);
        assert_eq!(material.metallic, 0.0);
    }
}
