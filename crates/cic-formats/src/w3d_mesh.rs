//! Semantic decoding for W3D version-3 static mesh chunks.
//!
//! Provenance: this implementation was authored for Commanders in Chief from the
//! on-disk structure declarations in `w3d_file.h` and the count-driven reads in
//! `meshmdlio.cpp`/`meshgeometry.cpp` at `GeneralsGameCode` revision
//! `9f7abb866f5afd446db14149979e744c7216baaf`. Those sources are GPL-3.0-or-later
//! with Electronic Arts Section 7 terms; no source code or retail content is copied.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

use cic_core::{BinaryError, BinaryReader};

use crate::w3d::W3dChunk;
use crate::w3d_material::{W3dMaterialError, W3dMaterialSet, W3dRgba8, decode_materials};

const MESH_CHUNK: u32 = 0x0000_0000;
const VERTICES_CHUNK: u32 = 0x0000_0002;
const NORMALS_CHUNK: u32 = 0x0000_0003;
const HEADER3_CHUNK: u32 = 0x0000_001F;
const TRIANGLES_CHUNK: u32 = 0x0000_0020;
const VERTEX_INFLUENCES_CHUNK: u32 = 0x0000_000E;

const HEADER3_BYTES: usize = 116;
const VECTOR_BYTES: usize = 12;
const TRIANGLE_BYTES: usize = 32;
const MINIMUM_HEADER3_VERSION: u32 = 0x0003_0000;
const MAXIMUM_VERIFIED_MESH_VERSION: u32 = 0x0004_0002;
const GEOMETRY_TYPE_MASK: u32 = 0x00FF_0000;
const GEOMETRY_TYPE_SKIN: u32 = 0x0002_0000;
const VERTEX_CHANNEL_LOCATION: u32 = 0x0000_0001;
const VERTEX_CHANNEL_BONE_ID: u32 = 0x0000_0010;
const FACE_CHANNEL_FACE: u32 = 0x0000_0001;

/// Explicit allocation limits for one decoded W3D mesh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct W3dMeshLimits {
    /// Maximum vertices declared by the mesh header.
    pub maximum_vertices: usize,
    /// Maximum triangles declared by the mesh header.
    pub maximum_triangles: usize,
    /// Maximum material passes declared by material info.
    pub maximum_material_passes: usize,
    /// Maximum vertex materials declared by material info.
    pub maximum_vertex_materials: usize,
    /// Maximum shaders declared by material info.
    pub maximum_shaders: usize,
    /// Maximum textures declared by material info.
    pub maximum_textures: usize,
    /// Maximum vertex-material name length, excluding its terminator.
    pub maximum_material_name_bytes: usize,
    /// Maximum mapper-argument string length, excluding its terminator.
    pub maximum_mapper_argument_bytes: usize,
    /// Maximum texture name length, excluding its terminator.
    pub maximum_texture_name_bytes: usize,
    /// Maximum frame count declared by one animated texture.
    pub maximum_texture_animation_frames: usize,
    /// Maximum texture coordinates across one stage.
    pub maximum_texture_coordinates: usize,
    /// Maximum texture stages decoded for one material pass.
    pub maximum_texture_stages_per_pass: usize,
}

impl Default for W3dMeshLimits {
    fn default() -> Self {
        Self {
            maximum_vertices: 4_000_000,
            maximum_triangles: 4_000_000,
            maximum_material_passes: 64,
            maximum_vertex_materials: 65_536,
            maximum_shaders: 65_536,
            maximum_textures: 65_536,
            maximum_material_name_bytes: 255,
            maximum_mapper_argument_bytes: 4_096,
            maximum_texture_name_bytes: 255,
            maximum_texture_animation_frames: 65_536,
            maximum_texture_coordinates: 12_000_000,
            maximum_texture_stages_per_pass: 8,
        }
    }
}

/// One renderer-neutral three-component vector from a W3D mesh.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct W3dVector3 {
    x: f32,
    y: f32,
    z: f32,
}

impl W3dVector3 {
    /// Returns the X component.
    #[must_use]
    pub const fn x(self) -> f32 {
        self.x
    }

