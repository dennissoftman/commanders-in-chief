//! glTF 2.0 model import.
//!
//! # Why glTF rather than another custom format
//!
//! Terrain got a custom container because no standard describes a heightfield well. Models are the
//! opposite case: glTF is a published, versioned standard for exactly this, every DCC tool exports
//! it, and its data model — nodes, meshes, PBR materials, skins, animation samplers — already
//! matches what a renderer needs. Inventing a mesh format here would mean writing an exporter for
//! Blender before anyone could make a single asset.
//!
//! `.glb` is the expected form. It is a single seekable file with its buffers inline, so it needs no
//! sidecar resolution — which matters because assets arrive through [`cic_vfs`], where a relative
//! `uri` pointing at the host filesystem has no meaning and must not be followed.
//!
//! # Scope
//!
//! This imports the default scene's static geometry, flattening the node hierarchy and baking each
//! node's world transform into its vertices. Skins and animations are read as far as detecting that
//! they exist — a skinned model imports its bind-pose geometry and reports `has_skin`, rather than
//! silently dropping the rig without telling the caller.
//!
//! Images embedded in the container are decoded and normalized to straight-alpha RGBA8. That
//! normalization happens here rather than in the renderer because it is a property of the *format* —
//! glTF permits ten pixel layouts and a renderer should not have to know any of them.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

use cic_vfs::{ResourceReadError, Vfs, VirtualPath};

use crate::texture::{TextureAsset, TextureError, TextureLimits, decode_dds};

/// Explicit bounds applied while importing an untrusted model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelLimits {
    /// Maximum vertices summed across every imported primitive.
    pub maximum_vertices: usize,
    /// Maximum indices summed across every imported primitive.
    pub maximum_indices: usize,
    /// Maximum primitives retained from one model.
    pub maximum_primitives: usize,
    /// Maximum materials retained from one model.
    pub maximum_materials: usize,
    /// Maximum node-hierarchy depth walked while accumulating transforms.
    pub maximum_depth: usize,
    /// Maximum images retained from one model.
    pub maximum_images: usize,
    /// Maximum pixels along either axis of one image.
    pub maximum_image_dimension: u32,
    /// Maximum RGBA bytes summed across every retained image.
    pub maximum_image_bytes: usize,
}

impl Default for ModelLimits {
    fn default() -> Self {
        Self {
            maximum_vertices: 4 * 1_024 * 1_024,
            maximum_indices: 16 * 1_024 * 1_024,
            maximum_primitives: 4_096,
            maximum_materials: 1_024,
            maximum_depth: 128,
            maximum_images: 256,
            // The baseline device limit for a 2D texture. Anything larger cannot be uploaded, so
            // accepting it would only move the failure somewhere less informative.
            maximum_image_dimension: 8_192,
            maximum_image_bytes: 512 * 1_024 * 1_024,
        }
    }
}

/// One decoded image from a model, as straight-alpha RGBA8.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row-major RGBA bytes from the top-left, `width * height * 4` long.
    pub rgba: Vec<u8>,
    /// The name the container gave this image, or an empty string when it gave none.
    ///
    /// Carried for one purpose: it is the key a block-compressed sidecar is found under. See
    /// [`resolve_model_textures`], and [`ModelTextures`] for why the override is a separate table rather
    /// than a second shape this struct can take.
    pub name: String,
}

/// One imported vertex.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelVertex {
    /// Position in the model's own space, with the node transform already applied.
    pub position: [f32; 3],
    /// Unit normal, likewise transformed.
    pub normal: [f32; 3],
    /// First texture coordinate set, or zero when the primitive has none.
    pub uv: [f32; 2],
    /// Unit tangent in `xyz` and the bitangent's handedness in `w`, as glTF defines it.
    ///
    /// A normal map stores a perturbation in *texture* space, so reading one needs the surface basis
    /// that texture space corresponds to — and that basis is a property of how the UVs were laid out,
    /// which nothing but the mesh knows. glTF makes `TANGENT` optional and says to derive it from the
    /// texture coordinates when a normal-mapped primitive omits it, which is what
    /// [`generate_tangents`] does.
    ///
    /// `w` is `+1` or `-1` and selects which way the bitangent points, because a mirrored UV island
    /// flips handedness without changing either the normal or the tangent. Storing the sign is four
    /// bytes; storing the bitangent itself is twelve and can disagree with the other two.
    pub tangent: [f32; 4],
}

impl Default for ModelVertex {
    /// A vertex at the origin facing `+Z`, with no texture coordinates and an unset tangent.
    ///
    /// The tangent is `(0, 0, 0, 1)` rather than a unit vector, matching what the importer writes for a
    /// primitive whose material has no normal map: nothing reads it, and a zero says so.
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            normal: [0.0, 0.0, 1.0],
            uv: [0.0; 2],
            tangent: [0.0, 0.0, 0.0, 1.0],
        }
    }
}

/// How a material's alpha is meant to be interpreted.
///
/// glTF's three modes, kept as an enum rather than reduced to a boolean. The distinction is
/// load-bearing for a deferred renderer: masked geometry can be drawn in the G-buffer and in every
/// shadow cascade by discarding fragments, and blended geometry fundamentally cannot, because a
/// G-buffer pixel holds one material and blending needs two.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AlphaMode {
    /// Alpha is ignored and the surface is fully opaque.
    Opaque,
    /// A fragment is drawn or discarded according to whether its alpha reaches `cutoff`.
    ///
    /// The mode foliage is authored in: a leaf card is a quad whose texture's alpha is the leaf
    /// outline, and everything a renderer needs to know about it is where to cut.
    Masked {
        /// Alpha at or above which the fragment survives.
        cutoff: f32,
    },
    /// Alpha is a coverage fraction to blend by.
    Blended,
}

