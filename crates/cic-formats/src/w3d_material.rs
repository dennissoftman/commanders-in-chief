//! W3D vertex-material, shader, texture, and texture-coordinate decoding.
//!
//! Provenance: this implementation was authored for Commanders in Chief from
//! `w3d_file.h`, `meshmdlio.cpp`, `vertmaterial.cpp`, `texture.cpp`, and `MAPPERS.TXT` at
//! `GeneralsGameCode` revision `9f7abb866f5afd446db14149979e744c7216baaf`. Those sources are
//! GPL-3.0-or-later with Electronic Arts Section 7 terms; no source code or retail
//! content is copied.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

use cic_core::{BinaryError, BinaryReader};

use crate::w3d::W3dChunk;
use crate::w3d_mesh::W3dMeshLimits;

const MATERIAL_INFO_CHUNK: u32 = 0x0000_0028;
const SHADERS_CHUNK: u32 = 0x0000_0029;
const VERTEX_MATERIALS_CHUNK: u32 = 0x0000_002A;
const VERTEX_MATERIAL_CHUNK: u32 = 0x0000_002B;
const VERTEX_MATERIAL_NAME_CHUNK: u32 = 0x0000_002C;
const VERTEX_MATERIAL_INFO_CHUNK: u32 = 0x0000_002D;
const VERTEX_MAPPER_ARGS0_CHUNK: u32 = 0x0000_002E;
const VERTEX_MAPPER_ARGS1_CHUNK: u32 = 0x0000_002F;
const TEXTURES_CHUNK: u32 = 0x0000_0030;
const TEXTURE_CHUNK: u32 = 0x0000_0031;
const TEXTURE_NAME_CHUNK: u32 = 0x0000_0032;
const TEXTURE_INFO_CHUNK: u32 = 0x0000_0033;
const MATERIAL_PASS_CHUNK: u32 = 0x0000_0038;
const VERTEX_MATERIAL_IDS_CHUNK: u32 = 0x0000_0039;
const SHADER_IDS_CHUNK: u32 = 0x0000_003A;
const DIFFUSE_COLORS_CHUNK: u32 = 0x0000_003B;
const DIFFUSE_ILLUMINATION_CHUNK: u32 = 0x0000_003C;
const SPECULAR_COLORS_CHUNK: u32 = 0x0000_003E;
const TEXTURE_STAGE_CHUNK: u32 = 0x0000_0048;
const TEXTURE_IDS_CHUNK: u32 = 0x0000_0049;
const STAGE_TEXCOORDS_CHUNK: u32 = 0x0000_004A;
const PER_FACE_TEXCOORD_IDS_CHUNK: u32 = 0x0000_004B;

const MATERIAL_INFO_BYTES: usize = 16;
const VERTEX_MATERIAL_INFO_BYTES: usize = 32;
const COLOR_BYTES: usize = 4;
const SHADER_BYTES: usize = 16;
const TEXTURE_INFO_BYTES: usize = 12;
const TEXCOORD_BYTES: usize = 8;

/// The four inventory counts in `W3dMaterialInfoStruct`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct W3dMaterialInfo {
    passes: u32,
    vertex_materials: u32,
    shaders: u32,
    textures: u32,
}

impl W3dMaterialInfo {
    /// Returns the declared material-pass count.
    #[must_use]
    pub const fn pass_count(self) -> u32 {
        self.passes
    }

    /// Returns the declared vertex-material count.
    #[must_use]
    pub const fn vertex_material_count(self) -> u32 {
        self.vertex_materials
    }

    /// Returns the declared shader count.
    #[must_use]
    pub const fn shader_count(self) -> u32 {
        self.shaders
    }

    /// Returns the declared texture count.
    #[must_use]
    pub const fn texture_count(self) -> u32 {
        self.textures
    }
}

/// One byte-per-channel RGB value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct W3dRgb8 {
    red: u8,
    green: u8,
    blue: u8,
}

impl W3dRgb8 {
    /// Returns the red channel.
    #[must_use]
    pub const fn red(self) -> u8 {
        self.red
    }

    /// Returns the green channel.
    #[must_use]
    pub const fn green(self) -> u8 {
        self.green
    }

    /// Returns the blue channel.
    #[must_use]
    pub const fn blue(self) -> u8 {
        self.blue
    }
}

/// One byte-per-channel RGBA value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct W3dRgba8 {
    red: u8,
    green: u8,
    blue: u8,
    alpha: u8,
}

impl W3dRgba8 {
    /// Returns the red channel.
    #[must_use]
    pub const fn red(self) -> u8 {
        self.red
    }

    /// Returns the green channel.
    #[must_use]
    pub const fn green(self) -> u8 {
        self.green
    }

    /// Returns the blue channel.
    #[must_use]
    pub const fn blue(self) -> u8 {
        self.blue
    }

    /// Returns the alpha channel.
    #[must_use]
    pub const fn alpha(self) -> u8 {
        self.alpha
    }
}

/// One fixed 32-byte `W3dVertexMaterialStruct` plus its optional raw name.
#[derive(Debug, Clone, PartialEq)]
pub struct W3dVertexMaterial {
    name: Option<Vec<u8>>,
    attributes: u32,
    ambient: W3dRgb8,
    diffuse: W3dRgb8,
    specular: W3dRgb8,
    emissive: W3dRgb8,
    shininess: f32,
    opacity: f32,
    translucency: f32,
    mappers: [W3dMapper; 2],
}

/// One of the two fixed-function texture-coordinate mapper selections encoded in vertex-material
/// attributes, plus its optional original argument string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct W3dMapper {
    mode: W3dMapperMode,
    arguments: Option<Vec<u8>>,
}

impl W3dMapper {
    #[must_use]
    pub const fn mode(&self) -> W3dMapperMode {
        self.mode
    }

    #[must_use]
    pub fn argument_bytes(&self) -> Option<&[u8]> {
        self.arguments.as_deref()
    }
}

/// Raw mapper selector with stable names for every mode defined by the pinned W3D header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct W3dMapperMode(u8);

impl W3dMapperMode {
    #[must_use]
    pub const fn code(self) -> u8 {
        self.0
    }

    #[must_use]
    pub const fn name(self) -> Option<&'static str> {
        match self.0 {
            0 => Some("uv"),
            1 => Some("environment"),
            2 => Some("cheap_environment"),
            3 => Some("screen"),
            4 => Some("linear_offset"),
            5 => Some("silhouette"),
            6 => Some("scale"),
            7 => Some("grid"),
            8 => Some("rotate"),
            9 => Some("sine_linear_offset"),
            10 => Some("step_linear_offset"),
            11 => Some("zigzag_linear_offset"),
            12 => Some("world_classic_environment"),
            13 => Some("world_environment"),
            14 => Some("grid_classic_environment"),
            15 => Some("grid_environment"),
            16 => Some("random"),
            17 => Some("edge"),
            18 => Some("bump_environment"),
            19 => Some("grid_world_classic_environment"),
            20 => Some("grid_world_environment"),
            _ => None,
        }
    }
}

impl W3dVertexMaterial {
    /// Returns the optional uninterpreted name bytes without the zero terminator.
    #[must_use]
    pub fn name_bytes(&self) -> Option<&[u8]> {
        self.name.as_deref()
    }

    /// Returns raw vertex-material attribute bits.
    #[must_use]
    pub const fn attributes(&self) -> u32 {
        self.attributes
    }

    /// Returns the ambient color.
    #[must_use]
    pub const fn ambient(&self) -> W3dRgb8 {
        self.ambient
    }

    /// Returns the diffuse color.
    #[must_use]
    pub const fn diffuse(&self) -> W3dRgb8 {
        self.diffuse
    }

    /// Returns the specular color.
    #[must_use]
    pub const fn specular(&self) -> W3dRgb8 {
        self.specular
    }

    /// Returns the emissive color.
    #[must_use]
    pub const fn emissive(&self) -> W3dRgb8 {
        self.emissive
    }

    /// Returns shininess.
    #[must_use]
    pub const fn shininess(&self) -> f32 {
        self.shininess
    }

    /// Returns opacity.
    #[must_use]
    pub const fn opacity(&self) -> f32 {
        self.opacity
    }

    /// Returns translucency.
    #[must_use]
    pub const fn translucency(&self) -> f32 {
        self.translucency
    }

    /// Returns mapper metadata for stage zero or one.
    #[must_use]
    pub fn mapper(&self, stage: usize) -> Option<&W3dMapper> {
        self.mappers.get(stage)
    }
}

/// A material-pass assignment encoded as one shared ID or one ID per vertex.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum W3dMaterialIds {
    /// One vertex material applies to every vertex.
    Single(u32),
    /// File-order vertex material IDs.
    PerVertex(Vec<u32>),
}

impl W3dMaterialIds {
    /// Returns the material ID for a zero-based vertex.
    #[must_use]
    pub fn for_vertex(&self, vertex: usize) -> Option<u32> {
        match self {
            Self::Single(id) => Some(*id),
            Self::PerVertex(ids) => ids.get(vertex).copied(),
        }
    }
}