    /// Returns the Y component.
    #[must_use]
    pub const fn y(self) -> f32 {
        self.y
    }

    /// Returns the Z component.
    #[must_use]
    pub const fn z(self) -> f32 {
        self.z
    }
}

/// The fixed 116-byte `W3dMeshHeader3Struct` value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct W3dMeshHeader3 {
    version: u32,
    attributes: u32,
    mesh_name: [u8; 16],
    container_name: [u8; 16],
    triangle_count: u32,
    vertex_count: u32,
    material_count: u32,
    damage_stage_count: u32,
    sort_level: i32,
    prelit_version: u32,
    future_count: u32,
    vertex_channels: u32,
    face_channels: u32,
    minimum: W3dVector3,
    maximum: W3dVector3,
    sphere_center: W3dVector3,
    sphere_radius: f32,
}

impl W3dMeshHeader3 {
    /// Returns the packed major/minor mesh version.
    #[must_use]
    pub const fn version(self) -> u32 {
        self.version
    }

    /// Returns the raw mesh attribute bits.
    #[must_use]
    pub const fn attributes(self) -> u32 {
        self.attributes
    }

    /// Returns the uninterpreted fixed-width mesh-name bytes.
    #[must_use]
    pub const fn mesh_name_bytes(&self) -> &[u8; 16] {
        &self.mesh_name
    }

    /// Returns the uninterpreted fixed-width container-name bytes.
    #[must_use]
    pub const fn container_name_bytes(&self) -> &[u8; 16] {
        &self.container_name
    }

    /// Returns the declared triangle count.
    #[must_use]
    pub const fn triangle_count(self) -> u32 {
        self.triangle_count
    }

    /// Returns the declared vertex count.
    #[must_use]
    pub const fn vertex_count(self) -> u32 {
        self.vertex_count
    }

    /// Returns the declared material count.
    #[must_use]
    pub const fn material_count(self) -> u32 {
        self.material_count
    }

    /// Returns the declared damage-stage count.
    #[must_use]
    pub const fn damage_stage_count(self) -> u32 {
        self.damage_stage_count
    }

    /// Returns the static mesh sorting level.
    #[must_use]
    pub const fn sort_level(self) -> i32 {
        self.sort_level
    }

    /// Returns the raw prelighting-tool version.
    #[must_use]
    pub const fn prelit_version(self) -> u32 {
        self.prelit_version
    }

    /// Returns the preserved future-count field.
    #[must_use]
    pub const fn future_count(self) -> u32 {
        self.future_count
    }

    /// Returns the vertex-channel bit field.
    #[must_use]
    pub const fn vertex_channels(self) -> u32 {
        self.vertex_channels
    }

    /// Returns the face-channel bit field.
    #[must_use]
    pub const fn face_channels(self) -> u32 {
        self.face_channels
    }

    /// Returns the bounding-box minimum.
    #[must_use]
    pub const fn minimum(self) -> W3dVector3 {
        self.minimum
    }

    /// Returns the bounding-box maximum.
    #[must_use]
    pub const fn maximum(self) -> W3dVector3 {
        self.maximum
    }

    /// Returns the bounding-sphere center.
    #[must_use]
    pub const fn sphere_center(self) -> W3dVector3 {
        self.sphere_center
    }

    /// Returns the bounding-sphere radius.
    #[must_use]
    pub const fn sphere_radius(self) -> f32 {
        self.sphere_radius
    }
}

/// One fixed 32-byte `W3dTriStruct` value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct W3dTriangle {
    vertex_indices: [u32; 3],
    attributes: u32,
    normal: W3dVector3,
    distance: f32,
}

impl W3dTriangle {
    /// Returns the three vertex indices in file order.
    #[must_use]
    pub const fn vertex_indices(self) -> [u32; 3] {
        self.vertex_indices
    }

    /// Returns the raw triangle attribute bits.
    #[must_use]
    pub const fn attributes(self) -> u32 {
        self.attributes
    }

    /// Returns the stored plane normal.
    #[must_use]
    pub const fn normal(self) -> W3dVector3 {
        self.normal
    }

    /// Returns the stored plane distance.
    #[must_use]
    pub const fn distance(self) -> f32 {
        self.distance
    }
}