impl AlphaMode {
    /// The alpha at which a fragment survives, for a mode that cuts.
    ///
    /// `None` for [`Self::Opaque`], which draws every fragment. [`Self::Blended`] reports the same
    /// cutoff a masked material would default to, because a renderer that cannot blend has to choose
    /// between drawing a blended surface as opaque and not drawing it at all — and a cut at half
    /// coverage is the closer of the two answers. Whether to take that answer is the renderer's
    /// decision; this only supplies the figure.
    #[must_use]
    pub const fn cutoff(self) -> Option<f32> {
        match self {
            Self::Opaque => None,
            Self::Masked { cutoff } => Some(cutoff),
            Self::Blended => Some(DEFAULT_ALPHA_CUTOFF),
        }
    }
}

/// The cutoff glTF applies when a masked material declares none.
pub const DEFAULT_ALPHA_CUTOFF: f32 = 0.5;

/// One imported triangle list.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelPrimitive {
    /// Vertices, in accessor order.
    pub vertices: Vec<ModelVertex>,
    /// Triangle indices into `vertices`.
    pub indices: Vec<u32>,
    /// Index into [`Model::materials`], or `None` for the glTF default material.
    pub material: Option<usize>,
}

impl ModelPrimitive {
    /// Fills in a tangent frame from this primitive's positions and texture coordinates.
    ///
    /// Public because a `Model` is a plain struct that anything may build: test fixtures do, and so will
    /// procedural content and the map editor. Such a caller has the same problem an exporter that omits
    /// `TANGENT` has — a normal map needs the surface basis its texture space corresponds to, and only
    /// the mesh knows it — and should not have to reimplement the derivation to solve it.
    ///
    /// The importer calls this for any normal-mapped primitive whose source omitted `TANGENT`. Calling it
    /// on a primitive that already has one overwrites it, which is the caller's decision to make.
    #[must_use]
    pub fn with_generated_tangents(mut self) -> Self {
        generate_tangents(&mut self.vertices, &self.indices);
        self
    }
}

/// A physically-based material, in the subset a first renderer needs.
///
/// # Why the map indices are separate fields rather than a table
///
/// glTF allows any texture to be shared between materials and between slots, so the natural encoding
/// is an index per slot into one image list. Naming the slots explicitly rather than holding a
/// `Vec<(Slot, usize)>` is what makes the *absence* of a map a type-level fact: a renderer reads
/// `normal_texture` and gets `None`, instead of searching a list and having to decide what a missing
/// entry means.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelMaterial {
    /// Material name, or an empty string when unnamed.
    pub name: String,
    /// Linear base colour and alpha.
    pub base_color: [f32; 4],
    /// Metallic factor in `0..=1`, multiplied by the blue channel of
    /// [`Self::metallic_roughness_texture`] where one is present.
    pub metallic: f32,
    /// Roughness factor in `0..=1`, multiplied by the green channel of
    /// [`Self::metallic_roughness_texture`] where one is present.
    pub roughness: f32,
    /// Index of the base-colour texture within the glTF's image list, if any.
    ///
    /// This one image is sRGB-encoded; every other map below is not. That asymmetry is the spec's, and
    /// it is worth stating here because it decides the texture format each map uploads in: base colour
    /// carries a colour a human authored, and the rest carry measurements.
    pub base_color_texture: Option<usize>,
    /// Index of the tangent-space normal map, if any.
    pub normal_texture: Option<usize>,
    /// Multiplier on the normal map's `xy`, which is how strongly it perturbs the surface.
    ///
    /// One leaves the map as authored. Applied to the tangent-space `xy` before the `z` is rebuilt, so
    /// a scale of zero yields the geometric normal exactly rather than a flattened approximation of it.
    pub normal_scale: f32,
    /// Index of the combined metallic-roughness map, if any.
    ///
    /// glTF packs both into one image: roughness in green, metallic in blue. Red and alpha are unused
    /// by the core spec, which is why an occlusion map is so often the same image.
    pub metallic_roughness_texture: Option<usize>,
    /// Index of the ambient-occlusion map, if any. Occlusion is in red.
    pub occlusion_texture: Option<usize>,
    /// How much of the occlusion map to apply, in `0..=1`.
    pub occlusion_strength: f32,
    /// Linear emissive colour, before [`Self::emissive_strength`].
    pub emissive: [f32; 3],
    /// Multiplier on [`Self::emissive`], from `KHR_materials_emissive_strength`.
    ///
    /// One when the extension is absent, which is the value that makes the extension's presence
    /// invisible to a reader that does not care about it.
    pub emissive_strength: f32,
    /// How the material's alpha is meant to be interpreted.
    pub alpha_mode: AlphaMode,
    /// Whether the material asks to be drawn from both sides.
    ///
    /// Foliage needs it: a leaf card is a single quad meant to be seen from either face, and
    /// back-face culling makes half of a canopy vanish depending on where the camera stands.
    pub double_sided: bool,
}