/// One fixed 16-byte W3D fixed-function shader description.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct W3dShader {
    bytes: [u8; SHADER_BYTES],
}

impl W3dShader {
    /// Returns the exact on-disk shader bytes in field order.
    #[must_use]
    pub const fn bytes(self) -> [u8; SHADER_BYTES] {
        self.bytes
    }

    /// Returns whether the shader declares texturing enabled.
    #[must_use]
    pub const fn texturing(self) -> u8 {
        self.bytes[8]
    }

    /// Returns the source blend function selector.
    #[must_use]
    pub const fn source_blend(self) -> u8 {
        self.bytes[7]
    }

    /// Returns the destination blend function selector.
    #[must_use]
    pub const fn destination_blend(self) -> u8 {
        self.bytes[3]
    }

    /// Returns the alpha-test selector.
    #[must_use]
    pub const fn alpha_test(self) -> u8 {
        self.bytes[12]
    }
}

/// Optional animation and sampling flags for one texture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct W3dTextureInfo {
    attributes: u16,
    animation_type: W3dTextureAnimationType,
    frame_count: u32,
    frame_rate: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum W3dTextureAnimationType {
    Loop,
    PingPong,
    Once,
    Manual,
}

impl W3dTextureAnimationType {
    #[must_use]
    pub const fn code(self) -> u16 {
        match self {
            Self::Loop => 0,
            Self::PingPong => 1,
            Self::Once => 2,
            Self::Manual => 3,
        }
    }
}

impl W3dTextureInfo {
    #[must_use]
    pub const fn attributes(self) -> u16 {
        self.attributes
    }
    #[must_use]
    pub const fn animation_type(self) -> u16 {
        self.animation_type.code()
    }
    #[must_use]
    pub const fn animation(self) -> W3dTextureAnimationType {
        self.animation_type
    }
    #[must_use]
    pub const fn frame_count(self) -> u32 {
        self.frame_count
    }
    #[must_use]
    pub const fn frame_rate(self) -> f32 {
        self.frame_rate
    }
}

/// One texture table entry.
#[derive(Debug, Clone, PartialEq)]
pub struct W3dTexture {
    name: Vec<u8>,
    info: Option<W3dTextureInfo>,
}

impl W3dTexture {
    #[must_use]
    pub fn name_bytes(&self) -> &[u8] {
        &self.name
    }
    #[must_use]
    pub const fn info(&self) -> Option<W3dTextureInfo> {
        self.info
    }
}

/// A per-triangle assignment encoded as one shared ID or one ID per triangle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum W3dFaceIds {
    Single(u32),
    PerTriangle(Vec<u32>),
}

impl W3dFaceIds {
    #[must_use]
    pub fn for_triangle(&self, triangle: usize) -> Option<u32> {
        match self {
            Self::Single(id) => Some(*id),
            Self::PerTriangle(ids) => ids.get(triangle).copied(),
        }
    }
}

/// One W3D texture coordinate, before the runtime's V-axis inversion.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct W3dTexCoord {
    u: f32,
    v: f32,
}

impl W3dTexCoord {
    #[must_use]
    pub const fn u(self) -> f32 {
        self.u
    }
    #[must_use]
    pub const fn v(self) -> f32 {
        self.v
    }
}

/// Texture binding and UV data for one material-pass stage.
#[derive(Debug, Clone, PartialEq)]
pub struct W3dTextureStage {
    texture_ids: Option<W3dFaceIds>,
    texture_coordinates: Vec<W3dTexCoord>,
    per_face_coordinate_ids: Option<Vec<[u32; 3]>>,
}

impl W3dTextureStage {
    #[must_use]
    pub const fn texture_ids(&self) -> Option<&W3dFaceIds> {
        self.texture_ids.as_ref()
    }
    #[must_use]
    pub fn texture_coordinates(&self) -> &[W3dTexCoord] {
        &self.texture_coordinates
    }
    #[must_use]
    pub fn per_face_coordinate_ids(&self) -> Option<&[[u32; 3]]> {
        self.per_face_coordinate_ids.as_deref()
    }
    #[must_use]
    pub fn coordinate_indices(&self, triangle: usize, vertices: [u32; 3]) -> Option<[u32; 3]> {
        self.per_face_coordinate_ids
            .as_ref()
            .and_then(|ids| ids.get(triangle).copied())
            .or(Some(vertices))
    }
}

/// Decoded color-relevant values for one material pass.
#[derive(Debug, Clone, PartialEq)]
pub struct W3dMaterialPass {
    vertex_material_ids: Option<W3dMaterialIds>,
    shader_ids: Option<W3dFaceIds>,
    diffuse_colors: Option<Vec<W3dRgba8>>,
    diffuse_illumination: Option<Vec<W3dRgb8>>,
    specular_colors: Option<Vec<W3dRgb8>>,
    texture_stages: Vec<W3dTextureStage>,
}

impl W3dMaterialPass {
    /// Returns the shared or per-vertex material assignments when present.
    #[must_use]
    pub const fn vertex_material_ids(&self) -> Option<&W3dMaterialIds> {
        self.vertex_material_ids.as_ref()
    }

    /// Returns explicit per-vertex diffuse colors (`DCG`) when present.
    #[must_use]
    pub fn diffuse_colors(&self) -> Option<&[W3dRgba8]> {
        self.diffuse_colors.as_deref()
    }

    #[must_use]
    pub fn diffuse_illumination(&self) -> Option<&[W3dRgb8]> {
        self.diffuse_illumination.as_deref()
    }

    #[must_use]
    pub fn specular_colors(&self) -> Option<&[W3dRgb8]> {
        self.specular_colors.as_deref()
    }

    #[must_use]
    pub const fn shader_ids(&self) -> Option<&W3dFaceIds> {
        self.shader_ids.as_ref()
    }
    #[must_use]
    pub fn texture_stages(&self) -> &[W3dTextureStage] {
        &self.texture_stages
    }
}

/// Material inventory, vertex materials, and color-relevant pass assignments.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct W3dMaterialSet {
    info: Option<W3dMaterialInfo>,
    vertex_materials: Vec<W3dVertexMaterial>,
    shaders: Vec<W3dShader>,
    textures: Vec<W3dTexture>,
    passes: Vec<W3dMaterialPass>,
}

impl W3dMaterialSet {
    /// Returns material inventory metadata, or `None` for a geometry-only mesh.
    #[must_use]
    pub const fn info(&self) -> Option<W3dMaterialInfo> {
        self.info
    }

    /// Returns vertex materials in file order.
    #[must_use]
    pub fn vertex_materials(&self) -> &[W3dVertexMaterial] {
        &self.vertex_materials
    }

    /// Returns material passes in file order.
    #[must_use]
    pub fn passes(&self) -> &[W3dMaterialPass] {
        &self.passes
    }

    #[must_use]
    pub fn shaders(&self) -> &[W3dShader] {
        &self.shaders
    }
    #[must_use]
    pub fn textures(&self) -> &[W3dTexture] {
        &self.textures
    }

    /// Resolves first-pass preview colors for all vertices.
    #[must_use]
    pub fn preview_vertex_colors(&self, vertex_count: usize) -> Option<Vec<W3dRgba8>> {
        let pass = self.passes.first()?;
        if let Some(colors) = &pass.diffuse_colors {
            return Some(colors.clone());
        }
        let ids = pass.vertex_material_ids.as_ref()?;
        let mut colors = Vec::with_capacity(vertex_count);
        for vertex in 0..vertex_count {
            let id = usize::try_from(ids.for_vertex(vertex)?).ok()?;
            let material = self.vertex_materials.get(id)?;
            let diffuse = material.diffuse;
            colors.push(W3dRgba8 {
                red: diffuse.red,
                green: diffuse.green,
                blue: diffuse.blue,
                alpha: u8::MAX,
            });
        }
        Some(colors)
    }
}

