//! W3D chunk-container inventory decoding.
//!
//! Format facts and clean-room implementation provenance are recorded in
//! `docs/formats/w3d.md` and `docs/provenance/w3d.md`.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

use cic_core::{BinaryError, BinaryReader};

const CONTAINER_FLAG: u32 = 0x8000_0000;
const PAYLOAD_LENGTH_MASK: u32 = 0x7FFF_FFFF;
const CHUNK_HEADER_BYTES: usize = 8;

/// Explicit resource limits for one W3D chunk inventory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct W3dLimits {
    /// Maximum complete input length.
    pub maximum_file_bytes: usize,
    /// Maximum chunks across every nesting level.
    pub maximum_chunks: usize,
    /// Maximum zero-based chunk nesting depth.
    pub maximum_depth: usize,
}

impl Default for W3dLimits {
    fn default() -> Self {
        Self {
            maximum_file_bytes: 256 * 1024 * 1024,
            maximum_chunks: 1_000_000,
            maximum_depth: 64,
        }
    }
}

/// One complete W3D chunk stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct W3dFile {
    chunks: Vec<W3dChunk>,
}

impl W3dFile {
    /// Returns top-level chunks in file order.
    #[must_use]
    pub fn chunks(&self) -> &[W3dChunk] {
        &self.chunks
    }
}

/// One W3D chunk with its absolute header offset and preserved payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct W3dChunk {
    id: u32,
    offset: usize,
    payload_length: usize,
    payload: W3dPayload,
}

impl W3dChunk {
    /// Returns the numeric chunk identifier.
    #[must_use]
    pub const fn id(&self) -> u32 {
        self.id
    }

    /// Returns the absolute byte offset of this chunk's header.
    #[must_use]
    pub const fn offset(&self) -> usize {
        self.offset
    }

    /// Returns payload length, excluding the eight-byte chunk header.
    #[must_use]
    pub const fn payload_length(&self) -> usize {
        self.payload_length
    }

    /// Returns whether the header marks this as a child-chunk container.
    #[must_use]
    pub const fn is_container(&self) -> bool {
        matches!(self.payload, W3dPayload::Chunks(_))
    }

    /// Returns the preserved payload representation.
    #[must_use]
    pub const fn payload(&self) -> &W3dPayload {
        &self.payload
    }

    /// Returns leaf data, or `None` for a container.
    #[must_use]
    pub fn data(&self) -> Option<&[u8]> {
        match &self.payload {
            W3dPayload::Data(bytes) => Some(bytes),
            W3dPayload::Chunks(_) => None,
        }
    }

    /// Returns child chunks, or `None` for a data leaf.
    #[must_use]
    pub fn children(&self) -> Option<&[Self]> {
        match &self.payload {
            W3dPayload::Data(_) => None,
            W3dPayload::Chunks(chunks) => Some(chunks),
        }
    }
}

/// Lossless chunk payload classification from the header's high bit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum W3dPayload {
    /// Opaque leaf bytes, including bytes of unknown chunk types.
    Data(Vec<u8>),
    /// Nested chunks that exactly fill the declared payload region.
    Chunks(Vec<W3dChunk>),
}

/// A structured W3D inventory failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum W3dError {
    /// A bounded binary read or resource limit failed.
    Binary(BinaryError),
    /// A zero-byte stream cannot identify a W3D asset.
    Empty,
    /// Absolute offset arithmetic exceeded the platform's address range.
    OffsetOverflow,
}

impl Display for W3dError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binary(error) => Display::fmt(error, formatter),
            Self::Empty => formatter.write_str("W3D chunk stream is empty"),
            Self::OffsetOverflow => formatter.write_str("W3D chunk offset arithmetic overflowed"),
        }
    }
}

impl Error for W3dError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Binary(error) => Some(error),
            Self::Empty | Self::OffsetOverflow => None,
        }
    }
}

impl From<BinaryError> for W3dError {
    fn from(error: BinaryError) -> Self {
        Self::Binary(error)
    }
}

/// Inventories a complete W3D chunk stream and preserves all leaf payload bytes.
///
/// # Errors
///
/// Returns [`W3dError`] for empty input, truncation, a child stream that does not exactly
/// fill its parent, depth/count/file limit excess, or offset arithmetic overflow.
pub fn parse_w3d(
    bytes: &[u8],
    source: impl Into<String>,
    limits: W3dLimits,
) -> Result<W3dFile, W3dError> {
    enforce_limit("W3D file size", bytes.len(), limits.maximum_file_bytes)?;
    if bytes.is_empty() {
        return Err(W3dError::Empty);
    }

    let mut reader = BinaryReader::new(bytes, source);
    let mut chunk_count = 0_usize;
    let chunks = parse_region(&mut reader, 0, 0, &mut chunk_count, limits)?;
    Ok(W3dFile { chunks })
}