/// One immutable static mesh decoded from a `W3D_CHUNK_MESH` container.
#[derive(Debug, Clone, PartialEq)]
pub struct W3dStaticMesh {
    header: W3dMeshHeader3,
    vertices: Vec<W3dVector3>,
    normals: Vec<W3dVector3>,
    triangles: Vec<W3dTriangle>,
    vertex_bones: Option<Vec<u16>>,
    materials: W3dMaterialSet,
}

impl W3dStaticMesh {
    /// Returns the decoded version-3 mesh header.
    #[must_use]
    pub const fn header(&self) -> W3dMeshHeader3 {
        self.header
    }

    /// Returns vertices in file order.
    #[must_use]
    pub fn vertices(&self) -> &[W3dVector3] {
        &self.vertices
    }

    /// Returns vertex normals in file order.
    #[must_use]
    pub fn normals(&self) -> &[W3dVector3] {
        &self.normals
    }

    /// Returns triangles in file order.
    #[must_use]
    pub fn triangles(&self) -> &[W3dTriangle] {
        &self.triangles
    }

    /// Returns one rigid bone index per vertex for skin geometry.
    #[must_use]
    pub fn vertex_bones(&self) -> Option<&[u16]> {
        self.vertex_bones.as_deref()
    }

    /// Returns decoded material colors and first-pass assignments.
    #[must_use]
    pub const fn materials(&self) -> &W3dMaterialSet {
        &self.materials
    }

    /// Resolves first-pass per-vertex diffuse colors for geometry previews.
    ///
    /// Explicit per-vertex DCG colors take precedence over vertex-material diffuse colors.
    #[must_use]
    pub fn preview_vertex_colors(&self) -> Option<Vec<W3dRgba8>> {
        self.materials.preview_vertex_colors(self.vertices.len())
    }
}

/// A structured static-mesh decoding failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum W3dMeshError {
    /// A bounded binary read or geometry count limit failed.
    Binary(BinaryError),
    /// Material metadata or color assignment was malformed.
    Material(W3dMaterialError),
    /// The supplied chunk was not `W3D_CHUNK_MESH`.
    NotMeshChunk {
        /// Actual numeric chunk identifier.
        actual: u32,
    },
    /// The mesh chunk was a data leaf instead of a child container.
    MeshMustBeContainer,
    /// The required header was absent or not the first mesh child.
    HeaderMustBeFirst {
        /// First child identifier, or `None` for an empty mesh container.
        actual: Option<u32>,
    },
    /// A required geometry chunk was absent.
    MissingChunk {
        /// Required numeric chunk identifier.
        id: u32,
    },
    /// A required geometry chunk appeared more than once.
    DuplicateChunk {
        /// Duplicated numeric chunk identifier.
        id: u32,
    },
    /// A semantic geometry chunk incorrectly carried child chunks.
    ChunkMustBeData {
        /// Numeric chunk identifier.
        id: u32,
    },
    /// A fixed-size or count-sized payload had the wrong byte length.
    InvalidChunkLength {
        /// Numeric chunk identifier.
        id: u32,
        /// Actual payload byte length.
        actual: usize,
        /// Required payload byte length.
        expected: usize,
    },
    /// The mesh header version is outside the implemented Header3 range.
    UnsupportedVersion {
        /// Actual packed version.
        actual: u32,
    },
    /// Geometry type and bone-channel declarations were unsupported or inconsistent.
    UnsupportedGeometry {
        /// Raw mesh attributes.
        attributes: u32,
        /// Raw vertex channels.
        vertex_channels: u32,
    },
    /// A semantic chunk was present for a geometry type that does not permit it.
    UnexpectedChunk {
        /// Unexpected numeric chunk identifier.
        id: u32,
    },
    /// A mandatory location or face channel was not declared.
    MissingRequiredChannel {
        /// Channel set name.
        kind: &'static str,
        /// Required channel bit.
        required: u32,
        /// Actual channel bits.
        actual: u32,
    },
    /// Count-to-byte-size multiplication overflowed.
    SizeOverflow {
        /// Record array being sized.
        what: &'static str,
    },
    /// A triangle referenced a vertex outside the decoded vertex array.
    VertexIndexOutOfRange {
        /// Zero-based triangle index.
        triangle: usize,
        /// Zero-based corner within the triangle.
        corner: usize,
        /// Referenced vertex index.
        index: u32,
        /// Declared vertex count.
        vertex_count: usize,
    },
}