impl Default for ModelMaterial {
    /// The material glTF defines for a primitive that declares none.
    ///
    /// Its figures are the spec's, not a choice made here: opaque white, fully metallic, fully rough. Two
    /// of those are surprising and both are deliberate in the spec — a fully rough metal has no visible
    /// highlight and almost no diffuse term, so a mesh that reaches a renderer with no material at all
    /// looks obviously unfinished rather than plausibly grey.
    ///
    /// A renderer wanting a *neutral* stand-in should not use this; see `pack_materials` in
    /// `cic-render`, which constructs a mid-grey dielectric and says why.
    fn default() -> Self {
        Self {
            name: String::new(),
            base_color: [1.0; 4],
            metallic: 1.0,
            roughness: 1.0,
            base_color_texture: None,
            normal_texture: None,
            normal_scale: 1.0,
            metallic_roughness_texture: None,
            occlusion_texture: None,
            occlusion_strength: 1.0,
            emissive: [0.0; 3],
            emissive_strength: 1.0,
            alpha_mode: AlphaMode::Opaque,
            double_sided: false,
        }
    }
}

/// An imported model.
#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    /// The default scene's name, or an empty string.
    pub name: String,
    /// Flattened triangle lists.
    pub primitives: Vec<ModelPrimitive>,
    /// Materials referenced by `primitives`.
    pub materials: Vec<ModelMaterial>,
    /// Decoded images, indexed by [`ModelMaterial::base_color_texture`].
    pub images: Vec<ModelImage>,
    /// Whether the source declared any skin, so a caller knows bind-pose geometry was imported
    /// rather than assuming the model is genuinely static.
    pub has_skin: bool,
    /// Whether the source declared any animation.
    pub has_animation: bool,
}

impl Model {
    /// Returns the total vertex count across every primitive.
    #[must_use]
    pub fn vertex_count(&self) -> usize {
        self.primitives.iter().map(|p| p.vertices.len()).sum()
    }

    /// Returns the total triangle count across every primitive.
    #[must_use]
    pub fn triangle_count(&self) -> usize {
        self.primitives.iter().map(|p| p.indices.len() / 3).sum()
    }

    /// Returns the axis-aligned bounds as `(minimum, maximum)`, or `None` when there is no geometry.
    #[must_use]
    pub fn bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        let mut minimum = [f32::INFINITY; 3];
        let mut maximum = [f32::NEG_INFINITY; 3];
        let mut seen = false;
        for primitive in &self.primitives {
            for vertex in &primitive.vertices {
                seen = true;
                for axis in 0..3 {
                    minimum[axis] = minimum[axis].min(vertex.position[axis]);
                    maximum[axis] = maximum[axis].max(vertex.position[axis]);
                }
            }
        }
        seen.then_some((minimum, maximum))
    }
}

/// Directory a model's block-compressed textures are looked up in, relative to the mount root.
///
/// One directory for the whole package rather than one beside each model, because glTF image names are
/// already the sharing mechanism: two models that name the same image are meant to get the same texture,
/// and a per-model directory would silently give them two.
pub const TEXTURE_DIRECTORY: &str = "textures";

/// Block-compressed textures found for a model's images, parallel to [`Model::images`] by index.
///
/// # Why an override table rather than a second shape for `ModelImage`
///
/// The alternative is a `ModelImage` that holds *either* RGBA8 pixels or compressed blocks. That reads
/// well until something has to consume it: every existing reader of `.rgba` would have to branch, and
/// the branch is a decode — so the type that was supposed to describe an image would be the thing
/// deciding when the CPU spends a pass on one.
///
/// Keeping the override beside the model instead means the compressed bytes reach the uploader without
/// anything in between having to look at them, and a caller that does not know about block compression
/// keeps working exactly as it did. The index parallelism is the same invariant `base_color_texture`
/// already relies on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelTextures {
    assets: Vec<Option<TextureAsset>>,
}

impl ModelTextures {
    /// Wraps an override per image index, for a caller that resolved them some other way.
    ///
    /// [`resolve_model_textures`] is the ordinary way in. This exists for the same reason
    /// [`ModelPrimitive::with_generated_tangents`] is public: a `Model` is a plain struct that procedural
    /// content, the map editor and test fixtures all build directly, and such a caller should not have to
    /// mount a virtual filesystem to say which textures its images have.
    ///
    /// The vector is parallel to [`Model::images`] by index; entries past its end are simply never found.
    #[must_use]
    pub const fn new(assets: Vec<Option<TextureAsset>>) -> Self {
        Self { assets }
    }

    /// Returns the compressed texture found for one image index, if any.
    #[must_use]
    pub fn get(&self, image: usize) -> Option<&TextureAsset> {
        self.assets.get(image)?.as_ref()
    }

    /// Whether any image at all was overridden.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.assets.iter().all(Option::is_none)
    }

    /// How many of the model's images were overridden.
    #[must_use]
    pub fn resolved_count(&self) -> usize {
        self.assets.iter().filter(|asset| asset.is_some()).count()
    }
}