/// Returns the GPL-header name of a known W3D chunk identifier.
#[must_use]
pub const fn w3d_chunk_name(id: u32) -> Option<&'static str> {
    match id {
        0x0000_0000 => Some("W3D_CHUNK_MESH"),
        0x0000_0002 => Some("W3D_CHUNK_VERTICES"),
        0x0000_0003 => Some("W3D_CHUNK_VERTEX_NORMALS"),
        0x0000_000C => Some("W3D_CHUNK_MESH_USER_TEXT"),
        0x0000_000E => Some("W3D_CHUNK_VERTEX_INFLUENCES"),
        0x0000_001F => Some("W3D_CHUNK_MESH_HEADER3"),
        0x0000_0020 => Some("W3D_CHUNK_TRIANGLES"),
        0x0000_0022 => Some("W3D_CHUNK_VERTEX_SHADE_INDICES"),
        0x0000_0023 => Some("W3D_CHUNK_PRELIT_UNLIT"),
        0x0000_0024 => Some("W3D_CHUNK_PRELIT_VERTEX"),
        0x0000_0025 => Some("W3D_CHUNK_PRELIT_LIGHTMAP_MULTI_PASS"),
        0x0000_0026 => Some("W3D_CHUNK_PRELIT_LIGHTMAP_MULTI_TEXTURE"),
        0x0000_0028 => Some("W3D_CHUNK_MATERIAL_INFO"),
        0x0000_0029 => Some("W3D_CHUNK_SHADERS"),
        0x0000_002A => Some("W3D_CHUNK_VERTEX_MATERIALS"),
        0x0000_002B => Some("W3D_CHUNK_VERTEX_MATERIAL"),
        0x0000_002C => Some("W3D_CHUNK_VERTEX_MATERIAL_NAME"),
        0x0000_002D => Some("W3D_CHUNK_VERTEX_MATERIAL_INFO"),
        0x0000_002E => Some("W3D_CHUNK_VERTEX_MAPPER_ARGS0"),
        0x0000_002F => Some("W3D_CHUNK_VERTEX_MAPPER_ARGS1"),
        0x0000_0030 => Some("W3D_CHUNK_TEXTURES"),
        0x0000_0031 => Some("W3D_CHUNK_TEXTURE"),
        0x0000_0032 => Some("W3D_CHUNK_TEXTURE_NAME"),
        0x0000_0033 => Some("W3D_CHUNK_TEXTURE_INFO"),
        0x0000_0038 => Some("W3D_CHUNK_MATERIAL_PASS"),
        0x0000_0039 => Some("W3D_CHUNK_VERTEX_MATERIAL_IDS"),
        0x0000_003A => Some("W3D_CHUNK_SHADER_IDS"),
        0x0000_003B => Some("W3D_CHUNK_DCG"),
        0x0000_003C => Some("W3D_CHUNK_DIG"),
        0x0000_003E => Some("W3D_CHUNK_SCG"),
        0x0000_003F => Some("W3D_CHUNK_FXSHADER_IDS"),
        0x0000_0048 => Some("W3D_CHUNK_TEXTURE_STAGE"),
        0x0000_0049 => Some("W3D_CHUNK_TEXTURE_IDS"),
        0x0000_004A => Some("W3D_CHUNK_STAGE_TEXCOORDS"),
        0x0000_004B => Some("W3D_CHUNK_PER_FACE_TEXCOORD_IDS"),
        0x0000_0050 => Some("W3D_CHUNK_FX_SHADERS"),
        0x0000_0051 => Some("W3D_CHUNK_FX_SHADER"),
        0x0000_0052 => Some("W3D_CHUNK_FX_SHADER_INFO"),
        0x0000_0053 => Some("W3D_CHUNK_FX_SHADER_CONSTANT"),
        0x0000_0058 => Some("W3D_CHUNK_DEFORM"),
        0x0000_0059 => Some("W3D_CHUNK_DEFORM_SET"),
        0x0000_005A => Some("W3D_CHUNK_DEFORM_KEYFRAME"),
        0x0000_005B => Some("W3D_CHUNK_DEFORM_DATA"),
        0x0000_0060 => Some("W3D_CHUNK_VERTEX_TANGENTS"),
        0x0000_0061 => Some("W3D_CHUNK_VERTEX_BINORMALS"),
        0x0000_0080 => Some("W3D_CHUNK_PS2_SHADERS"),
        0x0000_0090 => Some("W3D_CHUNK_AABTREE"),
        0x0000_0091 => Some("W3D_CHUNK_AABTREE_HEADER"),
        0x0000_0092 => Some("W3D_CHUNK_AABTREE_POLYINDICES"),
        0x0000_0093 => Some("W3D_CHUNK_AABTREE_NODES"),
        0x0000_0100 => Some("W3D_CHUNK_HIERARCHY"),
        0x0000_0101 => Some("W3D_CHUNK_HIERARCHY_HEADER"),
        0x0000_0102 => Some("W3D_CHUNK_PIVOTS"),
        0x0000_0103 => Some("W3D_CHUNK_PIVOT_FIXUPS"),
        0x0000_0200 => Some("W3D_CHUNK_ANIMATION"),
        0x0000_0201 => Some("W3D_CHUNK_ANIMATION_HEADER"),
        0x0000_0202 => Some("W3D_CHUNK_ANIMATION_CHANNEL"),
        0x0000_0203 => Some("W3D_CHUNK_BIT_CHANNEL"),
        0x0000_0280 => Some("W3D_CHUNK_COMPRESSED_ANIMATION"),
        0x0000_0281 => Some("W3D_CHUNK_COMPRESSED_ANIMATION_HEADER"),
        0x0000_0282 => Some("W3D_CHUNK_COMPRESSED_ANIMATION_CHANNEL"),
        0x0000_0283 => Some("W3D_CHUNK_COMPRESSED_BIT_CHANNEL"),
        0x0000_02C0 => Some("W3D_CHUNK_MORPH_ANIMATION"),
        0x0000_0300 => Some("W3D_CHUNK_HMODEL"),
        0x0000_0400 => Some("W3D_CHUNK_LODMODEL"),
        0x0000_0420 => Some("W3D_CHUNK_COLLECTION"),
        0x0000_0440 => Some("W3D_CHUNK_POINTS"),
        0x0000_0460 => Some("W3D_CHUNK_LIGHT"),
        0x0000_0500 => Some("W3D_CHUNK_EMITTER"),
        0x0000_0600 => Some("W3D_CHUNK_AGGREGATE"),
        0x0000_0700 => Some("W3D_CHUNK_HLOD"),
        0x0000_0701 => Some("W3D_CHUNK_HLOD_HEADER"),
        0x0000_0702 => Some("W3D_CHUNK_HLOD_LOD_ARRAY"),
        0x0000_0703 => Some("W3D_CHUNK_HLOD_SUB_OBJECT_ARRAY_HEADER"),
        0x0000_0704 => Some("W3D_CHUNK_HLOD_SUB_OBJECT"),
        0x0000_0740 => Some("W3D_CHUNK_BOX"),
        0x0000_0750 => Some("W3D_CHUNK_NULL_OBJECT"),
        _ => None,
    }
}