impl Display for W3dMeshError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binary(error) => Display::fmt(error, formatter),
            Self::Material(error) => Display::fmt(error, formatter),
            Self::NotMeshChunk { actual } => {
                write!(formatter, "expected W3D mesh chunk, found 0x{actual:08X}")
            }
            Self::MeshMustBeContainer => formatter.write_str("W3D mesh chunk must be a container"),
            Self::HeaderMustBeFirst { actual } => match actual {
                Some(actual) => write!(
                    formatter,
                    "W3D mesh header3 must be the first child, found 0x{actual:08X}"
                ),
                None => formatter.write_str("W3D mesh container has no header3 child"),
            },
            Self::MissingChunk { id } => {
                write!(formatter, "W3D mesh is missing required chunk 0x{id:08X}")
            }
            Self::DuplicateChunk { id } => {
                write!(formatter, "W3D mesh repeats semantic chunk 0x{id:08X}")
            }
            Self::ChunkMustBeData { id } => {
                write!(
                    formatter,
                    "W3D semantic chunk 0x{id:08X} must be a data leaf"
                )
            }
            Self::InvalidChunkLength {
                id,
                actual,
                expected,
            } => write!(
                formatter,
                "W3D chunk 0x{id:08X} has {actual} payload bytes; expected {expected}"
            ),
            Self::UnsupportedVersion { actual } => write!(
                formatter,
                "unsupported W3D mesh header3 version 0x{actual:08X}; supported range is 3.0 through 4.2"
            ),
            Self::UnsupportedGeometry {
                attributes,
                vertex_channels,
            } => write!(
                formatter,
                "unsupported or inconsistent W3D geometry (attributes 0x{attributes:08X}, vertex channels 0x{vertex_channels:08X})"
            ),
            Self::UnexpectedChunk { id } => write!(
                formatter,
                "W3D geometry contains unexpected semantic chunk 0x{id:08X}"
            ),
            Self::MissingRequiredChannel {
                kind,
                required,
                actual,
            } => write!(
                formatter,
                "W3D mesh {kind} channels 0x{actual:08X} omit required bit 0x{required:08X}"
            ),
            Self::SizeOverflow { what } => {
                write!(formatter, "W3D {what} payload size overflowed")
            }
            Self::VertexIndexOutOfRange {
                triangle,
                corner,
                index,
                vertex_count,
            } => write!(
                formatter,
                "W3D triangle {triangle} corner {corner} references vertex {index}, but the mesh has {vertex_count} vertices"
            ),
        }
    }
}

impl Error for W3dMeshError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Binary(error) => Some(error),
            Self::Material(error) => Some(error),
            _ => None,
        }
    }
}

impl From<BinaryError> for W3dMeshError {
    fn from(error: BinaryError) -> Self {
        Self::Binary(error)
    }
}

impl From<W3dMaterialError> for W3dMeshError {
    fn from(error: W3dMaterialError) -> Self {
        Self::Material(error)
    }
}