/// A structured W3D material-color decoding failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum W3dMaterialError {
    /// A bounded read or configured count/name limit failed.
    Binary(BinaryError),
    /// Material children existed without their required inventory chunk.
    MissingMaterialInfo,
    /// A required nested material chunk was absent.
    MissingChunk {
        /// Missing numeric chunk identifier.
        id: u32,
    },
    /// A semantic child appeared more than once in one parent.
    DuplicateChunk {
        /// Duplicated chunk identifier.
        id: u32,
    },
    /// A required container was encoded as data.
    ChunkMustBeContainer {
        /// Numeric chunk identifier.
        id: u32,
    },
    /// A required data leaf was encoded as a container.
    ChunkMustBeData {
        /// Numeric chunk identifier.
        id: u32,
    },
    /// A fixed-size payload had the wrong length.
    InvalidChunkLength {
        /// Numeric chunk identifier.
        id: u32,
        /// Actual byte length.
        actual: usize,
        /// Required byte length.
        expected: usize,
    },
    /// An ID array was neither singleton nor per-vertex sized.
    InvalidIdArrayLength {
        /// Actual byte length.
        actual: usize,
        /// Required per-vertex byte length.
        per_vertex: usize,
    },
    /// A singleton-or-per-triangle assignment had another length.
    InvalidFaceIdArrayLength {
        /// Assignment kind.
        what: &'static str,
        /// Actual byte length.
        actual: usize,
        /// Required per-triangle byte length.
        per_triangle: usize,
    },
    /// Parsed children disagreed with material inventory metadata.
    CountMismatch {
        /// Counted resource kind.
        what: &'static str,
        /// Header declaration.
        declared: usize,
        /// Actual decoded count.
        actual: usize,
    },
    /// A material ID was outside the vertex-material table.
    MaterialIndexOutOfRange {
        /// Zero-based pass index.
        pass: usize,
        /// Zero-based vertex, or `None` for a singleton assignment.
        vertex: Option<usize>,
        /// Referenced material ID.
        index: u32,
        /// Available material count.
        material_count: usize,
    },
    /// A material name did not contain a zero terminator.
    UnterminatedName {
        /// Zero-based vertex-material index.
        material: usize,
    },
    /// A texture name did not contain a zero terminator.
    UnterminatedTextureName {
        texture: usize,
    },
    /// A vertex-material mapper argument string did not contain a zero terminator.
    UnterminatedMapperArguments {
        material: usize,
        stage: usize,
    },
    /// A shader, texture, or UV index was outside its decoded table.
    ResourceIndexOutOfRange {
        what: &'static str,
        pass: usize,
        element: Option<usize>,
        index: u32,
        count: usize,
    },
    /// A texture scalar or coordinate was NaN or infinite.
    NonFiniteTextureValue {
        what: &'static str,
        index: usize,
    },
    InvalidTextureAnimationType {
        texture: usize,
        animation_type: u16,
    },
    ZeroTextureFrameCount {
        texture: usize,
    },
    ZeroAnimatedTextureFrameRate {
        texture: usize,
    },
    /// A material scalar was NaN or infinite.
    NonFiniteScalar {
        /// Zero-based vertex-material index.
        material: usize,
        /// Scalar field name.
        field: &'static str,
    },
    /// Count-to-payload-size multiplication overflowed.
    SizeOverflow {
        /// Record array being sized.
        what: &'static str,
    },
}

impl Display for W3dMaterialError {
    #[allow(clippy::too_many_lines)]
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binary(error) => Display::fmt(error, formatter),
            Self::MissingMaterialInfo => {
                formatter.write_str("W3D material children require material info")
            }
            Self::MissingChunk { id } => {
                write!(formatter, "W3D material is missing chunk 0x{id:08X}")
            }
            Self::DuplicateChunk { id } => {
                write!(formatter, "W3D material chunk 0x{id:08X} is duplicated")
            }
            Self::ChunkMustBeContainer { id } => {
                write!(
                    formatter,
                    "W3D material chunk 0x{id:08X} must be a container"
                )
            }
            Self::ChunkMustBeData { id } => {
                write!(
                    formatter,
                    "W3D material chunk 0x{id:08X} must be a data leaf"
                )
            }
            Self::InvalidChunkLength {
                id,
                actual,
                expected,
            } => write!(
                formatter,
                "W3D material chunk 0x{id:08X} has {actual} bytes; expected {expected}"
            ),
            Self::InvalidIdArrayLength { actual, per_vertex } => write!(
                formatter,
                "W3D vertex-material IDs have {actual} bytes; expected 4 or {per_vertex}"
            ),
            Self::InvalidFaceIdArrayLength {
                what,
                actual,
                per_triangle,
            } => write!(
                formatter,
                "W3D {what} IDs have {actual} bytes; expected 4 or {per_triangle}"
            ),
            Self::CountMismatch {
                what,
                declared,
                actual,
            } => write!(
                formatter,
                "W3D material info declares {declared} {what}, but {actual} were decoded"
            ),
            Self::MaterialIndexOutOfRange {
                pass,
                vertex,
                index,
                material_count,
            } => write!(
                formatter,
                "W3D material pass {pass}{} references material {index}, but only {material_count} exist",
                vertex.map_or_else(String::new, |vertex| format!(" vertex {vertex}"))
            ),
            Self::UnterminatedName { material } => {
                write!(
                    formatter,
                    "W3D vertex material {material} name is not terminated"
                )
            }
            Self::UnterminatedTextureName { texture } => {
                write!(formatter, "W3D texture {texture} name is not terminated")
            }
            Self::UnterminatedMapperArguments { material, stage } => write!(
                formatter,
                "W3D vertex material {material} mapper stage {stage} arguments are not terminated"
            ),
            Self::ResourceIndexOutOfRange {
                what,
                pass,
                element,
                index,
                count,
            } => write!(
                formatter,
                "W3D material pass {pass}{} references {what} {index}, but only {count} exist",
                element.map_or_else(String::new, |value| format!(" element {value}"))
            ),
            Self::NonFiniteTextureValue { what, index } => {
                write!(formatter, "W3D {what} {index} is non-finite")
            }
            Self::InvalidTextureAnimationType {
                texture,
                animation_type,
            } => write!(
                formatter,
                "W3D texture {texture} has unsupported animation type {animation_type}"
            ),
            Self::ZeroTextureFrameCount { texture } => {
                write!(formatter, "W3D texture {texture} has zero animation frames")
            }
            Self::ZeroAnimatedTextureFrameRate { texture } => write!(
                formatter,
                "W3D animated texture {texture} has a zero frame rate"
            ),
            Self::NonFiniteScalar { material, field } => write!(
                formatter,
                "W3D vertex material {material} has a non-finite {field}"
            ),
            Self::SizeOverflow { what } => write!(formatter, "W3D {what} size overflowed"),
        }
    }
}

impl Error for W3dMaterialError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Binary(error) => Some(error),
            _ => None,
        }
    }
}

impl From<BinaryError> for W3dMaterialError {
    fn from(error: BinaryError) -> Self {
        Self::Binary(error)
    }
}

pub(crate) fn decode_materials(
    children: &[W3dChunk],
    vertex_count: usize,
    triangle_count: usize,
    limits: W3dMeshLimits,
) -> Result<W3dMaterialSet, W3dMaterialError> {
    let has_material_children = children.iter().any(|child| {
        matches!(
            child.id(),
            MATERIAL_INFO_CHUNK
                | SHADERS_CHUNK
                | VERTEX_MATERIALS_CHUNK
                | TEXTURES_CHUNK
                | MATERIAL_PASS_CHUNK
        )
    });
    if !has_material_children {
        return Ok(W3dMaterialSet::default());
    }

    let info_chunk = unique_child(children, MATERIAL_INFO_CHUNK)?
        .ok_or(W3dMaterialError::MissingMaterialInfo)?;
    let info_bytes = require_data(info_chunk)?;
    require_length(MATERIAL_INFO_CHUNK, info_bytes.len(), MATERIAL_INFO_BYTES)?;
    let info = parse_material_info(info_bytes, limits)?;
    let expected_materials = limited_count(
        info.vertex_materials,
        "W3D vertex material count",
        limits.maximum_vertex_materials,
    )?;
    let expected_passes = limited_count(
        info.passes,
        "W3D material pass count",
        limits.maximum_material_passes,
    )?;

    let vertex_materials = match unique_child(children, VERTEX_MATERIALS_CHUNK)? {
        Some(wrapper) => decode_vertex_materials(wrapper, limits)?,
        None => Vec::new(),
    };
    require_count(
        "vertex materials",
        expected_materials,
        vertex_materials.len(),
    )?;

    let shaders = unique_child(children, SHADERS_CHUNK)?
        .map(|chunk| decode_shaders(chunk, limits.maximum_shaders))
        .transpose()?
        .unwrap_or_default();
    require_count(
        "shaders",
        usize::try_from(info.shaders).unwrap_or(usize::MAX),
        shaders.len(),
    )?;

    let textures = unique_child(children, TEXTURES_CHUNK)?
        .map(|chunk| decode_textures(chunk, limits))
        .transpose()?
        .unwrap_or_default();
    require_count(
        "textures",
        usize::try_from(info.textures).unwrap_or(usize::MAX),
        textures.len(),
    )?;

    let mut passes = Vec::new();
    for child in children
        .iter()
        .filter(|child| child.id() == MATERIAL_PASS_CHUNK)
    {
        if passes.len() >= limits.maximum_material_passes {
            return Err(W3dMaterialError::Binary(BinaryError::LimitExceeded {
                what: "W3D material pass count",
                actual: passes.len().saturating_add(1),
                maximum: limits.maximum_material_passes,
            }));
        }
        passes.push(decode_pass(
            child,
            passes.len(),
            PassDecodeContext {
                vertex_count,
                triangle_count,
                material_count: vertex_materials.len(),
                shader_count: shaders.len(),
                texture_count: textures.len(),
                limits,
            },
        )?);
    }
    require_count("material passes", expected_passes, passes.len())?;

    Ok(W3dMaterialSet {
        info: Some(info),
        vertex_materials,
        shaders,
        textures,
        passes,
    })
}