fn parse_region(
    reader: &mut BinaryReader<'_>,
    region_offset: usize,
    depth: usize,
    chunk_count: &mut usize,
    limits: W3dLimits,
) -> Result<Vec<W3dChunk>, W3dError> {
    let mut chunks = Vec::new();
    while reader.remaining() != 0 {
        enforce_limit("W3D chunk depth", depth, limits.maximum_depth)?;
        let local_offset = reader.position();
        let absolute_offset = region_offset
            .checked_add(local_offset)
            .ok_or(W3dError::OffsetOverflow)?;
        let id = reader.read_u32_le()?;
        let size_and_flags = reader.read_u32_le()?;
        let payload_length =
            usize::try_from(size_and_flags & PAYLOAD_LENGTH_MASK).map_err(|_| {
                BinaryError::LimitExceeded {
                    what: "W3D chunk payload length",
                    actual: usize::MAX,
                    maximum: limits.maximum_file_bytes,
                }
            })?;
        enforce_limit(
            "W3D chunk payload length",
            payload_length,
            limits.maximum_file_bytes,
        )?;

        *chunk_count = chunk_count
            .checked_add(1)
            .ok_or(BinaryError::LimitExceeded {
                what: "W3D total chunk count",
                actual: usize::MAX,
                maximum: limits.maximum_chunks,
            })?;
        enforce_limit("W3D total chunk count", *chunk_count, limits.maximum_chunks)?;

        let mut payload_reader = reader.read_region(payload_length)?;
        let payload = if size_and_flags & CONTAINER_FLAG == 0 {
            W3dPayload::Data(payload_reader.read_exact(payload_length)?.to_vec())
        } else {
            let payload_offset = absolute_offset
                .checked_add(CHUNK_HEADER_BYTES)
                .ok_or(W3dError::OffsetOverflow)?;
            let child_depth = depth.checked_add(1).ok_or(W3dError::OffsetOverflow)?;
            W3dPayload::Chunks(parse_region(
                &mut payload_reader,
                payload_offset,
                child_depth,
                chunk_count,
                limits,
            )?)
        };
        chunks.push(W3dChunk {
            id,
            offset: absolute_offset,
            payload_length,
            payload,
        });
    }
    Ok(chunks)
}

