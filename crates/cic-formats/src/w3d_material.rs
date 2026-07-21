//! W3D vertex-material and first-pass diffuse-color decoding.
//!
//! Provenance: this implementation was authored for Commanders in Chief from
//! `w3d_file.h`, `meshmdlio.cpp`, and `vertmaterial.cpp` at `GeneralsGameCode`
//! revision `9f7abb866f5afd446db14149979e744c7216baaf`. Those sources are
//! GPL-3.0-or-later with Electronic Arts Section 7 terms; no source code or retail
//! content is copied.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

use cic_core::{BinaryError, BinaryReader};

use crate::w3d::W3dChunk;
use crate::w3d_mesh::W3dMeshLimits;

const MATERIAL_INFO_CHUNK: u32 = 0x0000_0028;
const VERTEX_MATERIALS_CHUNK: u32 = 0x0000_002A;
const VERTEX_MATERIAL_CHUNK: u32 = 0x0000_002B;
const VERTEX_MATERIAL_NAME_CHUNK: u32 = 0x0000_002C;
const VERTEX_MATERIAL_INFO_CHUNK: u32 = 0x0000_002D;
const MATERIAL_PASS_CHUNK: u32 = 0x0000_0038;
const VERTEX_MATERIAL_IDS_CHUNK: u32 = 0x0000_0039;
const DIFFUSE_COLORS_CHUNK: u32 = 0x0000_003B;

const MATERIAL_INFO_BYTES: usize = 16;
const VERTEX_MATERIAL_INFO_BYTES: usize = 32;
const COLOR_BYTES: usize = 4;

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

/// Decoded color-relevant values for one material pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct W3dMaterialPass {
    vertex_material_ids: Option<W3dMaterialIds>,
    diffuse_colors: Option<Vec<W3dRgba8>>,
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
}

/// Material inventory, vertex materials, and color-relevant pass assignments.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct W3dMaterialSet {
    info: Option<W3dMaterialInfo>,
    vertex_materials: Vec<W3dVertexMaterial>,
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
    limits: W3dMeshLimits,
) -> Result<W3dMaterialSet, W3dMaterialError> {
    let has_material_children = children.iter().any(|child| {
        matches!(
            child.id(),
            MATERIAL_INFO_CHUNK | VERTEX_MATERIALS_CHUNK | MATERIAL_PASS_CHUNK
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
            vertex_count,
            vertex_materials.len(),
        )?);
    }
    require_count("material passes", expected_passes, passes.len())?;

    Ok(W3dMaterialSet {
        info: Some(info),
        vertex_materials,
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
    parse_vertex_material(bytes, material_index, name)
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
    })
}

fn decode_pass(
    chunk: &W3dChunk,
    pass_index: usize,
    vertex_count: usize,
    material_count: usize,
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
                vertex_count,
                material_count,
            )
        })
        .transpose()?;
    let diffuse_colors = unique_child(children, DIFFUSE_COLORS_CHUNK)?
        .map(|chunk| parse_diffuse_colors(require_data(chunk)?, vertex_count))
        .transpose()?;
    Ok(W3dMaterialPass {
        vertex_material_ids,
        diffuse_colors,
    })
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
        DIFFUSE_COLORS_CHUNK, MATERIAL_INFO_CHUNK, MATERIAL_PASS_CHUNK, VERTEX_MATERIAL_IDS_CHUNK,
        VERTEX_MATERIAL_NAME_CHUNK, W3dMaterialError, W3dMaterialIds,
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