fn parse_material_info(
    bytes: &[u8],
    limits: W3dMeshLimits,
) -> Result<W3dMaterialInfo, W3dMaterialError> {
    let mut reader = BinaryReader::new(bytes, "W3D material info");
    let info = W3dMaterialInfo {
        passes: reader.read_u32_le()?,
        vertex_materials: reader.read_u32_le()?,
        shaders: reader.read_u32_le()?,
        textures: reader.read_u32_le()?,
    };
    limited_count(info.shaders, "W3D shader count", limits.maximum_shaders)?;
    limited_count(info.textures, "W3D texture count", limits.maximum_textures)?;
    Ok(info)
}

fn decode_vertex_materials(
    wrapper: &W3dChunk,
    limits: W3dMeshLimits,
) -> Result<Vec<W3dVertexMaterial>, W3dMaterialError> {
    let children = wrapper
        .children()
        .ok_or(W3dMaterialError::ChunkMustBeContainer {
            id: VERTEX_MATERIALS_CHUNK,
        })?;
    let mut materials = Vec::new();
    for child in children
        .iter()
        .filter(|child| child.id() == VERTEX_MATERIAL_CHUNK)
    {
        if materials.len() >= limits.maximum_vertex_materials {
            return Err(W3dMaterialError::Binary(BinaryError::LimitExceeded {
                what: "W3D vertex material count",
                actual: materials.len().saturating_add(1),
                maximum: limits.maximum_vertex_materials,
            }));
        }
        materials.push(decode_vertex_material(child, materials.len(), limits)?);
    }
    Ok(materials)
}

fn decode_vertex_material(
    chunk: &W3dChunk,
    material_index: usize,
    limits: W3dMeshLimits,
) -> Result<W3dVertexMaterial, W3dMaterialError> {
    let children = chunk
        .children()
        .ok_or(W3dMaterialError::ChunkMustBeContainer {
            id: VERTEX_MATERIAL_CHUNK,
        })?;
    let info_chunk = unique_child(children, VERTEX_MATERIAL_INFO_CHUNK)?.ok_or(
        W3dMaterialError::MissingChunk {
            id: VERTEX_MATERIAL_INFO_CHUNK,
        },
    )?;
    let bytes = require_data(info_chunk)?;
    require_length(
        VERTEX_MATERIAL_INFO_CHUNK,
        bytes.len(),
        VERTEX_MATERIAL_INFO_BYTES,
    )?;

    let name = unique_child(children, VERTEX_MATERIAL_NAME_CHUNK)?
        .map(|chunk| parse_name(chunk, material_index, limits.maximum_material_name_bytes))
        .transpose()?;
    let mapper_arguments0 = unique_child(children, VERTEX_MAPPER_ARGS0_CHUNK)?
        .map(|chunk| {
            parse_mapper_arguments(
                chunk,
                material_index,
                0,
                limits.maximum_mapper_argument_bytes,
            )
        })
        .transpose()?;
    let mapper_arguments1 = unique_child(children, VERTEX_MAPPER_ARGS1_CHUNK)?
        .map(|chunk| {
            parse_mapper_arguments(
                chunk,
                material_index,
                1,
                limits.maximum_mapper_argument_bytes,
            )
        })
        .transpose()?;
    parse_vertex_material(
        bytes,
        material_index,
        name,
        [mapper_arguments0, mapper_arguments1],
    )
}

fn parse_name(
    chunk: &W3dChunk,
    material_index: usize,
    maximum: usize,
) -> Result<Vec<u8>, W3dMaterialError> {
    let bytes = require_data(chunk)?;
    let maximum_with_terminator = maximum.saturating_add(1);
    if bytes.len() > maximum_with_terminator {
        return Err(W3dMaterialError::Binary(BinaryError::LimitExceeded {
            what: "W3D vertex material name bytes",
            actual: bytes.len(),
            maximum: maximum_with_terminator,
        }));
    }
    let length =
        bytes
            .iter()
            .position(|byte| *byte == 0)
            .ok_or(W3dMaterialError::UnterminatedName {
                material: material_index,
            })?;
    Ok(bytes[..length].to_vec())
}

fn parse_vertex_material(
    bytes: &[u8],
    material_index: usize,
    name: Option<Vec<u8>>,
    mapper_arguments: [Option<Vec<u8>>; 2],
) -> Result<W3dVertexMaterial, W3dMaterialError> {
    let mut reader = BinaryReader::new(bytes, "W3D vertex material info");
    let attributes = reader.read_u32_le()?;
    let ambient = read_rgb(&mut reader)?;
    let diffuse = read_rgb(&mut reader)?;
    let specular = read_rgb(&mut reader)?;
    let emissive = read_rgb(&mut reader)?;
    let shininess = read_finite(&mut reader, material_index, "shininess")?;
    let opacity = read_finite(&mut reader, material_index, "opacity")?;
    let translucency = read_finite(&mut reader, material_index, "translucency")?;
    let [arguments0, arguments1] = mapper_arguments;
    let mappers = [
        W3dMapper {
            mode: W3dMapperMode(
                u8::try_from((attributes & 0x00FF_0000) >> 16)
                    .expect("stage zero mapper bits fit u8"),
            ),
            arguments: arguments0,
        },
        W3dMapper {
            mode: W3dMapperMode(
                u8::try_from((attributes & 0x0000_FF00) >> 8)
                    .expect("stage one mapper bits fit u8"),
            ),
            arguments: arguments1,
        },
    ];
    Ok(W3dVertexMaterial {
        name,
        attributes,
        ambient,
        diffuse,
        specular,
        emissive,
        shininess,
        opacity,
        translucency,
        mappers,
    })
}

fn parse_mapper_arguments(
    chunk: &W3dChunk,
    material: usize,
    stage: usize,
    maximum: usize,
) -> Result<Vec<u8>, W3dMaterialError> {
    let bytes = require_data(chunk)?;
    let maximum_with_terminator = maximum.saturating_add(1);
    if bytes.len() > maximum_with_terminator {
        return Err(W3dMaterialError::Binary(BinaryError::LimitExceeded {
            what: "W3D mapper argument bytes",
            actual: bytes.len(),
            maximum: maximum_with_terminator,
        }));
    }
    let length = bytes
        .iter()
        .position(|byte| *byte == 0)
        .ok_or(W3dMaterialError::UnterminatedMapperArguments { material, stage })?;
    Ok(bytes[..length].to_vec())
}

fn decode_shaders(chunk: &W3dChunk, maximum: usize) -> Result<Vec<W3dShader>, W3dMaterialError> {
    let bytes = require_data(chunk)?;
    if !bytes.len().is_multiple_of(SHADER_BYTES) {
        return Err(W3dMaterialError::InvalidChunkLength {
            id: SHADERS_CHUNK,
            actual: bytes.len(),
            expected: bytes.len() / SHADER_BYTES * SHADER_BYTES,
        });
    }
    let count = bytes.len() / SHADER_BYTES;
    if count > maximum {
        return Err(BinaryError::LimitExceeded {
            what: "W3D shader count",
            actual: count,
            maximum,
        }
        .into());
    }
    Ok(bytes
        .chunks_exact(SHADER_BYTES)
        .map(|record| {
            let mut bytes = [0; SHADER_BYTES];
            bytes.copy_from_slice(record);
            W3dShader { bytes }
        })
        .collect())
}

fn decode_textures(
    wrapper: &W3dChunk,
    limits: W3dMeshLimits,
) -> Result<Vec<W3dTexture>, W3dMaterialError> {
    let children = wrapper
        .children()
        .ok_or(W3dMaterialError::ChunkMustBeContainer { id: TEXTURES_CHUNK })?;
    let mut textures = Vec::new();
    for child in children.iter().filter(|child| child.id() == TEXTURE_CHUNK) {
        if textures.len() >= limits.maximum_textures {
            return Err(BinaryError::LimitExceeded {
                what: "W3D texture count",
                actual: textures.len().saturating_add(1),
                maximum: limits.maximum_textures,
            }
            .into());
        }
        textures.push(decode_texture(child, textures.len(), limits)?);
    }
    Ok(textures)
}