fn enforce_limit(what: &'static str, actual: usize, maximum: usize) -> Result<(), BinaryError> {
    if actual > maximum {
        Err(BinaryError::LimitExceeded {
            what,
            actual,
            maximum,
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{W3dError, W3dLimits, W3dPayload, parse_w3d};

    fn fixture() -> Vec<u8> {
        let hex = include_str!("../tests/fixtures/minimal.w3d.hex");
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

    #[test]
    fn inventories_nested_and_unknown_chunks_losslessly() {
        let parsed =
            parse_w3d(&fixture(), "minimal.w3d", W3dLimits::default()).expect("valid fixture");
        assert_eq!(parsed.chunks().len(), 2);

        let mesh = &parsed.chunks()[0];
        assert_eq!(mesh.id(), 0);
        assert_eq!(mesh.offset(), 0);
        assert_eq!(mesh.payload_length(), 29);
        let children = mesh.children().expect("mesh container");
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].offset(), 8);
        assert_eq!(children[0].data(), Some(b"abc".as_slice()));
        assert_eq!(children[1].offset(), 19);
        assert!(children[1].is_container());
        let grandchild = &children[1].children().expect("nested container")[0];
        assert_eq!(grandchild.offset(), 27);
        assert_eq!(grandchild.data(), Some([1_u8, 2].as_slice()));

        let unknown = &parsed.chunks()[1];
        assert_eq!(unknown.id(), 0xDEAD_BEEF);
        assert_eq!(unknown.offset(), 37);
        assert_eq!(unknown.data(), Some([0xDE, 0xAD, 0xBE, 0xEF].as_slice()));
        assert!(matches!(mesh.payload(), W3dPayload::Chunks(_)));
    }

    #[test]
    fn every_truncated_prefix_returns_an_error() {
        let bytes = fixture();
        let single_top_level_chunk = &bytes[..37];
        for length in 0..single_top_level_chunk.len() {
            assert!(
                parse_w3d(
                    &single_top_level_chunk[..length],
                    "truncated.w3d",
                    W3dLimits::default()
                )
                .is_err(),
                "prefix of {length} bytes unexpectedly parsed"
            );
        }
    }

    #[test]
    fn rejects_empty_input_and_each_resource_limit() {
        assert_eq!(
            parse_w3d(&[], "empty.w3d", W3dLimits::default()),
            Err(W3dError::Empty)
        );
        let bytes = fixture();
        let cases = [
            (
                W3dLimits {
                    maximum_file_bytes: bytes.len() - 1,
                    ..W3dLimits::default()
                },
                "W3D file size",
            ),
            (
                W3dLimits {
                    maximum_chunks: 3,
                    ..W3dLimits::default()
                },
                "W3D total chunk count",
            ),
            (
                W3dLimits {
                    maximum_depth: 1,
                    ..W3dLimits::default()
                },
                "W3D chunk depth",
            ),
        ];
        for (limits, expected) in cases {
            assert!(matches!(
                parse_w3d(&bytes, "limits.w3d", limits),
                Err(W3dError::Binary(cic_core::BinaryError::LimitExceeded {
                    what,
                    ..
                })) if what == expected
            ));
        }
    }

    #[test]
    fn container_payload_must_be_an_exact_child_stream() {
        let mut bytes = fixture();
        bytes[4..8].copy_from_slice(&30_u32.wrapping_add(0x8000_0000).to_le_bytes());
        bytes.insert(37, 0);

        assert!(matches!(
            parse_w3d(&bytes, "padding.w3d", W3dLimits::default()),
            Err(W3dError::Binary(
                cic_core::BinaryError::UnexpectedEof { .. }
            ))
        ));
    }
}