/// Decodes one immutable, renderer-neutral static mesh from an inventoried mesh chunk.
///
/// Unknown sibling chunks remain preserved in the original [`W3dChunk`] and are ignored here.
/// Header3 must be first; vertices, normals, and triangles must each occur exactly once as
/// data leaves with lengths matching the header counts. Triangle indices are range checked.
///
/// # Errors
///
/// Returns [`W3dMeshError`] for the wrong chunk shape, missing/duplicate semantic chunks,
/// unsupported versions or inconsistent geometry declarations, count/length disagreements, configured
/// limit excess, truncation, or an out-of-range triangle index.
pub fn decode_static_mesh(
    chunk: &W3dChunk,
    limits: W3dMeshLimits,
) -> Result<W3dStaticMesh, W3dMeshError> {
    if chunk.id() != MESH_CHUNK {
        return Err(W3dMeshError::NotMeshChunk { actual: chunk.id() });
    }
    let children = chunk.children().ok_or(W3dMeshError::MeshMustBeContainer)?;
    if children.first().map(W3dChunk::id) != Some(HEADER3_CHUNK) {
        return Err(W3dMeshError::HeaderMustBeFirst {
            actual: children.first().map(W3dChunk::id),
        });
    }

    let header_data = required_data(children, HEADER3_CHUNK)?;
    if header_data.len() != HEADER3_BYTES {
        return Err(W3dMeshError::InvalidChunkLength {
            id: HEADER3_CHUNK,
            actual: header_data.len(),
            expected: HEADER3_BYTES,
        });
    }
    let header = parse_header(header_data)?;
    validate_header(header)?;

    let vertex_count = limited_count(
        header.vertex_count,
        "W3D mesh vertex count",
        limits.maximum_vertices,
    )?;
    let triangle_count = limited_count(
        header.triangle_count,
        "W3D mesh triangle count",
        limits.maximum_triangles,
    )?;

    let vertices = parse_vectors(
        required_data(children, VERTICES_CHUNK)?,
        vertex_count,
        VERTICES_CHUNK,
    )?;
    let normals = parse_vectors(
        required_data(children, NORMALS_CHUNK)?,
        vertex_count,
        NORMALS_CHUNK,
    )?;
    let triangles = parse_triangles(required_data(children, TRIANGLES_CHUNK)?, triangle_count)?;
    validate_indices(&triangles, vertex_count)?;
    let vertex_bones = if header.attributes & GEOMETRY_TYPE_MASK == GEOMETRY_TYPE_SKIN {
        Some(parse_vertex_bones(
            required_data(children, VERTEX_INFLUENCES_CHUNK)?,
            vertex_count,
        )?)
    } else {
        if optional_data(children, VERTEX_INFLUENCES_CHUNK)?.is_some() {
            return Err(W3dMeshError::UnexpectedChunk {
                id: VERTEX_INFLUENCES_CHUNK,
            });
        }
        None
    };
    let materials = decode_materials(children, vertex_count, triangle_count, limits)?;

    Ok(W3dStaticMesh {
        header,
        vertices,
        normals,
        triangles,
        vertex_bones,
        materials,
    })
}

fn required_data(children: &[W3dChunk], id: u32) -> Result<&[u8], W3dMeshError> {
    let mut matching = children.iter().filter(|child| child.id() == id);
    let chunk = matching.next().ok_or(W3dMeshError::MissingChunk { id })?;
    if matching.next().is_some() {
        return Err(W3dMeshError::DuplicateChunk { id });
    }
    chunk.data().ok_or(W3dMeshError::ChunkMustBeData { id })
}

fn optional_data(children: &[W3dChunk], id: u32) -> Result<Option<&[u8]>, W3dMeshError> {
    let mut matching = children.iter().filter(|child| child.id() == id);
    let first = matching.next();
    if matching.next().is_some() {
        return Err(W3dMeshError::DuplicateChunk { id });
    }
    first
        .map(|chunk| chunk.data().ok_or(W3dMeshError::ChunkMustBeData { id }))
        .transpose()
}

fn parse_vertex_bones(bytes: &[u8], vertex_count: usize) -> Result<Vec<u16>, W3dMeshError> {
    let expected = payload_size(vertex_count, 8, "vertex influence")?;
    if bytes.len() != expected {
        return Err(W3dMeshError::InvalidChunkLength {
            id: VERTEX_INFLUENCES_CHUNK,
            actual: bytes.len(),
            expected,
        });
    }
    let mut reader = BinaryReader::new(bytes, "W3D vertex influences");
    let mut bones = Vec::with_capacity(vertex_count);
    for _ in 0..vertex_count {
        bones.push(reader.read_u16_le()?);
        reader.skip(6)?;
    }
    Ok(bones)
}