fn decode_texture(
    chunk: &W3dChunk,
    texture_index: usize,
    limits: W3dMeshLimits,
) -> Result<W3dTexture, W3dMaterialError> {
    let children = chunk
        .children()
        .ok_or(W3dMaterialError::ChunkMustBeContainer { id: TEXTURE_CHUNK })?;
    let name_chunk =
        unique_child(children, TEXTURE_NAME_CHUNK)?.ok_or(W3dMaterialError::MissingChunk {
            id: TEXTURE_NAME_CHUNK,
        })?;
    let name_bytes = require_data(name_chunk)?;
    let maximum = limits.maximum_texture_name_bytes.saturating_add(1);
    if name_bytes.len() > maximum {
        return Err(BinaryError::LimitExceeded {
            what: "W3D texture name bytes",
            actual: name_bytes.len(),
            maximum,
        }
        .into());
    }
    let length = name_bytes.iter().position(|byte| *byte == 0).ok_or(
        W3dMaterialError::UnterminatedTextureName {
            texture: texture_index,
        },
    )?;
    let info = unique_child(children, TEXTURE_INFO_CHUNK)?
        .map(|chunk| {
            let bytes = require_data(chunk)?;
            require_length(TEXTURE_INFO_CHUNK, bytes.len(), TEXTURE_INFO_BYTES)?;
            let mut reader = BinaryReader::new(bytes, "W3D texture info");
            let attributes = reader.read_u16_le()?;
            let raw_animation_type = reader.read_u16_le()?;
            let animation_type = match raw_animation_type {
                0 => W3dTextureAnimationType::Loop,
                1 => W3dTextureAnimationType::PingPong,
                2 => W3dTextureAnimationType::Once,
                3 => W3dTextureAnimationType::Manual,
                _ => {
                    return Err(W3dMaterialError::InvalidTextureAnimationType {
                        texture: texture_index,
                        animation_type: raw_animation_type,
                    });
                }
            };
            let frame_count = reader.read_u32_le()?;
            if frame_count == 0 {
                return Err(W3dMaterialError::ZeroTextureFrameCount {
                    texture: texture_index,
                });
            }
            limited_count(
                frame_count,
                "W3D texture animation frame count",
                limits.maximum_texture_animation_frames,
            )?;
            let frame_rate = f32::from_bits(reader.read_u32_le()?);
            if !frame_rate.is_finite() {
                return Err(W3dMaterialError::NonFiniteTextureValue {
                    what: "texture frame rate",
                    index: texture_index,
                });
            }
            if frame_count > 1 && frame_rate <= 0.0 {
                return Err(W3dMaterialError::ZeroAnimatedTextureFrameRate {
                    texture: texture_index,
                });
            }
            Ok(W3dTextureInfo {
                attributes,
                animation_type,
                frame_count,
                frame_rate,
            })
        })
        .transpose()?;
    Ok(W3dTexture {
        name: name_bytes[..length].to_vec(),
        info,
    })
}

#[derive(Clone, Copy)]
struct PassDecodeContext {
    vertex_count: usize,
    triangle_count: usize,
    material_count: usize,
    shader_count: usize,
    texture_count: usize,
    limits: W3dMeshLimits,
}

fn decode_pass(
    chunk: &W3dChunk,
    pass_index: usize,
    context: PassDecodeContext,
) -> Result<W3dMaterialPass, W3dMaterialError> {
    let children = chunk
        .children()
        .ok_or(W3dMaterialError::ChunkMustBeContainer {
            id: MATERIAL_PASS_CHUNK,
        })?;
    let vertex_material_ids = unique_child(children, VERTEX_MATERIAL_IDS_CHUNK)?
        .map(|chunk| {
            parse_material_ids(
                require_data(chunk)?,
                pass_index,
                context.vertex_count,
                context.material_count,
            )
        })
        .transpose()?;
    let diffuse_colors = unique_child(children, DIFFUSE_COLORS_CHUNK)?
        .map(|chunk| parse_diffuse_colors(require_data(chunk)?, context.vertex_count))
        .transpose()?;
    let diffuse_illumination = unique_child(children, DIFFUSE_ILLUMINATION_CHUNK)?
        .map(|chunk| {
            parse_rgb_colors(
                require_data(chunk)?,
                DIFFUSE_ILLUMINATION_CHUNK,
                context.vertex_count,
                "diffuse illumination array",
            )
        })
        .transpose()?;
    let specular_colors = unique_child(children, SPECULAR_COLORS_CHUNK)?
        .map(|chunk| {
            parse_rgb_colors(
                require_data(chunk)?,
                SPECULAR_COLORS_CHUNK,
                context.vertex_count,
                "specular color array",
            )
        })
        .transpose()?;
    let shader_ids = unique_child(children, SHADER_IDS_CHUNK)?
        .map(|chunk| {
            parse_face_ids(
                require_data(chunk)?,
                "shader",
                pass_index,
                context.triangle_count,
                context.shader_count,
                false,
            )
        })
        .transpose()?;
    let mut texture_stages = Vec::new();
    for child in children
        .iter()
        .filter(|child| child.id() == TEXTURE_STAGE_CHUNK)
    {
        if texture_stages.len() >= context.limits.maximum_texture_stages_per_pass {
            return Err(BinaryError::LimitExceeded {
                what: "W3D texture stage count per pass",
                actual: texture_stages.len().saturating_add(1),
                maximum: context.limits.maximum_texture_stages_per_pass,
            }
            .into());
        }
        texture_stages.push(decode_texture_stage(
            child,
            pass_index,
            context.vertex_count,
            context.triangle_count,
            context.texture_count,
            context.limits,
        )?);
    }
    Ok(W3dMaterialPass {
        vertex_material_ids,
        shader_ids,
        diffuse_colors,
        diffuse_illumination,
        specular_colors,
        texture_stages,
    })
}

fn parse_face_ids(
    bytes: &[u8],
    what: &'static str,
    pass: usize,
    triangle_count: usize,
    table_count: usize,
    allow_none: bool,
) -> Result<W3dFaceIds, W3dMaterialError> {
    let per_triangle = payload_size(triangle_count, 4, "per-triangle ID array")?;
    if bytes.len() != 4 && bytes.len() != per_triangle {
        return Err(W3dMaterialError::InvalidFaceIdArrayLength {
            what,
            actual: bytes.len(),
            per_triangle,
        });
    }
    let mut reader = BinaryReader::new(bytes, "W3D per-face IDs");
    let count = if bytes.len() == 4 { 1 } else { triangle_count };
    let mut ids = Vec::with_capacity(count);
    for element in 0..count {
        let id = reader.read_u32_le()?;
        if !(usize::try_from(id).is_ok_and(|id| id < table_count) || allow_none && id == u32::MAX) {
            return Err(W3dMaterialError::ResourceIndexOutOfRange {
                what,
                pass,
                element: (count != 1).then_some(element),
                index: id,
                count: table_count,
            });
        }
        ids.push(id);
    }
    if count == 1 {
        Ok(W3dFaceIds::Single(*ids.first().ok_or(
            W3dMaterialError::SizeOverflow {
                what: "singleton ID",
            },
        )?))
    } else {
        Ok(W3dFaceIds::PerTriangle(ids))
    }
}

fn decode_texture_stage(
    chunk: &W3dChunk,
    pass: usize,
    vertex_count: usize,
    triangle_count: usize,
    texture_count: usize,
    limits: W3dMeshLimits,
) -> Result<W3dTextureStage, W3dMaterialError> {
    let children = chunk
        .children()
        .ok_or(W3dMaterialError::ChunkMustBeContainer {
            id: TEXTURE_STAGE_CHUNK,
        })?;
    let texture_ids = unique_child(children, TEXTURE_IDS_CHUNK)?
        .map(|chunk| {
            parse_face_ids(
                require_data(chunk)?,
                "texture",
                pass,
                triangle_count,
                texture_count,
                true,
            )
        })
        .transpose()?;
    let texture_coordinates = unique_child(children, STAGE_TEXCOORDS_CHUNK)?
        .map(|chunk| parse_texcoords(require_data(chunk)?, limits.maximum_texture_coordinates))
        .transpose()?
        .unwrap_or_default();
    let per_face_coordinate_ids = unique_child(children, PER_FACE_TEXCOORD_IDS_CHUNK)?
        .map(|chunk| {
            parse_uv_indices(
                require_data(chunk)?,
                pass,
                triangle_count,
                texture_coordinates.len(),
            )
        })
        .transpose()?;
    if per_face_coordinate_ids.is_none()
        && !texture_coordinates.is_empty()
        && texture_coordinates.len() != vertex_count
    {
        return Err(W3dMaterialError::InvalidChunkLength {
            id: STAGE_TEXCOORDS_CHUNK,
            actual: texture_coordinates.len().saturating_mul(TEXCOORD_BYTES),
            expected: vertex_count.saturating_mul(TEXCOORD_BYTES),
        });
    }
    Ok(W3dTextureStage {
        texture_ids,
        texture_coordinates,
        per_face_coordinate_ids,
    })
}