/// Looks up a block-compressed sidecar for each of a model's images.
///
/// # The convention
///
/// A glTF image named `hull_basecolor` is overridden by `textures/hull_basecolor.dds`. The name is the
/// glTF image's own `name`, so the link is authored in the DCC tool rather than derived from a filename
/// the container may not carry — and an unnamed image is simply never overridden.
///
/// The `.glb` still declares its images, because `gltf::import_slice` refuses a `uri` pointing outside
/// the container and *must*: following one would read the host filesystem from an untrusted asset. So an
/// author has two working arrangements, and both are supported by this being an override rather than a
/// replacement: keep the authored PNG embedded and let the sidecar win at runtime, or embed a 1x1
/// placeholder once the sidecar exists and pay nothing for it.
///
/// # Errors
///
/// An *absent* sidecar is not an error — that is the ordinary case for a model whose textures have not
/// been converted. A sidecar that exists and will not read is: it means a converted texture is being
/// silently rendered from its placeholder, which is exactly the failure a content author needs told
/// about rather than left to notice.
pub fn resolve_model_textures(
    model: &Model,
    vfs: &Vfs,
    limits: TextureLimits,
) -> Result<ModelTextures, ModelTextureError> {
    let mut assets = Vec::with_capacity(model.images.len());
    for image in &model.images {
        if image.name.is_empty() {
            assets.push(None);
            continue;
        }
        let path = format!("{TEXTURE_DIRECTORY}/{}.dds", image.name);
        let virtual_path = VirtualPath::new(&path).map_err(|error| ModelTextureError::Path {
            path: path.clone(),
            error,
        })?;
        let Some(entry) = vfs.resolve(&virtual_path) else {
            assets.push(None);
            continue;
        };
        let bytes = entry
            .read(limits.maximum_bytes)
            .map_err(|error| ModelTextureError::Read {
                path: path.clone(),
                error,
            })?;
        let texture = decode_dds(&bytes, limits)
            .map_err(|error| ModelTextureError::Texture { path, error })?;
        assets.push(Some(texture));
    }
    Ok(ModelTextures::new(assets))
}

/// A structured failure while resolving a model's block-compressed sidecars.
#[derive(Debug)]
pub enum ModelTextureError {
    /// An image name did not form a safe virtual path.
    Path {
        /// The path that was attempted.
        path: String,
        /// The underlying normalization failure.
        error: cic_vfs::PathError,
    },
    /// A sidecar existed but could not be read.
    Read {
        /// Mount-relative path of the sidecar.
        path: String,
        /// The underlying read failure.
        error: ResourceReadError,
    },
    /// A sidecar was read but is not a texture this engine can upload.
    Texture {
        /// Mount-relative path of the sidecar.
        path: String,
        /// The underlying container or format failure.
        error: TextureError,
    },
}

impl Display for ModelTextureError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Path { path, error } => {
                write!(formatter, "texture path `{path}` is unusable: {error}")
            }
            Self::Read { path, error } => {
                write!(formatter, "texture `{path}` could not be read: {error}")
            }
            Self::Texture { path, error } => {
                write!(formatter, "texture `{path}` is unusable: {error}")
            }
        }
    }
}

impl Error for ModelTextureError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Path { error, .. } => Some(error),
            Self::Read { error, .. } => Some(error),
            Self::Texture { error, .. } => Some(error),
        }
    }
}

/// Imports a `.glb` or self-contained `.gltf` model from bytes.
///
/// # Errors
///
/// Returns a structured [`ModelError`] when the container is malformed, references external files,
/// omits required attributes, uses a non-triangle topology, or exceeds a [`ModelLimits`] bound.
pub fn import_model(bytes: &[u8], limits: ModelLimits) -> Result<Model, ModelError> {
    let (document, buffers, images) =
        gltf::import_slice(bytes).map_err(|error| ModelError::Gltf(Box::new(error)))?;

    if document.materials().len() > limits.maximum_materials {
        return Err(ModelError::LimitExceeded {
            what: "material count",
            actual: document.materials().len(),
            maximum: limits.maximum_materials,
        });
    }
    // Names come from the document and the pixels from the decoded data, and the two are parallel by
    // index. Collected together here because a `ModelImage` needs both and only the document has the
    // name — which is the key a block-compressed sidecar is found under.
    let names: Vec<String> = document
        .images()
        .map(|image| image.name().unwrap_or_default().to_owned())
        .collect();
    let images = import_images(&images, &names, limits)?;

    let materials = document
        .materials()
        .map(|material| {
            let pbr = material.pbr_metallic_roughness();
            let normal = material.normal_texture();
            let occlusion = material.occlusion_texture();
            ModelMaterial {
                name: material.name().unwrap_or_default().to_owned(),
                base_color: pbr.base_color_factor(),
                metallic: pbr.metallic_factor(),
                roughness: pbr.roughness_factor(),
                base_color_texture: pbr
                    .base_color_texture()
                    .map(|info| info.texture().source().index()),
                normal_texture: normal.as_ref().map(|info| info.texture().source().index()),
                // The scale is the map's, so it defaults to one only when there is no map to scale.
                normal_scale: normal
                    .as_ref()
                    .map_or(1.0, gltf::material::NormalTexture::scale),
                metallic_roughness_texture: pbr
                    .metallic_roughness_texture()
                    .map(|info| info.texture().source().index()),
                occlusion_texture: occlusion
                    .as_ref()
                    .map(|info| info.texture().source().index()),
                occlusion_strength: occlusion
                    .as_ref()
                    .map_or(1.0, gltf::material::OcclusionTexture::strength),
                emissive: material.emissive_factor(),
                // Absent means one rather than zero: the extension multiplies the factor, so a document
                // without it must read exactly as it did before the extension existed.
                emissive_strength: material.emissive_strength().unwrap_or(1.0),
                alpha_mode: match material.alpha_mode() {
                    gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
                    gltf::material::AlphaMode::Mask => AlphaMode::Masked {
                        cutoff: material.alpha_cutoff().unwrap_or(DEFAULT_ALPHA_CUTOFF),
                    },
                    gltf::material::AlphaMode::Blend => AlphaMode::Blended,
                },
                double_sided: material.double_sided(),
            }
        })
        .collect::<Vec<_>>();

    // Which materials carry a normal map, so a primitive using one gets tangents derived when its
    // author omitted them. Collected before the walk because a primitive knows only its material
    // *index*, and deriving tangents for every primitive regardless would spend the work on the
    // majority of meshes that will never read a tangent.
    let normal_mapped = materials
        .iter()
        .map(|material| material.normal_texture.is_some())
        .collect::<Vec<_>>();

    let scene = document
        .default_scene()
        .or_else(|| document.scenes().next())
        .ok_or(ModelError::NoScene)?;

    let mut primitives = Vec::new();
    let mut totals = Totals::default();
    for node in scene.nodes() {
        walk(
            &node,
            IDENTITY,
            0,
            &buffers,
            limits,
            &normal_mapped,
            &mut primitives,
            &mut totals,
        )?;
    }

    Ok(Model {
        name: scene.name().unwrap_or_default().to_owned(),
        primitives,
        materials,
        images,
        has_skin: document.skins().len() > 0,
        has_animation: document.animations().len() > 0,
    })
}