fn parse_header(bytes: &[u8]) -> Result<W3dMeshHeader3, BinaryError> {
    let mut reader = BinaryReader::new(bytes, "W3D mesh header3");
    Ok(W3dMeshHeader3 {
        version: reader.read_u32_le()?,
        attributes: reader.read_u32_le()?,
        mesh_name: read_fixed_bytes(&mut reader)?,
        container_name: read_fixed_bytes(&mut reader)?,
        triangle_count: reader.read_u32_le()?,
        vertex_count: reader.read_u32_le()?,
        material_count: reader.read_u32_le()?,
        damage_stage_count: reader.read_u32_le()?,
        sort_level: i32::from_le_bytes(reader.read_u32_le()?.to_le_bytes()),
        prelit_version: reader.read_u32_le()?,
        future_count: reader.read_u32_le()?,
        vertex_channels: reader.read_u32_le()?,
        face_channels: reader.read_u32_le()?,
        minimum: read_vector(&mut reader)?,
        maximum: read_vector(&mut reader)?,
        sphere_center: read_vector(&mut reader)?,
        sphere_radius: read_f32(&mut reader)?,
    })
}

fn validate_header(header: W3dMeshHeader3) -> Result<(), W3dMeshError> {
    if !(MINIMUM_HEADER3_VERSION..=MAXIMUM_VERIFIED_MESH_VERSION).contains(&header.version) {
        return Err(W3dMeshError::UnsupportedVersion {
            actual: header.version,
        });
    }
    let geometry_type = header.attributes & GEOMETRY_TYPE_MASK;
    if !matches!(geometry_type, 0 | GEOMETRY_TYPE_SKIN)
        || (geometry_type == 0 && header.vertex_channels & VERTEX_CHANNEL_BONE_ID != 0)
        || (geometry_type == GEOMETRY_TYPE_SKIN
            && header.vertex_channels & VERTEX_CHANNEL_BONE_ID == 0)
    {
        return Err(W3dMeshError::UnsupportedGeometry {
            attributes: header.attributes,
            vertex_channels: header.vertex_channels,
        });
    }
    require_channel("vertex", header.vertex_channels, VERTEX_CHANNEL_LOCATION)?;
    require_channel("face", header.face_channels, FACE_CHANNEL_FACE)
}

fn require_channel(kind: &'static str, actual: u32, required: u32) -> Result<(), W3dMeshError> {
    if actual & required == 0 {
        Err(W3dMeshError::MissingRequiredChannel {
            kind,
            required,
            actual,
        })
    } else {
        Ok(())
    }
}

fn parse_vectors(
    bytes: &[u8],
    count: usize,
    chunk_id: u32,
) -> Result<Vec<W3dVector3>, W3dMeshError> {
    let expected = payload_size(count, VECTOR_BYTES, "vector")?;
    require_length(chunk_id, bytes.len(), expected)?;
    let mut reader = BinaryReader::new(bytes, "W3D vector array");
    let mut vectors = Vec::with_capacity(count);
    for _ in 0..count {
        vectors.push(read_vector(&mut reader)?);
    }
    Ok(vectors)
}

fn parse_triangles(bytes: &[u8], count: usize) -> Result<Vec<W3dTriangle>, W3dMeshError> {
    let expected = payload_size(count, TRIANGLE_BYTES, "triangle")?;
    require_length(TRIANGLES_CHUNK, bytes.len(), expected)?;
    let mut reader = BinaryReader::new(bytes, "W3D triangle array");
    let mut triangles = Vec::with_capacity(count);
    for _ in 0..count {
        triangles.push(W3dTriangle {
            vertex_indices: [
                reader.read_u32_le()?,
                reader.read_u32_le()?,
                reader.read_u32_le()?,
            ],
            attributes: reader.read_u32_le()?,
            normal: read_vector(&mut reader)?,
            distance: read_f32(&mut reader)?,
        });
    }
    Ok(triangles)
}

fn validate_indices(triangles: &[W3dTriangle], vertex_count: usize) -> Result<(), W3dMeshError> {
    for (triangle_index, triangle) in triangles.iter().enumerate() {
        for (corner, index) in triangle.vertex_indices.iter().copied().enumerate() {
            let valid = usize::try_from(index).is_ok_and(|index| index < vertex_count);
            if !valid {
                return Err(W3dMeshError::VertexIndexOutOfRange {
                    triangle: triangle_index,
                    corner,
                    index,
                    vertex_count,
                });
            }
        }
    }
    Ok(())
}