fn parse_texcoords(bytes: &[u8], maximum: usize) -> Result<Vec<W3dTexCoord>, W3dMaterialError> {
    if !bytes.len().is_multiple_of(TEXCOORD_BYTES) {
        return Err(W3dMaterialError::InvalidChunkLength {
            id: STAGE_TEXCOORDS_CHUNK,
            actual: bytes.len(),
            expected: bytes.len() / TEXCOORD_BYTES * TEXCOORD_BYTES,
        });
    }
    let count = bytes.len() / TEXCOORD_BYTES;
    if count > maximum {
        return Err(BinaryError::LimitExceeded {
            what: "W3D texture coordinate count",
            actual: count,
            maximum,
        }
        .into());
    }
    let mut reader = BinaryReader::new(bytes, "W3D texture coordinates");
    let mut coordinates = Vec::with_capacity(count);
    for _index in 0..count {
        let u = f32::from_bits(reader.read_u32_le()?);
        let v = f32::from_bits(reader.read_u32_le()?);
        // Texture coordinates are immutable source data. Some shipped meshes contain non-finite
        // payloads; presentation/export boundaries replace those values deterministically while
        // the parser preserves their exact bits for diagnostics and metadata.
        coordinates.push(W3dTexCoord { u, v });
    }
    Ok(coordinates)
}

fn parse_uv_indices(
    bytes: &[u8],
    pass: usize,
    triangles: usize,
    coordinate_count: usize,
) -> Result<Vec<[u32; 3]>, W3dMaterialError> {
    let expected = payload_size(triangles, 12, "per-face texture-coordinate ID array")?;
    require_length(PER_FACE_TEXCOORD_IDS_CHUNK, bytes.len(), expected)?;
    let mut reader = BinaryReader::new(bytes, "W3D texture-coordinate IDs");
    let mut result = Vec::with_capacity(triangles);
    for triangle in 0..triangles {
        let ids = [
            reader.read_u32_le()?,
            reader.read_u32_le()?,
            reader.read_u32_le()?,
        ];
        for id in ids {
            if !usize::try_from(id).is_ok_and(|id| id < coordinate_count) {
                return Err(W3dMaterialError::ResourceIndexOutOfRange {
                    what: "texture coordinate",
                    pass,
                    element: Some(triangle),
                    index: id,
                    count: coordinate_count,
                });
            }
        }
        result.push(ids);
    }
    Ok(result)
}

fn parse_material_ids(
    bytes: &[u8],
    pass_index: usize,
    vertex_count: usize,
    material_count: usize,
) -> Result<W3dMaterialIds, W3dMaterialError> {
    let per_vertex_bytes = payload_size(vertex_count, 4, "vertex-material ID array")?;
    let mut reader = BinaryReader::new(bytes, "W3D vertex-material IDs");
    if bytes.len() == 4 {
        let id = reader.read_u32_le()?;
        validate_material_id(id, pass_index, None, material_count)?;
        return Ok(W3dMaterialIds::Single(id));
    }
    if bytes.len() != per_vertex_bytes {
        return Err(W3dMaterialError::InvalidIdArrayLength {
            actual: bytes.len(),
            per_vertex: per_vertex_bytes,
        });
    }
    let mut ids = Vec::with_capacity(vertex_count);
    for vertex in 0..vertex_count {
        let id = reader.read_u32_le()?;
        validate_material_id(id, pass_index, Some(vertex), material_count)?;
        ids.push(id);
    }
    Ok(W3dMaterialIds::PerVertex(ids))
}

fn parse_diffuse_colors(
    bytes: &[u8],
    vertex_count: usize,
) -> Result<Vec<W3dRgba8>, W3dMaterialError> {
    let expected = payload_size(vertex_count, COLOR_BYTES, "diffuse color array")?;
    require_length(DIFFUSE_COLORS_CHUNK, bytes.len(), expected)?;
    let mut reader = BinaryReader::new(bytes, "W3D diffuse colors");
    let mut colors = Vec::with_capacity(vertex_count);
    for _ in 0..vertex_count {
        colors.push(W3dRgba8 {
            red: reader.read_u8()?,
            green: reader.read_u8()?,
            blue: reader.read_u8()?,
            alpha: reader.read_u8()?,
        });
    }
    Ok(colors)
}

fn parse_rgb_colors(
    bytes: &[u8],
    chunk_id: u32,
    vertex_count: usize,
    what: &'static str,
) -> Result<Vec<W3dRgb8>, W3dMaterialError> {
    let expected = payload_size(vertex_count, COLOR_BYTES, what)?;
    require_length(chunk_id, bytes.len(), expected)?;
    let mut reader = BinaryReader::new(bytes, "W3D RGB color array");
    let mut colors = Vec::with_capacity(vertex_count);
    for _ in 0..vertex_count {
        colors.push(read_rgb(&mut reader)?);
    }
    Ok(colors)
}

fn unique_child(children: &[W3dChunk], id: u32) -> Result<Option<&W3dChunk>, W3dMaterialError> {
    let mut matching = children.iter().filter(|child| child.id() == id);
    let first = matching.next();
    if matching.next().is_some() {
        Err(W3dMaterialError::DuplicateChunk { id })
    } else {
        Ok(first)
    }
}

fn require_data(chunk: &W3dChunk) -> Result<&[u8], W3dMaterialError> {
    chunk
        .data()
        .ok_or(W3dMaterialError::ChunkMustBeData { id: chunk.id() })
}

fn require_length(id: u32, actual: usize, expected: usize) -> Result<(), W3dMaterialError> {
    if actual == expected {
        Ok(())
    } else {
        Err(W3dMaterialError::InvalidChunkLength {
            id,
            actual,
            expected,
        })
    }
}

fn require_count(
    what: &'static str,
    declared: usize,
    actual: usize,
) -> Result<(), W3dMaterialError> {
    if declared == actual {
        Ok(())
    } else {
        Err(W3dMaterialError::CountMismatch {
            what,
            declared,
            actual,
        })
    }
}

fn validate_material_id(
    index: u32,
    pass: usize,
    vertex: Option<usize>,
    material_count: usize,
) -> Result<(), W3dMaterialError> {
    if usize::try_from(index).is_ok_and(|index| index < material_count) {
        Ok(())
    } else {
        Err(W3dMaterialError::MaterialIndexOutOfRange {
            pass,
            vertex,
            index,
            material_count,
        })
    }
}

fn read_rgb(reader: &mut BinaryReader<'_>) -> Result<W3dRgb8, BinaryError> {
    let color = W3dRgb8 {
        red: reader.read_u8()?,
        green: reader.read_u8()?,
        blue: reader.read_u8()?,
    };
    reader.skip(1)?;
    Ok(color)
}

fn read_finite(
    reader: &mut BinaryReader<'_>,
    material: usize,
    field: &'static str,
) -> Result<f32, W3dMaterialError> {
    let value = f32::from_bits(reader.read_u32_le()?);
    if value.is_finite() {
        Ok(value)
    } else {
        Err(W3dMaterialError::NonFiniteScalar { material, field })
    }
}

fn limited_count(value: u32, what: &'static str, maximum: usize) -> Result<usize, BinaryError> {
    let value = usize::try_from(value).map_err(|_| BinaryError::LimitExceeded {
        what,
        actual: usize::MAX,
        maximum,
    })?;
    if value > maximum {
        Err(BinaryError::LimitExceeded {
            what,
            actual: value,
            maximum,
        })
    } else {
        Ok(value)
    }
}

fn payload_size(
    count: usize,
    record_bytes: usize,
    what: &'static str,
) -> Result<usize, W3dMaterialError> {
    count
        .checked_mul(record_bytes)
        .ok_or(W3dMaterialError::SizeOverflow { what })
}

#[cfg(test)]
mod tests {
    use super::{
        DIFFUSE_COLORS_CHUNK, DIFFUSE_ILLUMINATION_CHUNK, MATERIAL_INFO_CHUNK, MATERIAL_PASS_CHUNK,
        SHADER_IDS_CHUNK, SHADERS_CHUNK, SPECULAR_COLORS_CHUNK, STAGE_TEXCOORDS_CHUNK,
        TEXTURE_CHUNK, TEXTURE_IDS_CHUNK, TEXTURE_INFO_CHUNK, TEXTURE_NAME_CHUNK,
        TEXTURE_STAGE_CHUNK, TEXTURES_CHUNK, VERTEX_MAPPER_ARGS0_CHUNK, VERTEX_MAPPER_ARGS1_CHUNK,
        VERTEX_MATERIAL_CHUNK, VERTEX_MATERIAL_IDS_CHUNK, VERTEX_MATERIAL_INFO_CHUNK,
        VERTEX_MATERIAL_NAME_CHUNK, VERTEX_MATERIALS_CHUNK, W3dFaceIds, W3dMaterialError,
        W3dMaterialIds,
    };
    use crate::{W3dChunk, W3dLimits, W3dMeshError, W3dMeshLimits, decode_static_mesh, parse_w3d};

    fn fixture() -> Vec<u8> {
        let hex = include_str!("../tests/fixtures/colored-mesh.w3d.hex");
        let digits = hex
            .bytes()
            .filter(u8::is_ascii_hexdigit)
            .collect::<Vec<_>>();
        digits
            .chunks_exact(2)
            .map(|pair| {
                let pair = std::str::from_utf8(pair).expect("ASCII fixture");
                u8::from_str_radix(pair, 16).expect("valid hex fixture")
            })
            .collect()
    }