/// Normalizes every decoded image to straight-alpha RGBA8, under explicit bounds.
///
/// The index of each image is preserved, because [`ModelMaterial::base_color_texture`] is that index.
/// An image that cannot be normalized therefore fails the import rather than being dropped, which
/// would silently shift every later material onto the wrong picture.
fn import_images(
    images: &[gltf::image::Data],
    names: &[String],
    limits: ModelLimits,
) -> Result<Vec<ModelImage>, ModelError> {
    if images.len() > limits.maximum_images {
        return Err(ModelError::LimitExceeded {
            what: "image count",
            actual: images.len(),
            maximum: limits.maximum_images,
        });
    }
    let mut total = 0usize;
    let mut output = Vec::with_capacity(images.len());
    for (index, image) in images.iter().enumerate() {
        let width = usize::try_from(image.width).unwrap_or(usize::MAX);
        let height = usize::try_from(image.height).unwrap_or(usize::MAX);
        if image.width > limits.maximum_image_dimension
            || image.height > limits.maximum_image_dimension
        {
            return Err(ModelError::LimitExceeded {
                what: "image dimension",
                actual: width.max(height),
                maximum: usize::try_from(limits.maximum_image_dimension).unwrap_or(usize::MAX),
            });
        }
        let pixels = width.saturating_mul(height);
        // Accumulated and checked before the conversion allocates, not after.
        total = total.saturating_add(pixels.saturating_mul(4));
        if total > limits.maximum_image_bytes {
            return Err(ModelError::LimitExceeded {
                what: "image bytes",
                actual: total,
                maximum: limits.maximum_image_bytes,
            });
        }
        output.push(ModelImage {
            width: image.width,
            height: image.height,
            rgba: to_rgba8(image, pixels)?,
            name: names.get(index).cloned().unwrap_or_default(),
        });
    }
    Ok(output)
}

/// Widens one decoded image to RGBA8.
///
/// The 16-bit layouts keep their high byte. That is a real loss and it is the right one here: the
/// renderer's colour arrays are 8-bit, so the alternative is not more precision but a second format
/// path that discards the same bits one stage later.
///
/// The floating-point layouts are refused rather than tone mapped. They carry linear values outside
/// `0..=1`, and guessing an exposure for them would produce a picture nobody authored.
fn to_rgba8(image: &gltf::image::Data, pixels: usize) -> Result<Vec<u8>, ModelError> {
    use gltf::image::Format;

    // Channels per pixel, and bytes per channel. A 16-bit channel keeps its most significant byte,
    // which is the second one: the decoder emits native-endian samples and every target this builds
    // for is little-endian.
    let (channels, channel_bytes) = match image.format {
        Format::R8 => (1usize, 1usize),
        Format::R8G8 => (2, 1),
        Format::R8G8B8 => (3, 1),
        Format::R8G8B8A8 => (4, 1),
        Format::R16 => (1, 2),
        Format::R16G16 => (2, 2),
        Format::R16G16B16 => (3, 2),
        Format::R16G16B16A16 => (4, 2),
        format => {
            return Err(ModelError::UnsupportedImageFormat {
                format: format!("{format:?}"),
            });
        }
    };

    let stride = channels * channel_bytes;
    let mut rgba = Vec::with_capacity(pixels.saturating_mul(4));
    for pixel in image.pixels.chunks_exact(stride) {
        // `channel` is below `channels` and the chunk is exactly `channels * channel_bytes` long, so
        // the highest index reached is the chunk's last byte.
        let at = |channel: usize| pixel[channel * channel_bytes + (channel_bytes - 1)];
        let red = at(0);
        rgba.push(red);
        // One and two channels mean greyscale, with the second channel as alpha where it exists —
        // which is what the glTF material rules amount to for a base-colour texture.
        rgba.push(if channels >= 3 { at(1) } else { red });
        rgba.push(if channels >= 3 { at(2) } else { red });
        rgba.push(match channels {
            2 => at(1),
            4 => at(3),
            _ => u8::MAX,
        });
    }
    if rgba.len() != pixels.saturating_mul(4) {
        return Err(ModelError::TruncatedImage {
            width: image.width,
            height: image.height,
        });
    }
    Ok(rgba)
}

#[derive(Debug, Default)]
struct Totals {
    vertices: usize,
    indices: usize,
}

const IDENTITY: [[f32; 4]; 4] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