fn read_fixed_bytes<const LENGTH: usize>(
    reader: &mut BinaryReader<'_>,
) -> Result<[u8; LENGTH], BinaryError> {
    let mut bytes = [0_u8; LENGTH];
    bytes.copy_from_slice(reader.read_exact(LENGTH)?);
    Ok(bytes)
}

fn read_vector(reader: &mut BinaryReader<'_>) -> Result<W3dVector3, BinaryError> {
    Ok(W3dVector3 {
        x: read_f32(reader)?,
        y: read_f32(reader)?,
        z: read_f32(reader)?,
    })
}

fn read_f32(reader: &mut BinaryReader<'_>) -> Result<f32, BinaryError> {
    Ok(f32::from_bits(reader.read_u32_le()?))
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
) -> Result<usize, W3dMeshError> {
    count
        .checked_mul(record_bytes)
        .ok_or(W3dMeshError::SizeOverflow { what })
}

fn require_length(id: u32, actual: usize, expected: usize) -> Result<(), W3dMeshError> {
    if actual == expected {
        Ok(())
    } else {
        Err(W3dMeshError::InvalidChunkLength {
            id,
            actual,
            expected,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        HEADER3_CHUNK, NORMALS_CHUNK, TRIANGLES_CHUNK, VERTICES_CHUNK, W3dMeshError, W3dMeshLimits,
        decode_static_mesh,
    };
    use crate::{W3dLimits, parse_w3d};

    fn fixture() -> Vec<u8> {
        let hex = include_str!("../tests/fixtures/static-mesh.w3d.hex");
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

    fn decode(bytes: &[u8]) -> Result<super::W3dStaticMesh, W3dMeshError> {
        let file =
            parse_w3d(bytes, "static-mesh.w3d", W3dLimits::default()).expect("valid chunk framing");
        decode_static_mesh(&file.chunks()[0], W3dMeshLimits::default())
    }

    fn child_payload_offset(bytes: &[u8], id: u32) -> usize {
        let file = parse_w3d(bytes, "offsets.w3d", W3dLimits::default()).expect("valid framing");
        file.chunks()[0]
            .children()
            .expect("mesh children")
            .iter()
            .find(|child| child.id() == id)
            .expect("fixture child")
            .offset()
            + 8
    }

    fn child_range(bytes: &[u8], id: u32) -> std::ops::Range<usize> {
        let file = parse_w3d(bytes, "ranges.w3d", W3dLimits::default()).expect("valid framing");
        let child = file.chunks()[0]
            .children()
            .expect("mesh children")
            .iter()
            .find(|child| child.id() == id)
            .expect("fixture child");
        child.offset()..child.offset() + 8 + child.payload_length()
    }

    fn update_outer_payload_length(bytes: &mut [u8]) {
        let payload_length = u32::try_from(bytes.len() - 8).expect("small fixture");
        bytes[4..8].copy_from_slice(&(payload_length | 0x8000_0000).to_le_bytes());
    }

    #[test]
    fn decodes_original_static_triangle_fixture() {
        let mesh = decode(&fixture()).expect("valid static mesh");

        assert_eq!(mesh.header().version(), 0x0004_0002);
        assert_eq!(mesh.header().mesh_name_bytes()[..4], *b"Tri\0");
        assert_eq!(mesh.header().container_name_bytes()[..5], *b"Test\0");
        assert_eq!(mesh.header().triangle_count(), 1);
        assert_eq!(mesh.header().vertex_count(), 3);
        assert_eq!(mesh.vertices().len(), 3);
        assert_eq!(mesh.vertices()[1].x().to_bits(), 1.0_f32.to_bits());
        assert_eq!(mesh.vertices()[2].y().to_bits(), 1.0_f32.to_bits());
        assert_eq!(mesh.normals().len(), 3);
        assert_eq!(mesh.normals()[0].z().to_bits(), 1.0_f32.to_bits());
        assert_eq!(mesh.triangles()[0].vertex_indices(), [0, 1, 2]);
        assert_eq!(
            mesh.triangles()[0].normal().z().to_bits(),
            1.0_f32.to_bits()
        );
        assert_eq!(mesh.triangles()[0].distance().to_bits(), 0.0_f32.to_bits());
    }

    #[test]
    fn rejects_header_count_and_payload_length_disagreement() {
        let mut bytes = fixture();
        let header = child_payload_offset(&bytes, HEADER3_CHUNK);
        bytes[header + 44..header + 48].copy_from_slice(&4_u32.to_le_bytes());

        assert!(matches!(
            decode(&bytes),
            Err(W3dMeshError::InvalidChunkLength {
                id: VERTICES_CHUNK,
                actual: 36,
                expected: 48
            })
        ));
    }

    #[test]
    fn rejects_out_of_range_triangle_index() {
        let mut bytes = fixture();
        let triangles = child_payload_offset(&bytes, TRIANGLES_CHUNK);
        bytes[triangles + 8..triangles + 12].copy_from_slice(&3_u32.to_le_bytes());

        assert!(matches!(
            decode(&bytes),
            Err(W3dMeshError::VertexIndexOutOfRange {
                triangle: 0,
                corner: 2,
                index: 3,
                vertex_count: 3
            })
        ));
    }

    #[test]
    fn rejects_unsupported_versions_channels_and_geometry() {
        let cases = [
            (0_usize, 0x0002_0000_u32, "version"),
            (4, 0x0002_0000, "geometry"),
            (68, 0x0000_0011, "bone channel"),
            (68, 0, "location channel"),
            (72, 0, "face channel"),
        ];

        for (relative_offset, value, expected) in cases {
            let mut bytes = fixture();
            let header = child_payload_offset(&bytes, HEADER3_CHUNK);
            bytes[header + relative_offset..header + relative_offset + 4]
                .copy_from_slice(&value.to_le_bytes());
            let error = decode(&bytes).expect_err(expected);
            match expected {
                "version" => assert!(matches!(error, W3dMeshError::UnsupportedVersion { .. })),
                "geometry" | "bone channel" => {
                    assert!(matches!(error, W3dMeshError::UnsupportedGeometry { .. }));
                }
                _ => assert!(matches!(error, W3dMeshError::MissingRequiredChannel { .. })),
            }
        }
    }

    #[test]
    fn enforces_geometry_count_limits_before_allocation() {
        let bytes = fixture();
        let file = parse_w3d(&bytes, "limits.w3d", W3dLimits::default()).expect("valid framing");
        let error = decode_static_mesh(
            &file.chunks()[0],
            W3dMeshLimits {
                maximum_vertices: 2,
                ..W3dMeshLimits::default()
            },
        )
        .expect_err("vertex limit");
        assert!(matches!(
            error,
            W3dMeshError::Binary(cic_core::BinaryError::LimitExceeded {
                what: "W3D mesh vertex count",
                actual: 3,
                maximum: 2
            })
        ));
    }

    #[test]
    fn requires_each_geometry_chunk_exactly_once_as_data() {
        let mut missing = fixture();
        missing.drain(child_range(&missing, NORMALS_CHUNK));
        update_outer_payload_length(&mut missing);
        assert!(matches!(
            decode(&missing),
            Err(W3dMeshError::MissingChunk { id: NORMALS_CHUNK })
        ));

        let mut duplicate = fixture();
        let triangle = duplicate[child_range(&duplicate, TRIANGLES_CHUNK)].to_vec();
        duplicate.extend_from_slice(&triangle);
        update_outer_payload_length(&mut duplicate);
        assert!(matches!(
            decode(&duplicate),
            Err(W3dMeshError::DuplicateChunk {
                id: TRIANGLES_CHUNK
            })
        ));

        let mut container = fixture();
        let vertices = child_range(&container, VERTICES_CHUNK);
        container[vertices.start + 4..vertices.start + 8]
            .copy_from_slice(&0x8000_0000_u32.to_le_bytes());
        container.drain(vertices.start + 8..vertices.end);
        update_outer_payload_length(&mut container);
        assert!(matches!(
            decode(&container),
            Err(W3dMeshError::ChunkMustBeData { id: VERTICES_CHUNK })
        ));
    }
}