    fn decode(bytes: &[u8]) -> Result<crate::W3dStaticMesh, W3dMeshError> {
        let file = parse_w3d(bytes, "colored-mesh.w3d", W3dLimits::default())
            .expect("valid chunk framing");
        decode_static_mesh(&file.chunks()[0], W3dMeshLimits::default())
    }

    fn append_chunk(output: &mut Vec<u8>, id: u32, container: bool, payload: &[u8]) {
        output.extend_from_slice(&id.to_le_bytes());
        let size = u32::try_from(payload.len()).expect("fixture payload fits u32")
            | if container { 0x8000_0000 } else { 0 };
        output.extend_from_slice(&size.to_le_bytes());
        output.extend_from_slice(payload);
    }

    fn textured_fixture() -> Vec<u8> {
        let mut bytes = fixture();
        let info = payload_offset(&bytes, MATERIAL_INFO_CHUNK);
        bytes[info + 8..info + 12].copy_from_slice(&1_u32.to_le_bytes());
        bytes[info + 12..info + 16].copy_from_slice(&1_u32.to_le_bytes());
        let pass_header = payload_offset(&bytes, MATERIAL_PASS_CHUNK) - 8;

        append_chunk(&mut bytes, SHADER_IDS_CHUNK, false, &0_u32.to_le_bytes());
        let mut stage = Vec::new();
        append_chunk(&mut stage, TEXTURE_IDS_CHUNK, false, &0_u32.to_le_bytes());
        let mut coordinates = Vec::new();
        for value in [0.0_f32, 0.0, 1.0, 0.0, 0.0, 1.0] {
            coordinates.extend_from_slice(&value.to_le_bytes());
        }
        append_chunk(&mut stage, STAGE_TEXCOORDS_CHUNK, false, &coordinates);
        append_chunk(&mut bytes, TEXTURE_STAGE_CHUNK, true, &stage);
        bytes[pass_header + 4..pass_header + 8]
            .copy_from_slice(&(76_u32 | 0x8000_0000).to_le_bytes());

        let shader = [3, 1, 0, 0, 0, 1, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0];
        append_chunk(&mut bytes, SHADERS_CHUNK, false, &shader);
        let mut texture = Vec::new();
        append_chunk(&mut texture, TEXTURE_NAME_CHUNK, false, b"checker.tga\0");
        let mut texture_info = Vec::new();
        texture_info.extend_from_slice(&0_u16.to_le_bytes());
        texture_info.extend_from_slice(&0_u16.to_le_bytes());
        texture_info.extend_from_slice(&1_u32.to_le_bytes());
        texture_info.extend_from_slice(&0_f32.to_le_bytes());
        append_chunk(&mut texture, TEXTURE_INFO_CHUNK, false, &texture_info);
        let mut texture_entry = Vec::new();
        append_chunk(&mut texture_entry, TEXTURE_CHUNK, true, &texture);
        append_chunk(&mut bytes, TEXTURES_CHUNK, true, &texture_entry);
        bytes[4..8].copy_from_slice(&(508_u32 | 0x8000_0000).to_le_bytes());
        bytes
    }

    fn find_chunk(chunks: &[W3dChunk], id: u32) -> Option<&W3dChunk> {
        for chunk in chunks {
            if chunk.id() == id {
                return Some(chunk);
            }
            if let Some(found) = chunk
                .children()
                .and_then(|children| find_chunk(children, id))
            {
                return Some(found);
            }
        }
        None
    }

    fn payload_offset(bytes: &[u8], id: u32) -> usize {
        let file = parse_w3d(bytes, "offsets.w3d", W3dLimits::default()).expect("valid framing");
        find_chunk(file.chunks(), id)
            .expect("fixture chunk")
            .offset()
            + 8
    }

    fn increase_container_length(bytes: &mut [u8], header: usize, addition: usize) {
        let size = u32::from_le_bytes(
            bytes[header + 4..header + 8]
                .try_into()
                .expect("container size word"),
        );
        let payload = (size & 0x7FFF_FFFF)
            .checked_add(u32::try_from(addition).expect("test addition fits u32"))
            .expect("test container size fits u32");
        bytes[header + 4..header + 8].copy_from_slice(&(payload | 0x8000_0000).to_le_bytes());
    }

    fn enhanced_material_fixture() -> Vec<u8> {
        let mut bytes = fixture();
        let file = parse_w3d(&bytes, "enhanced-material.w3d", W3dLimits::default())
            .expect("valid base fixture");
        let mesh = &file.chunks()[0];
        let materials =
            find_chunk(file.chunks(), VERTEX_MATERIALS_CHUNK).expect("vertex-material wrapper");
        let material =
            find_chunk(file.chunks(), VERTEX_MATERIAL_CHUNK).expect("vertex-material container");
        let mesh_header = mesh.offset();
        let wrapper_header = materials.offset();
        let vertex_material_header = material.offset();
        let insertion = vertex_material_header + 8 + material.payload_length();
        let mut mapper_chunks = Vec::new();
        append_chunk(
            &mut mapper_chunks,
            VERTEX_MAPPER_ARGS0_CHUNK,
            false,
            b"UPerSec=1.0;\0",
        );
        append_chunk(
            &mut mapper_chunks,
            VERTEX_MAPPER_ARGS1_CHUNK,
            false,
            b"Speed=0.25;\0",
        );
        let addition = mapper_chunks.len();
        bytes.splice(insertion..insertion, mapper_chunks);
        increase_container_length(&mut bytes, vertex_material_header, addition);
        increase_container_length(&mut bytes, wrapper_header, addition);
        increase_container_length(&mut bytes, mesh_header, addition);

        let info = payload_offset(&bytes, VERTEX_MATERIAL_INFO_CHUNK);
        bytes[info..info + 4].copy_from_slice(&0x0004_0800_u32.to_le_bytes());

        let file = parse_w3d(&bytes, "enhanced-material.w3d", W3dLimits::default())
            .expect("valid mapper fixture");
        let pass = find_chunk(file.chunks(), MATERIAL_PASS_CHUNK).expect("material pass");
        let pass_header = pass.offset();
        let insertion = pass_header + 8 + pass.payload_length();
        let mut color_chunks = Vec::new();
        append_chunk(
            &mut color_chunks,
            DIFFUSE_ILLUMINATION_CHUNK,
            false,
            &[10, 20, 30, 0, 40, 50, 60, 0, 70, 80, 90, 0],
        );
        append_chunk(
            &mut color_chunks,
            SPECULAR_COLORS_CHUNK,
            false,
            &[1, 2, 3, 0, 4, 5, 6, 0, 7, 8, 9, 0],
        );
        let addition = color_chunks.len();
        bytes.splice(insertion..insertion, color_chunks);
        increase_container_length(&mut bytes, pass_header, addition);
        increase_container_length(&mut bytes, mesh_header, addition);
        bytes
    }