#[allow(clippy::too_many_arguments)]
fn walk(
    node: &gltf::Node<'_>,
    parent: [[f32; 4]; 4],
    depth: usize,
    buffers: &[gltf::buffer::Data],
    limits: ModelLimits,
    normal_mapped: &[bool],
    output: &mut Vec<ModelPrimitive>,
    totals: &mut Totals,
) -> Result<(), ModelError> {
    if depth > limits.maximum_depth {
        return Err(ModelError::LimitExceeded {
            what: "node depth",
            actual: depth,
            maximum: limits.maximum_depth,
        });
    }
    let world = multiply(parent, node.transform().matrix());

    if let Some(mesh) = node.mesh() {
        for primitive in mesh.primitives() {
            if output.len() >= limits.maximum_primitives {
                return Err(ModelError::LimitExceeded {
                    what: "primitive count",
                    actual: output.len() + 1,
                    maximum: limits.maximum_primitives,
                });
            }
            output.push(import_primitive(
                &primitive,
                world,
                buffers,
                limits,
                normal_mapped,
                totals,
            )?);
        }
    }

    for child in node.children() {
        walk(
            &child,
            world,
            depth.saturating_add(1),
            buffers,
            limits,
            normal_mapped,
            output,
            totals,
        )?;
    }
    Ok(())
}

/// Reads one triangle-list primitive, baking `world` into its vertices.
fn import_primitive(
    primitive: &gltf::Primitive<'_>,
    world: [[f32; 4]; 4],
    buffers: &[gltf::buffer::Data],
    limits: ModelLimits,
    normal_mapped: &[bool],
    totals: &mut Totals,
) -> Result<ModelPrimitive, ModelError> {
    if primitive.mode() != gltf::mesh::Mode::Triangles {
        return Err(ModelError::UnsupportedTopology {
            mode: format!("{:?}", primitive.mode()),
        });
    }

    let reader = primitive.reader(|buffer| buffers.get(buffer.index()).map(|data| &data.0[..]));
    let positions = reader
        .read_positions()
        .ok_or(ModelError::MissingAttribute("POSITION"))?
        .collect::<Vec<_>>();

    totals.vertices += positions.len();
    if totals.vertices > limits.maximum_vertices {
        return Err(ModelError::LimitExceeded {
            what: "vertex count",
            actual: totals.vertices,
            maximum: limits.maximum_vertices,
        });
    }

    // A missing normal set is legal glTF -- the spec says to compute flat normals. Rather than
    // silently shipping zero normals to the GPU, they are generated below.
    let normals: Option<Vec<[f32; 3]>> = reader.read_normals().map(Iterator::collect);
    let uvs = reader
        .read_tex_coords(0)
        .map(|iter| iter.into_f32().collect::<Vec<_>>());
    let tangents: Option<Vec<[f32; 4]>> = reader.read_tangents().map(Iterator::collect);

    let indices = match reader.read_indices() {
        Some(indices) => indices.into_u32().collect::<Vec<_>>(),
        // An unindexed primitive is a plain triangle sequence.
        None => (0..u32::try_from(positions.len()).map_err(|_| ModelError::LimitExceeded {
            what: "vertex count",
            actual: positions.len(),
            maximum: u32::MAX as usize,
        })?)
            .collect(),
    };
    if !indices.len().is_multiple_of(3) {
        return Err(ModelError::IndexCountNotTriangles(indices.len()));
    }
    totals.indices += indices.len();
    if totals.indices > limits.maximum_indices {
        return Err(ModelError::LimitExceeded {
            what: "index count",
            actual: totals.indices,
            maximum: limits.maximum_indices,
        });
    }
    // Checked before any vertex is built, so a hostile index cannot reach an indexing operation.
    for index in &indices {
        if *index as usize >= positions.len() {
            return Err(ModelError::IndexOutOfRange {
                index: *index,
                vertices: positions.len(),
            });
        }
    }

    let normal_matrix = normal_basis(world);
    let mut vertices = Vec::with_capacity(positions.len());
    for (position_index, position) in positions.iter().enumerate() {
        let normal = normals
            .as_ref()
            .and_then(|set| set.get(position_index).copied())
            .unwrap_or([0.0, 0.0, 0.0]);
        // A supplied tangent is a direction in the same space as the positions, so it takes the same
        // basis as the normal. Its `w` is a handedness sign and must pass through untransformed --
        // scaling or rotating it would turn a flag into a number.
        let tangent = tangents
            .as_ref()
            .and_then(|set| set.get(position_index).copied())
            .map_or([0.0, 0.0, 0.0, 1.0], |tangent| {
                let direction = normalize(transform_direction(
                    normal_matrix,
                    [tangent[0], tangent[1], tangent[2]],
                ));
                [
                    direction[0],
                    direction[1],
                    direction[2],
                    if tangent[3] < 0.0 { -1.0 } else { 1.0 },
                ]
            });
        vertices.push(ModelVertex {
            position: transform_point(world, *position),
            normal: normalize(transform_direction(normal_matrix, normal)),
            uv: uvs
                .as_ref()
                .and_then(|set| set.get(position_index).copied())
                .unwrap_or([0.0, 0.0]),
            tangent,
        });
    }
    if normals.is_none() {
        generate_flat_normals(&mut vertices, &indices);
    }
    // Derived only where a tangent will actually be read. glTF requires `TANGENT` on a normal-mapped
    // primitive in principle and exporters routinely omit it, so the fallback is not an edge case; but
    // a primitive with no normal map has nothing to read a tangent frame in, and the derivation is the
    // most expensive thing in this function.
    let wants_tangents = primitive
        .material()
        .index()
        .is_some_and(|index| normal_mapped.get(index).copied().unwrap_or(false));
    if tangents.is_none() && wants_tangents {
        generate_tangents(&mut vertices, &indices);
    }

    Ok(ModelPrimitive {
        vertices,
        indices,
        material: primitive.material().index(),
    })
}