    #[test]
    fn decodes_vertex_material_diffuse_color_and_single_assignment() {
        let mesh = decode(&fixture()).expect("valid colored mesh");
        let materials = mesh.materials();
        let info = materials.info().expect("material info");

        assert_eq!(info.pass_count(), 1);
        assert_eq!(info.vertex_material_count(), 1);
        assert_eq!(info.shader_count(), 0);
        assert_eq!(info.texture_count(), 0);
        assert_eq!(
            materials.vertex_materials()[0].name_bytes(),
            Some(b"Red".as_slice())
        );
        assert_eq!(materials.vertex_materials()[0].diffuse().red(), u8::MAX);
        assert_eq!(materials.vertex_materials()[0].diffuse().green(), 0);
        assert!(matches!(
            materials.passes()[0].vertex_material_ids(),
            Some(W3dMaterialIds::Single(0))
        ));
        assert_eq!(
            mesh.preview_vertex_colors().expect("resolved colors"),
            vec![
                crate::W3dRgba8 {
                    red: 255,
                    green: 0,
                    blue: 0,
                    alpha: 255,
                };
                3
            ]
        );

        let mut per_vertex = fixture();
        let ids = payload_offset(&per_vertex, VERTEX_MATERIAL_IDS_CHUNK);
        let pass = payload_offset(&per_vertex, MATERIAL_PASS_CHUNK) - 8;
        per_vertex[ids - 4..ids].copy_from_slice(&12_u32.to_le_bytes());
        per_vertex[pass + 4..pass + 8].copy_from_slice(&(20_u32 | 0x8000_0000).to_le_bytes());
        per_vertex[4..8].copy_from_slice(&(372_u32 | 0x8000_0000).to_le_bytes());
        per_vertex.extend_from_slice(&[0_u8; 8]);
        let mesh = decode(&per_vertex).expect("valid per-vertex material IDs");
        assert!(matches!(
            mesh.materials().passes()[0].vertex_material_ids(),
            Some(W3dMaterialIds::PerVertex(ids)) if ids == &[0, 0, 0]
        ));
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn decodes_shader_texture_binding_and_uvs() {
        let mesh = decode(&textured_fixture()).expect("valid textured mesh");
        let materials = mesh.materials();
        assert_eq!(materials.shaders().len(), 1);
        assert_eq!(materials.shaders()[0].texturing(), 1);
        assert_eq!(materials.textures()[0].name_bytes(), b"checker.tga");
        assert_eq!(
            materials.textures()[0]
                .info()
                .expect("texture info")
                .frame_count(),
            1
        );
        let pass = &materials.passes()[0];
        assert!(matches!(pass.shader_ids(), Some(W3dFaceIds::Single(0))));
        let stage = &pass.texture_stages()[0];
        assert!(matches!(stage.texture_ids(), Some(W3dFaceIds::Single(0))));
        assert_eq!(stage.texture_coordinates().len(), 3);
        assert_eq!(
            stage.texture_coordinates()[2].v().to_bits(),
            1.0_f32.to_bits()
        );

        let mut invalid = textured_fixture();
        let id = payload_offset(&invalid, TEXTURE_IDS_CHUNK);
        invalid[id..id + 4].copy_from_slice(&1_u32.to_le_bytes());
        assert!(matches!(
            decode(&invalid),
            Err(W3dMeshError::Material(
                W3dMaterialError::ResourceIndexOutOfRange {
                    what: "texture",
                    index: 1,
                    count: 1,
                    ..
                }
            ))
        ));

        let mut unterminated = textured_fixture();
        let name = payload_offset(&unterminated, TEXTURE_NAME_CHUNK);
        unterminated[name + b"checker.tga".len()] = b'!';
        assert!(matches!(
            decode(&unterminated),
            Err(W3dMeshError::Material(
                W3dMaterialError::UnterminatedTextureName { texture: 0 }
            ))
        ));

        let mut non_finite_uv = textured_fixture();
        let uv = payload_offset(&non_finite_uv, STAGE_TEXCOORDS_CHUNK);
        non_finite_uv[uv..uv + 4].copy_from_slice(&f32::NAN.to_le_bytes());
        let decoded = decode(&non_finite_uv).expect("non-finite UV source bits are preserved");
        let uv = decoded.materials().passes()[0].texture_stages()[0].texture_coordinates()[0];
        assert!(uv.u().is_nan());

        let mut invalid_animation = textured_fixture();
        let texture_info = payload_offset(&invalid_animation, TEXTURE_INFO_CHUNK);
        invalid_animation[texture_info + 2..texture_info + 4].copy_from_slice(&4_u16.to_le_bytes());
        assert!(matches!(
            decode(&invalid_animation),
            Err(W3dMeshError::Material(
                W3dMaterialError::InvalidTextureAnimationType {
                    texture: 0,
                    animation_type: 4
                }
            ))
        ));

        let mut zero_frames = textured_fixture();
        let texture_info = payload_offset(&zero_frames, TEXTURE_INFO_CHUNK);
        zero_frames[texture_info + 4..texture_info + 8].copy_from_slice(&0_u32.to_le_bytes());
        assert!(matches!(
            decode(&zero_frames),
            Err(W3dMeshError::Material(
                W3dMaterialError::ZeroTextureFrameCount { texture: 0 }
            ))
        ));

        let mut stopped_animation = textured_fixture();
        let texture_info = payload_offset(&stopped_animation, TEXTURE_INFO_CHUNK);
        stopped_animation[texture_info + 4..texture_info + 8].copy_from_slice(&2_u32.to_le_bytes());
        assert!(matches!(
            decode(&stopped_animation),
            Err(W3dMeshError::Material(
                W3dMaterialError::ZeroAnimatedTextureFrameRate { texture: 0 }
            ))
        ));

        let bytes = textured_fixture();
        let file = parse_w3d(&bytes, "texture-limit.w3d", W3dLimits::default())
            .expect("valid chunk framing");
        assert!(matches!(
            decode_static_mesh(
                &file.chunks()[0],
                W3dMeshLimits {
                    maximum_textures: 0,
                    ..W3dMeshLimits::default()
                }
            ),
            Err(W3dMeshError::Material(W3dMaterialError::Binary(
                cic_core::BinaryError::LimitExceeded {
                    what: "W3D texture count",
                    actual: 1,
                    maximum: 0
                }
            )))
        ));

        assert!(matches!(
            decode_static_mesh(
                &file.chunks()[0],
                W3dMeshLimits {
                    maximum_texture_stages_per_pass: 0,
                    ..W3dMeshLimits::default()
                }
            ),
            Err(W3dMeshError::Material(W3dMaterialError::Binary(
                cic_core::BinaryError::LimitExceeded {
                    what: "W3D texture stage count per pass",
                    actual: 1,
                    maximum: 0
                }
            )))
        ));
    }

    #[test]
    fn decodes_mapper_arguments_and_secondary_pass_colors() {
        let bytes = enhanced_material_fixture();
        let mesh = decode(&bytes).expect("enhanced material fixture");
        let material = &mesh.materials().vertex_materials()[0];
        let mapper0 = material.mapper(0).expect("stage zero mapper");
        assert_eq!(mapper0.mode().code(), 4);
        assert_eq!(mapper0.mode().name(), Some("linear_offset"));
        assert_eq!(mapper0.argument_bytes(), Some(b"UPerSec=1.0;".as_slice()));
        let mapper1 = material.mapper(1).expect("stage one mapper");
        assert_eq!(mapper1.mode().code(), 8);
        assert_eq!(mapper1.argument_bytes(), Some(b"Speed=0.25;".as_slice()));
        let pass = &mesh.materials().passes()[0];
        assert_eq!(pass.diffuse_illumination().expect("DIG")[2].green(), 80);
        assert_eq!(pass.specular_colors().expect("SCG")[1].blue(), 6);

        let mut unterminated = bytes;
        let arguments = payload_offset(&unterminated, VERTEX_MAPPER_ARGS0_CHUNK);
        unterminated[arguments + b"UPerSec=1.0;".len()] = b'!';
        assert!(matches!(
            decode(&unterminated),
            Err(W3dMeshError::Material(
                W3dMaterialError::UnterminatedMapperArguments {
                    material: 0,
                    stage: 0
                }
            ))
        ));
    }

    #[test]
    fn explicit_diffuse_color_array_overrides_vertex_material_color() {
        let mut bytes = fixture();
        let pass = payload_offset(&bytes, MATERIAL_PASS_CHUNK) - 8;
        bytes[pass + 4..pass + 8].copy_from_slice(&(32_u32 | 0x8000_0000).to_le_bytes());
        bytes[4..8].copy_from_slice(&(384_u32 | 0x8000_0000).to_le_bytes());
        bytes.extend_from_slice(&DIFFUSE_COLORS_CHUNK.to_le_bytes());
        bytes.extend_from_slice(&12_u32.to_le_bytes());
        bytes.extend_from_slice(&[255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255]);

        let colors = decode(&bytes)
            .expect("valid DCG colors")
            .preview_vertex_colors()
            .expect("resolved colors");
        assert_eq!(colors[0].red(), 255);
        assert_eq!(colors[1].green(), 255);
        assert_eq!(colors[2].blue(), 255);
    }

    #[test]
    fn rejects_material_count_id_name_and_limit_violations() {
        let mut wrong_count = fixture();
        let info = payload_offset(&wrong_count, MATERIAL_INFO_CHUNK);
        wrong_count[info + 4..info + 8].copy_from_slice(&2_u32.to_le_bytes());
        assert!(matches!(
            decode(&wrong_count),
            Err(W3dMeshError::Material(W3dMaterialError::CountMismatch {
                what: "vertex materials",
                declared: 2,
                actual: 1
            }))
        ));

        let mut bad_id = fixture();
        let ids = payload_offset(&bad_id, VERTEX_MATERIAL_IDS_CHUNK);
        bad_id[ids..ids + 4].copy_from_slice(&1_u32.to_le_bytes());
        assert!(matches!(
            decode(&bad_id),
            Err(W3dMeshError::Material(
                W3dMaterialError::MaterialIndexOutOfRange {
                    pass: 0,
                    vertex: None,
                    index: 1,
                    material_count: 1
                }
            ))
        ));

        let mut unterminated = fixture();
        let name = payload_offset(&unterminated, VERTEX_MATERIAL_NAME_CHUNK);
        unterminated[name + 3] = b'!';
        assert!(matches!(
            decode(&unterminated),
            Err(W3dMeshError::Material(W3dMaterialError::UnterminatedName {
                material: 0
            }))
        ));

        let bytes = fixture();
        let file =
            parse_w3d(&bytes, "limits.w3d", W3dLimits::default()).expect("valid chunk framing");
        assert!(matches!(
            decode_static_mesh(
                &file.chunks()[0],
                W3dMeshLimits {
                    maximum_vertex_materials: 0,
                    ..W3dMeshLimits::default()
                }
            ),
            Err(W3dMeshError::Material(W3dMaterialError::Binary(
                cic_core::BinaryError::LimitExceeded {
                    what: "W3D vertex material count",
                    actual: 1,
                    maximum: 0
                }
            )))
        ));
    }
}