/// Accumulates area-weighted face normals, which is what the glTF spec's flat-normal fallback
/// amounts to for a shared-vertex triangle list.
fn generate_flat_normals(vertices: &mut [ModelVertex], indices: &[u32]) {
    for vertex in vertices.iter_mut() {
        vertex.normal = [0.0; 3];
    }
    for triangle in indices.chunks_exact(3) {
        let [a, b, c] = [
            triangle[0] as usize,
            triangle[1] as usize,
            triangle[2] as usize,
        ];
        let edge1 = subtract(vertices[b].position, vertices[a].position);
        let edge2 = subtract(vertices[c].position, vertices[a].position);
        let face = cross(edge1, edge2);
        for index in [a, b, c] {
            for (component, contribution) in vertices[index].normal.iter_mut().zip(face) {
                *component += contribution;
            }
        }
    }
    for vertex in vertices.iter_mut() {
        vertex.normal = normalize(vertex.normal);
    }
}

/// Derives a per-vertex tangent frame from the texture coordinates, for a normal-mapped primitive
/// whose author supplied none.
///
/// # What it computes
///
/// A normal map's `x` axis is "the direction on the surface along which `u` increases". For one
/// triangle that is a linear system: the two edge vectors are known in both position and UV space, so
/// solving the 2x2 gives the position-space direction of `du` and of `dv` exactly. Each triangle's
/// solution is accumulated at its three vertices and the sum normalized, which averages the frame
/// across a smooth surface the same way the flat-normal fallback averages face normals.
///
/// The accumulated tangent is then Gram-Schmidt orthogonalized against the *vertex* normal rather than
/// used as it came out. The two disagree wherever the surface is curved — the tangent is a chord
/// direction and the normal is interpolated — and a basis whose axes are not perpendicular skews the
/// perturbation, tilting a normal map's flat regions.
///
/// # Handedness
///
/// `w` records whether the bitangent is `cross(normal, tangent)` or its negation, which is decided by
/// the sign of the UV parametrisation's determinant. A mirrored UV island — an extremely common way to
/// texture a symmetric object with half the atlas — has the opposite sign from its twin while sharing
/// both the normal and the tangent direction, so without this the lighting on one half of the model is
/// inverted along one axis. That failure looks like a lighting bug rather than a data one, which is
/// why it is worth the four bytes.
///
/// # Degenerate cases
///
/// A triangle with zero UV area contributes nothing rather than an infinity: its determinant is zero
/// and the solve is skipped. A vertex left with no contribution at all — every triangle touching it
/// degenerate, or the primitive having no texture coordinates — falls back to any unit vector
/// perpendicular to its normal, which is an arbitrary but *valid* frame. The alternative, a zero
/// tangent, produces a zero-length basis vector and a normal that is not a direction.
pub fn generate_tangents(vertices: &mut [ModelVertex], indices: &[u32]) {
    let mut accumulated = vec![[0.0f32; 3]; vertices.len()];
    let mut handedness = vec![0.0f32; vertices.len()];

    for triangle in indices.chunks_exact(3) {
        let [a, b, c] = [
            triangle[0] as usize,
            triangle[1] as usize,
            triangle[2] as usize,
        ];
        let edge1 = subtract(vertices[b].position, vertices[a].position);
        let edge2 = subtract(vertices[c].position, vertices[a].position);
        let uv1 = [
            vertices[b].uv[0] - vertices[a].uv[0],
            vertices[b].uv[1] - vertices[a].uv[1],
        ];
        let uv2 = [
            vertices[c].uv[0] - vertices[a].uv[0],
            vertices[c].uv[1] - vertices[a].uv[1],
        ];
        // The determinant of the UV edge pair. Zero means the triangle occupies no area in texture
        // space, so no direction on it corresponds to `du` and there is nothing to contribute.
        let determinant = uv1[0] * uv2[1] - uv2[0] * uv1[1];
        if determinant.abs() < 1.0e-12 || !determinant.is_finite() {
            continue;
        }
        let inverse = 1.0 / determinant;
        let tangent = [
            (edge1[0] * uv2[1] - edge2[0] * uv1[1]) * inverse,
            (edge1[1] * uv2[1] - edge2[1] * uv1[1]) * inverse,
            (edge1[2] * uv2[1] - edge2[2] * uv1[1]) * inverse,
        ];
        if !tangent.iter().all(|value| value.is_finite()) {
            continue;
        }
        for index in [a, b, c] {
            for (component, contribution) in accumulated[index].iter_mut().zip(tangent) {
                *component += contribution;
            }
            // Summed rather than assigned, so a vertex shared between islands of opposite handedness
            // takes whichever contributed more triangles instead of whichever was visited last.
            handedness[index] += determinant;
        }
    }

    for (index, vertex) in vertices.iter_mut().enumerate() {
        let normal = vertex.normal;
        let raw = accumulated[index];
        // Gram-Schmidt: remove the component along the normal, leaving the part that lies in the
        // tangent plane. See the handedness note above for why the sign is carried separately.
        let along = dot(raw, normal);
        let orthogonal = [
            raw[0] - normal[0] * along,
            raw[1] - normal[1] * along,
            raw[2] - normal[2] * along,
        ];
        let length = length(orthogonal);
        let direction = if length > 1.0e-8 && length.is_finite() {
            [
                orthogonal[0] / length,
                orthogonal[1] / length,
                orthogonal[2] / length,
            ]
        } else {
            perpendicular(normal)
        };
        vertex.tangent = [
            direction[0],
            direction[1],
            direction[2],
            if handedness[index] < 0.0 { -1.0 } else { 1.0 },
        ];
    }
}

/// Any unit vector perpendicular to `normal`.
///
/// The axis crossed against is chosen to be the one `normal` points along *least*, so the cross
/// product is never near zero — crossing against a fixed axis fails exactly when the surface faces
/// along it, which for terrain-standing geometry is the common case rather than a rare one.
fn perpendicular(normal: [f32; 3]) -> [f32; 3] {
    let axis = if normal[0].abs() <= normal[1].abs() && normal[0].abs() <= normal[2].abs() {
        [1.0, 0.0, 0.0]
    } else if normal[1].abs() <= normal[2].abs() {
        [0.0, 1.0, 0.0]
    } else {
        [0.0, 0.0, 1.0]
    };
    normalize(cross(normal, axis))
}

fn dot(left: [f32; 3], right: [f32; 3]) -> f32 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn length(vector: [f32; 3]) -> f32 {
    dot(vector, vector).sqrt()
}

fn multiply(left: [[f32; 4]; 4], right: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    // glTF matrices are column-major: `matrix[column][row]`.
    let mut result = [[0.0f32; 4]; 4];
    for column in 0..4 {
        for row in 0..4 {
            let mut sum = 0.0;
            for step in 0..4 {
                sum += left[step][row] * right[column][step];
            }
            result[column][row] = sum;
        }
    }
    result
}

fn transform_point(matrix: [[f32; 4]; 4], point: [f32; 3]) -> [f32; 3] {
    let mut result = [0.0f32; 3];
    for row in 0..3 {
        result[row] = matrix[0][row] * point[0]
            + matrix[1][row] * point[1]
            + matrix[2][row] * point[2]
            + matrix[3][row];
    }
    result
}

fn transform_direction(matrix: [[f32; 3]; 3], direction: [f32; 3]) -> [f32; 3] {
    let mut result = [0.0f32; 3];
    for row in 0..3 {
        result[row] = matrix[0][row] * direction[0]
            + matrix[1][row] * direction[1]
            + matrix[2][row] * direction[2];
    }
    result
}

/// Returns the upper-left 3x3 of a transform, which is the correct basis for a normal as long as the
/// transform carries no non-uniform scale or shear.
///
/// A full inverse-transpose is the general answer. It is deliberately not used here: node transforms
/// in practice are rotate-translate-uniform-scale, where this agrees exactly, and the normals are
/// renormalized afterward — so the general case would cost an inverse per node to change nothing.
const fn normal_basis(matrix: [[f32; 4]; 4]) -> [[f32; 3]; 3] {
    [
        [matrix[0][0], matrix[0][1], matrix[0][2]],
        [matrix[1][0], matrix[1][1], matrix[1][2]],
        [matrix[2][0], matrix[2][1], matrix[2][2]],
    ]
}

fn subtract(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn cross(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn normalize(vector: [f32; 3]) -> [f32; 3] {
    let length = (vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]).sqrt();
    if length > 0.0 && length.is_finite() {
        [vector[0] / length, vector[1] / length, vector[2] / length]
    } else {
        [0.0, 0.0, 1.0]
    }
}

/// A structured failure while importing a model.
#[derive(Debug)]
pub enum ModelError {
    /// The glTF container itself was malformed, or referenced an external resource.
    Gltf(Box<gltf::Error>),
    /// The document declared no scene.
    NoScene,
    /// A required vertex attribute was absent.
    MissingAttribute(&'static str),
    /// A primitive used a topology other than triangles.
    UnsupportedTopology {
        /// The declared mode.
        mode: String,
    },
    /// A primitive's index count was not a multiple of three.
    IndexCountNotTriangles(usize),
    /// An index pointed past the primitive's vertex array.
    IndexOutOfRange {
        /// The offending index.
        index: u32,
        /// Vertices actually present.
        vertices: usize,
    },
    /// An explicit [`ModelLimits`] bound was exceeded.
    LimitExceeded {
        /// Which bound was crossed.
        what: &'static str,
        /// Observed value.
        actual: usize,
        /// Configured maximum.
        maximum: usize,
    },
    /// An image used a pixel layout this importer does not normalize to RGBA8.
    UnsupportedImageFormat {
        /// The declared layout.
        format: String,
    },
    /// An image's payload was shorter than its declared dimensions.
    TruncatedImage {
        /// Declared width.
        width: u32,
        /// Declared height.
        height: u32,
    },
}

impl Display for ModelError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gltf(error) => write!(formatter, "glTF import failed: {error}"),
            Self::NoScene => formatter.write_str("glTF declares no scene"),
            Self::MissingAttribute(name) => {
                write!(formatter, "primitive is missing its {name} attribute")
            }
            Self::UnsupportedTopology { mode } => {
                write!(formatter, "unsupported primitive topology {mode}")
            }
            Self::IndexCountNotTriangles(count) => {
                write!(formatter, "index count {count} is not a multiple of three")
            }
            Self::IndexOutOfRange { index, vertices } => write!(
                formatter,
                "index {index} is past the {vertices} vertices present"
            ),
            Self::LimitExceeded {
                what,
                actual,
                maximum,
            } => write!(formatter, "{what} {actual} exceeds maximum {maximum}"),
            Self::UnsupportedImageFormat { format } => {
                write!(formatter, "unsupported image pixel layout {format}")
            }
            Self::TruncatedImage { width, height } => write!(
                formatter,
                "an image's payload is shorter than the {width}x{height} it declares"
            ),
        }
    }
}

impl Error for ModelError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Gltf(error) => Some(error.as_ref()),
            _ => None,
        }
    }
}
