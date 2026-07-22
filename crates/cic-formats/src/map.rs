//! MAP `CkMp` chunk inventory and terrain-height decoding.
//!
//! Format facts and clean-room implementation provenance are recorded in
//! `docs/formats/map.md` and `docs/provenance/map.md`. The source basis is
//! `GeneralsGameCode` revision `9f7abb866f5afd446db14149979e744c7216baaf`,
//! licensed under GPL-3.0-or-later with Electronic Arts Section 7 terms.

use std::borrow::Cow;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use cic_core::{BinaryError, BinaryReader};

use crate::refpack::{RefPackError, decompress_ear};

const MAGIC: &[u8; 4] = b"CkMp";
const CHUNK_HEADER_BYTES: usize = 10;
const HEIGHT_MAP_LABEL: &[u8] = b"HeightMapData";

/// Explicit resource limits for one MAP file and its height field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapLimits {
    /// Maximum complete input length.
    pub maximum_file_bytes: usize,
    /// Maximum length after an `EAR` `RefPack` wrapper is decompressed.
    pub maximum_decompressed_bytes: usize,
    /// Maximum symbol-table entries.
    pub maximum_symbols: usize,
    /// Maximum bytes in one symbol name.
    pub maximum_symbol_bytes: usize,
    /// Maximum top-level chunks.
    pub maximum_chunks: usize,
    /// Maximum payload bytes in one chunk.
    pub maximum_chunk_bytes: usize,
    /// Maximum width or height of a decoded height field.
    pub maximum_height_dimension: usize,
    /// Maximum decoded height samples.
    pub maximum_height_samples: usize,
    /// Maximum version-4 playable boundaries.
    pub maximum_boundaries: usize,
    /// Maximum declared terrain bitmap tiles.
    pub maximum_bitmap_tiles: usize,
    /// Maximum declared edge/shore bitmap tiles.
    pub maximum_edge_tiles: usize,
    /// Maximum declared blend-table entries.
    pub maximum_blended_tiles: usize,
    /// Maximum declared cliff-table entries.
    pub maximum_cliff_records: usize,
    /// Maximum terrain or edge texture classes.
    pub maximum_texture_classes: usize,
    /// Maximum bytes in one texture-class name.
    pub maximum_texture_name_bytes: usize,
    /// Maximum polygon triggers inspected by the water-only decoder.
    pub maximum_polygon_triggers: usize,
    /// Maximum points in one polygon trigger.
    pub maximum_polygon_points: usize,
    /// Maximum points retained across all water polygon triggers.
    pub maximum_water_points: usize,
    /// Maximum bytes in one polygon-trigger name.
    pub maximum_trigger_name_bytes: usize,
}

impl Default for MapLimits {
    fn default() -> Self {
        Self {
            maximum_file_bytes: 512 * 1024 * 1024,
            maximum_decompressed_bytes: 512 * 1024 * 1024,
            maximum_symbols: 4_096,
            maximum_symbol_bytes: 255,
            maximum_chunks: 1_000_000,
            maximum_chunk_bytes: 512 * 1024 * 1024,
            maximum_height_dimension: 16_384,
            maximum_height_samples: 16_777_216,
            maximum_boundaries: 4_096,
            maximum_bitmap_tiles: 2_047,
            maximum_edge_tiles: 2_047,
            maximum_blended_tiles: 16_192,
            maximum_cliff_records: 32_384,
            maximum_texture_classes: 256,
            maximum_texture_name_bytes: 1_024,
            maximum_polygon_triggers: 65_536,
            maximum_polygon_points: 65_536,
            maximum_water_points: 1_000_000,
            maximum_trigger_name_bytes: 1_024,
        }
    }
}

/// One complete MAP symbol table and top-level chunk stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapFile {
    compression: MapCompression,
    symbols: Vec<MapSymbol>,
    chunks: Vec<MapChunk>,
}

impl MapFile {
    /// Returns how the parsed chunk stream was stored.
    #[must_use]
    pub const fn compression(&self) -> MapCompression {
        self.compression
    }

    /// Returns symbol-table entries in file order.
    #[must_use]
    pub fn symbols(&self) -> &[MapSymbol] {
        &self.symbols
    }

    /// Returns top-level chunks in file order.
    #[must_use]
    pub fn chunks(&self) -> &[MapChunk] {
        &self.chunks
    }

    /// Resolves an identifier using the legacy reader's last-table-entry-wins behavior.
    #[must_use]
    pub fn symbol_name(&self, id: u32) -> Option<&[u8]> {
        self.symbols
            .iter()
            .rev()
            .find(|symbol| symbol.id == id)
            .map(|symbol| symbol.name.as_slice())
    }
}

/// Storage wrapper used by a parsed MAP resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapCompression {
    /// The input was a bare `CkMp` stream.
    None,
    /// The input used the source-established `EAR\0` `RefPack` wrapper.
    RefPack,
}

impl Display for MapCompression {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::None => "none",
            Self::RefPack => "refpack",
        })
    }
}

/// One MAP symbol-table entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapSymbol {
    offset: usize,
    id: u32,
    name: Vec<u8>,
}

impl MapSymbol {
    /// Returns the absolute offset of the entry's length byte.
    #[must_use]
    pub const fn offset(&self) -> usize {
        self.offset
    }

    /// Returns the numeric chunk identifier.
    #[must_use]
    pub const fn id(&self) -> u32 {
        self.id
    }

    /// Returns the losslessly preserved symbol-name bytes.
    #[must_use]
    pub fn name_bytes(&self) -> &[u8] {
        &self.name
    }
}

/// One top-level MAP chunk with an opaque preserved payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapChunk {
    offset: usize,
    id: u32,
    version: u16,
    data: Vec<u8>,
}

impl MapChunk {
    /// Returns the absolute offset of the chunk header.
    #[must_use]
    pub const fn offset(&self) -> usize {
        self.offset
    }

    /// Returns the numeric symbol-table identifier.
    #[must_use]
    pub const fn id(&self) -> u32 {
        self.id
    }

    /// Returns the chunk's version field.
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }

    /// Returns the opaque payload bytes.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }
}

/// One playable-boundary coordinate from a version-4 height chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapBoundary {
    x: i32,
    y: i32,
}

impl MapBoundary {
    /// Returns the stored signed boundary X coordinate.
    #[must_use]
    pub const fn x(self) -> i32 {
        self.x
    }

    /// Returns the stored signed boundary Y coordinate.
    #[must_use]
    pub const fn y(self) -> i32 {
        self.y
    }
}

/// Immutable, renderer-neutral height samples from `HeightMapData`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapHeightField {
    version: u16,
    width: u32,
    height: u32,
    border_size: u32,
    boundaries: Vec<MapBoundary>,
    samples: Vec<u8>,
}

impl MapHeightField {
    /// Returns the source chunk version.
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }

    /// Returns the stored sample width.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the stored sample height.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Returns the stored border size, or zero for versions 1 and 2.
    #[must_use]
    pub const fn border_size(&self) -> u32 {
        self.border_size
    }

    /// Returns stored version-4 boundaries or the single boundary derived for older versions.
    #[must_use]
    pub fn boundaries(&self) -> &[MapBoundary] {
        &self.boundaries
    }

    /// Returns row-major, one-byte source height samples without renderer scaling.
    #[must_use]
    pub fn samples(&self) -> &[u8] {
        &self.samples
    }

    /// Returns the source grid spacing established for this format version.
    #[must_use]
    pub const fn cell_size_world_units(&self) -> u8 {
        if self.version == 1 { 5 } else { 10 }
    }
}

/// A structured MAP inventory failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapError {
    /// A bounded binary read or resource limit failed.
    Binary(BinaryError),
    /// An `EAR\0` `RefPack` wrapper was malformed.
    RefPack(RefPackError),
    /// The four-byte file signature was not `CkMp`.
    InvalidMagic([u8; 4]),
    /// The symbol table is empty and cannot resolve chunk identifiers.
    EmptySymbolTable,
    /// A signed format field contained a negative value.
    NegativeValue { field: &'static str, value: i32 },
    /// A chunk stream ended with fewer than ten header bytes.
    TruncatedChunkHeader { offset: usize, remaining: usize },
    /// A chunk payload length exceeded the bytes left in the file.
    ChunkPayloadOutOfBounds {
        index: usize,
        offset: usize,
        id: u32,
        version: u16,
        declared: usize,
        remaining: usize,
    },
}

impl Display for MapError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binary(error) => Display::fmt(error, formatter),
            Self::RefPack(error) => Display::fmt(error, formatter),
            Self::InvalidMagic(actual) => write!(
                formatter,
                "invalid MAP signature {actual:02X?}; expected CkMp"
            ),
            Self::EmptySymbolTable => formatter.write_str("MAP symbol table is empty"),
            Self::NegativeValue { field, value } => {
                write!(formatter, "MAP {field} is negative: {value}")
            }
            Self::TruncatedChunkHeader { offset, remaining } => write!(
                formatter,
                "MAP chunk header at offset {offset} has only {remaining} of {CHUNK_HEADER_BYTES} bytes"
            ),
            Self::ChunkPayloadOutOfBounds {
                index,
                offset,
                id,
                version,
                declared,
                remaining,
            } => write!(
                formatter,
                "MAP chunk {index} at offset {offset} (id 0x{id:08X}, version {version}) declares {declared} payload bytes but only {remaining} remain"
            ),
        }
    }
}

impl Error for MapError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Binary(error) => Some(error),
            Self::RefPack(error) => Some(error),
            Self::InvalidMagic(_)
            | Self::EmptySymbolTable
            | Self::NegativeValue { .. }
            | Self::TruncatedChunkHeader { .. }
            | Self::ChunkPayloadOutOfBounds { .. } => None,
        }
    }
}

impl From<BinaryError> for MapError {
    fn from(error: BinaryError) -> Self {
        Self::Binary(error)
    }
}

impl From<RefPackError> for MapError {
    fn from(error: RefPackError) -> Self {
        Self::RefPack(error)
    }
}

/// A structured `HeightMapData` semantic failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapHeightError {
    /// A bounded binary read or resource limit failed.
    Binary(BinaryError),
    /// No top-level height chunk exists.
    MissingHeightMap,
    /// More than one top-level height chunk exists.
    DuplicateHeightMap,
    /// The height chunk version is not established by the pinned source.
    UnsupportedVersion(u16),
    /// A height field required to be nonnegative contained a negative value.
    NegativeValue { field: &'static str, value: i32 },
    /// A width or height was zero.
    ZeroDimension { field: &'static str },
    /// Width multiplied by height overflowed the host address range.
    SampleCountOverflow,
    /// The border cannot fit within the stored dimensions.
    InvalidBorder {
        border: u32,
        width: u32,
        height: u32,
    },
    /// The declared sample count did not equal width times height.
    SampleCountMismatch { declared: usize, expected: usize },
    /// Bytes remained after the exact versioned height layout.
    TrailingBytes(usize),
}

impl Display for MapHeightError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binary(error) => Display::fmt(error, formatter),
            Self::MissingHeightMap => formatter.write_str("MAP has no HeightMapData chunk"),
            Self::DuplicateHeightMap => {
                formatter.write_str("MAP has more than one HeightMapData chunk")
            }
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported HeightMapData version {version}")
            }
            Self::NegativeValue { field, value } => {
                write!(formatter, "HeightMapData {field} is negative: {value}")
            }
            Self::ZeroDimension { field } => {
                write!(formatter, "HeightMapData {field} is zero")
            }
            Self::SampleCountOverflow => {
                formatter.write_str("HeightMapData width times height overflowed")
            }
            Self::InvalidBorder {
                border,
                width,
                height,
            } => write!(
                formatter,
                "HeightMapData border {border} does not fit dimensions {width}x{height}"
            ),
            Self::SampleCountMismatch { declared, expected } => write!(
                formatter,
                "HeightMapData declares {declared} samples but dimensions require {expected}"
            ),
            Self::TrailingBytes(count) => {
                write!(formatter, "HeightMapData has {count} trailing bytes")
            }
        }
    }
}

impl Error for MapHeightError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Binary(error) => Some(error),
            _ => None,
        }
    }
}

impl From<BinaryError> for MapHeightError {
    fn from(error: BinaryError) -> Self {
        Self::Binary(error)
    }
}

/// Inventories a complete `CkMp` file and preserves every top-level chunk payload.
///
/// # Errors
///
/// Returns [`MapError`] for an invalid signature, empty table, negative length, truncation,
/// or any configured file, symbol, name, chunk-count, or payload limit excess.
pub fn parse_map(
    bytes: &[u8],
    source: impl Into<String>,
    limits: MapLimits,
) -> Result<MapFile, MapError> {
    enforce_limit("MAP file size", bytes.len(), limits.maximum_file_bytes)?;
    let source = source.into();
    let (compression, decoded) = if bytes.starts_with(b"EAR\0") {
        (
            MapCompression::RefPack,
            Cow::Owned(decompress_ear(
                bytes,
                &source,
                limits.maximum_decompressed_bytes,
            )?),
        )
    } else {
        (MapCompression::None, Cow::Borrowed(bytes))
    };
    let mut reader = BinaryReader::new(decoded.as_ref(), source);
    let mut actual_magic = [0_u8; 4];
    actual_magic.copy_from_slice(reader.read_exact(4)?);
    if &actual_magic != MAGIC {
        return Err(MapError::InvalidMagic(actual_magic));
    }

    let symbol_count = read_nonnegative(&mut reader, "symbol count")?;
    enforce_limit("MAP symbol count", symbol_count, limits.maximum_symbols)?;
    if symbol_count == 0 {
        return Err(MapError::EmptySymbolTable);
    }
    let mut symbols = Vec::with_capacity(symbol_count);
    for _ in 0..symbol_count {
        let offset = reader.position();
        let name_length = usize::from(reader.read_u8()?);
        enforce_limit(
            "MAP symbol name length",
            name_length,
            limits.maximum_symbol_bytes,
        )?;
        let name = reader.read_exact(name_length)?.to_vec();
        let id = reader.read_u32_le()?;
        symbols.push(MapSymbol { offset, id, name });
    }

    let mut chunks = Vec::new();
    while reader.remaining() != 0 {
        if reader.remaining() < CHUNK_HEADER_BYTES {
            return Err(MapError::TruncatedChunkHeader {
                offset: reader.position(),
                remaining: reader.remaining(),
            });
        }
        let offset = reader.position();
        let id = reader.read_u32_le()?;
        let version = reader.read_u16_le()?;
        let data_length = read_nonnegative(&mut reader, "chunk payload length")?;
        if data_length > reader.remaining() {
            return Err(MapError::ChunkPayloadOutOfBounds {
                index: chunks.len(),
                offset,
                id,
                version,
                declared: data_length,
                remaining: reader.remaining(),
            });
        }
        enforce_limit(
            "MAP chunk payload length",
            data_length,
            limits.maximum_chunk_bytes,
        )?;
        let following_count = chunks
            .len()
            .checked_add(1)
            .ok_or(BinaryError::LimitExceeded {
                what: "MAP chunk count",
                actual: usize::MAX,
                maximum: limits.maximum_chunks,
            })?;
        enforce_limit("MAP chunk count", following_count, limits.maximum_chunks)?;
        chunks.push(MapChunk {
            offset,
            id,
            version,
            data: reader.read_exact(data_length)?.to_vec(),
        });
    }

    Ok(MapFile {
        compression,
        symbols,
        chunks,
    })
}

/// Decodes the unique top-level `HeightMapData` payload for established versions 1 through 4.
///
/// The inventory retains the original payload. This semantic value retains the stored grid and
/// does not apply the legacy engine's version-1 downsampling compatibility transform.
///
/// # Errors
///
/// Returns [`MapHeightError`] when the height chunk is missing or duplicated, its version is
/// unsupported, a field is invalid, a configured limit is exceeded, or the payload does not
/// close exactly.
pub fn decode_map_height(
    map: &MapFile,
    limits: MapLimits,
) -> Result<MapHeightField, MapHeightError> {
    let chunk = height_chunk(map)?;
    if !(1..=4).contains(&chunk.version) {
        return Err(MapHeightError::UnsupportedVersion(chunk.version));
    }

    let mut reader = BinaryReader::new(
        &chunk.data,
        format!("HeightMapData@{}", chunk.offset + CHUNK_HEADER_BYTES),
    );
    let width = read_height_u32(&mut reader, "width")?;
    let height = read_height_u32(&mut reader, "height")?;
    if width == 0 {
        return Err(MapHeightError::ZeroDimension { field: "width" });
    }
    if height == 0 {
        return Err(MapHeightError::ZeroDimension { field: "height" });
    }
    let width_usize = usize::try_from(width).map_err(|_| BinaryError::LimitExceeded {
        what: "MAP height width",
        actual: usize::MAX,
        maximum: limits.maximum_height_dimension,
    })?;
    let height_usize = usize::try_from(height).map_err(|_| BinaryError::LimitExceeded {
        what: "MAP height height",
        actual: usize::MAX,
        maximum: limits.maximum_height_dimension,
    })?;
    enforce_height_limit(
        "MAP height width",
        width_usize,
        limits.maximum_height_dimension,
    )?;
    enforce_height_limit(
        "MAP height height",
        height_usize,
        limits.maximum_height_dimension,
    )?;
    let expected_samples = width_usize
        .checked_mul(height_usize)
        .ok_or(MapHeightError::SampleCountOverflow)?;
    enforce_height_limit(
        "MAP height sample count",
        expected_samples,
        limits.maximum_height_samples,
    )?;

    let border_size = if chunk.version >= 3 {
        read_height_u32(&mut reader, "border size")?
    } else {
        0
    };
    let doubled_border = border_size
        .checked_mul(2)
        .ok_or(MapHeightError::InvalidBorder {
            border: border_size,
            width,
            height,
        })?;
    if doubled_border > width || doubled_border > height {
        return Err(MapHeightError::InvalidBorder {
            border: border_size,
            width,
            height,
        });
    }

    let boundaries = if chunk.version >= 4 {
        let count = read_height_usize(&mut reader, "boundary count")?;
        enforce_height_limit("MAP boundary count", count, limits.maximum_boundaries)?;
        let mut boundaries = Vec::with_capacity(count);
        for _ in 0..count {
            boundaries.push(MapBoundary {
                x: read_height_i32(&mut reader)?,
                y: read_height_i32(&mut reader)?,
            });
        }
        boundaries
    } else {
        vec![MapBoundary {
            x: i32::try_from(width - doubled_border).map_err(|_| height_integer_limit_error())?,
            y: i32::try_from(height - doubled_border).map_err(|_| height_integer_limit_error())?,
        }]
    };

    let declared_samples = read_height_usize(&mut reader, "sample count")?;
    if declared_samples != expected_samples {
        return Err(MapHeightError::SampleCountMismatch {
            declared: declared_samples,
            expected: expected_samples,
        });
    }
    let samples = reader.read_exact(declared_samples)?.to_vec();
    if reader.remaining() != 0 {
        return Err(MapHeightError::TrailingBytes(reader.remaining()));
    }

    Ok(MapHeightField {
        version: chunk.version,
        width,
        height,
        border_size,
        boundaries,
        samples,
    })
}

fn height_chunk(map: &MapFile) -> Result<&MapChunk, MapHeightError> {
    let mut matches = map
        .chunks
        .iter()
        .filter(|chunk| map.symbol_name(chunk.id) == Some(HEIGHT_MAP_LABEL));
    let chunk = matches.next().ok_or(MapHeightError::MissingHeightMap)?;
    if matches.next().is_some() {
        return Err(MapHeightError::DuplicateHeightMap);
    }
    Ok(chunk)
}

fn read_nonnegative(reader: &mut BinaryReader<'_>, field: &'static str) -> Result<usize, MapError> {
    let value = i32::from_le_bytes(reader.read_u32_le()?.to_le_bytes());
    usize::try_from(value).map_err(|_| MapError::NegativeValue { field, value })
}

fn read_height_u32(
    reader: &mut BinaryReader<'_>,
    field: &'static str,
) -> Result<u32, MapHeightError> {
    let value = i32::from_le_bytes(reader.read_u32_le()?.to_le_bytes());
    u32::try_from(value).map_err(|_| MapHeightError::NegativeValue { field, value })
}

fn read_height_i32(reader: &mut BinaryReader<'_>) -> Result<i32, MapHeightError> {
    Ok(i32::from_le_bytes(reader.read_u32_le()?.to_le_bytes()))
}

fn height_integer_limit_error() -> MapHeightError {
    MapHeightError::Binary(BinaryError::LimitExceeded {
        what: "MAP height integer",
        actual: usize::MAX,
        maximum: i32::MAX as usize,
    })
}

fn read_height_usize(
    reader: &mut BinaryReader<'_>,
    field: &'static str,
) -> Result<usize, MapHeightError> {
    let value = read_height_u32(reader, field)?;
    usize::try_from(value).map_err(|_| {
        MapHeightError::Binary(BinaryError::LimitExceeded {
            what: "MAP height integer",
            actual: usize::MAX,
            maximum: usize::MAX,
        })
    })
}

fn enforce_limit(what: &'static str, actual: usize, maximum: usize) -> Result<(), MapError> {
    if actual > maximum {
        Err(MapError::Binary(BinaryError::LimitExceeded {
            what,
            actual,
            maximum,
        }))
    } else {
        Ok(())
    }
}

fn enforce_height_limit(
    what: &'static str,
    actual: usize,
    maximum: usize,
) -> Result<(), MapHeightError> {
    if actual > maximum {
        Err(MapHeightError::Binary(BinaryError::LimitExceeded {
            what,
            actual,
            maximum,
        }))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use cic_core::BinaryError;

    use super::{MapError, MapHeightError, MapLimits, decode_map_height, parse_map};

    fn fixture() -> Vec<u8> {
        let hex = include_str!("../tests/fixtures/minimal.map.hex");
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
    fn inventories_symbols_and_opaque_chunks_in_file_order() {
        let map = parse_map(&fixture(), "minimal.map", MapLimits::default()).expect("valid MAP");

        assert_eq!(map.symbols().len(), 2);
        assert_eq!(map.symbols()[0].offset(), 8);
        assert_eq!(map.symbols()[0].id(), 7);
        assert_eq!(map.symbols()[0].name_bytes(), b"HeightMapData");
        assert_eq!(map.symbols()[1].offset(), 26);
        assert_eq!(map.symbol_name(9), Some(b"Mystery".as_slice()));
        assert_eq!(map.symbol_name(0xFEED_BEEF), None);

        assert_eq!(map.chunks().len(), 3);
        assert_eq!(map.chunks()[0].offset(), 38);
        assert_eq!(map.chunks()[0].version(), 4);
        assert_eq!(map.chunks()[0].data().len(), 34);
        assert_eq!(map.chunks()[1].data(), b"xyz");
        assert_eq!(map.chunks()[2].data(), [0xAA, 0xBB]);
    }

    #[test]
    fn decodes_version_four_height_samples_and_boundaries() {
        let map = parse_map(&fixture(), "minimal.map", MapLimits::default()).expect("valid MAP");
        let height = decode_map_height(&map, MapLimits::default()).expect("valid height field");

        assert_eq!(height.version(), 4);
        assert_eq!((height.width(), height.height()), (3, 2));
        assert_eq!(height.border_size(), 0);
        assert_eq!(height.boundaries().len(), 1);
        assert_eq!(
            (height.boundaries()[0].x(), height.boundaries()[0].y()),
            (3, 2)
        );
        assert_eq!(height.samples(), [0, 16, 32, 48, 64, 255]);
        assert_eq!(height.cell_size_world_units(), 10);
    }

    #[test]
    fn preserves_signed_version_four_boundaries() {
        let mut bytes = fixture();
        bytes[64..68].copy_from_slice(&(-6_i32).to_le_bytes());
        bytes[68..72].copy_from_slice(&(-9_i32).to_le_bytes());
        let map = parse_map(&bytes, "signed-boundary.map", MapLimits::default())
            .expect("valid inventory");
        let height = decode_map_height(&map, MapLimits::default()).expect("signed boundary");
        assert_eq!(
            (height.boundaries()[0].x(), height.boundaries()[0].y()),
            (-6, -9)
        );
    }

    #[test]
    fn decompresses_ear_wrapped_map_before_inventory() {
        let uncompressed = fixture();
        assert_eq!(uncompressed.len(), 107);
        let mut wrapped = b"EAR\0".to_vec();
        wrapped.extend_from_slice(&107_i32.to_le_bytes());
        wrapped.extend_from_slice(&[0x10, 0xFB, 0, 0, 107, 0xF9]);
        wrapped.extend_from_slice(&uncompressed[..104]);
        wrapped.push(0xFF);
        wrapped.extend_from_slice(&uncompressed[104..]);

        let map = parse_map(&wrapped, "compressed.map", MapLimits::default())
            .expect("valid compressed MAP");
        assert_eq!(map.compression(), super::MapCompression::RefPack);
        assert_eq!(map.chunks().len(), 3);
        assert_eq!(
            decode_map_height(&map, MapLimits::default())
                .expect("valid height field")
                .samples(),
            [0, 16, 32, 48, 64, 255]
        );
    }

    #[test]
    fn dispatches_established_height_versions() {
        for version in 1_u16..=4 {
            let mut bytes = fixture();
            bytes[42..44].copy_from_slice(&version.to_le_bytes());
            let payload = match version {
                1 | 2 => {
                    let mut payload = Vec::new();
                    payload.extend_from_slice(&3_i32.to_le_bytes());
                    payload.extend_from_slice(&2_i32.to_le_bytes());
                    payload.extend_from_slice(&6_i32.to_le_bytes());
                    payload.extend_from_slice(&[0, 16, 32, 48, 64, 255]);
                    payload
                }
                3 => {
                    let mut payload = Vec::new();
                    payload.extend_from_slice(&3_i32.to_le_bytes());
                    payload.extend_from_slice(&2_i32.to_le_bytes());
                    payload.extend_from_slice(&0_i32.to_le_bytes());
                    payload.extend_from_slice(&6_i32.to_le_bytes());
                    payload.extend_from_slice(&[0, 16, 32, 48, 64, 255]);
                    payload
                }
                4 => bytes[48..82].to_vec(),
                _ => unreachable!(),
            };
            bytes.splice(48..82, payload.iter().copied());
            bytes[44..48].copy_from_slice(
                &i32::try_from(payload.len())
                    .expect("fixture payload length")
                    .to_le_bytes(),
            );
            let map = parse_map(&bytes, "versions.map", MapLimits::default()).expect("valid MAP");
            let height = decode_map_height(&map, MapLimits::default()).expect("known version");
            assert_eq!(height.version(), version);
            assert_eq!(
                height.cell_size_world_units(),
                if version == 1 { 5 } else { 10 }
            );
        }
    }

    #[test]
    fn every_truncated_prefix_returns_an_error() {
        let bytes = fixture();
        for length in 0..bytes.len() {
            if [38, 82, 95].contains(&length) {
                continue;
            }
            assert!(
                parse_map(&bytes[..length], "truncated.map", MapLimits::default()).is_err(),
                "prefix of {length} bytes unexpectedly parsed"
            );
        }
    }

    #[test]
    fn rejects_bad_magic_negative_lengths_and_limits() {
        let mut bad_magic = fixture();
        bad_magic[0] = b'X';
        assert!(matches!(
            parse_map(&bad_magic, "magic.map", MapLimits::default()),
            Err(MapError::InvalidMagic(_))
        ));

        let mut negative_length = fixture();
        negative_length[44..48].copy_from_slice(&(-1_i32).to_le_bytes());
        assert!(matches!(
            parse_map(&negative_length, "negative.map", MapLimits::default()),
            Err(MapError::NegativeValue {
                field: "chunk payload length",
                value: -1
            })
        ));

        assert!(matches!(
            parse_map(
                &fixture(),
                "limit.map",
                MapLimits {
                    maximum_symbols: 1,
                    ..MapLimits::default()
                }
            ),
            Err(MapError::Binary(BinaryError::LimitExceeded {
                what: "MAP symbol count",
                ..
            }))
        ));
    }

    #[test]
    fn rejects_invalid_height_semantics() {
        let mut unsupported = fixture();
        unsupported[42..44].copy_from_slice(&5_u16.to_le_bytes());
        let map = parse_map(&unsupported, "unsupported.map", MapLimits::default())
            .expect("valid inventory");
        assert_eq!(
            decode_map_height(&map, MapLimits::default()),
            Err(MapHeightError::UnsupportedVersion(5))
        );

        let mut bad_count = fixture();
        bad_count[72..76].copy_from_slice(&5_i32.to_le_bytes());
        let map =
            parse_map(&bad_count, "count.map", MapLimits::default()).expect("valid inventory");
        assert_eq!(
            decode_map_height(&map, MapLimits::default()),
            Err(MapHeightError::SampleCountMismatch {
                declared: 5,
                expected: 6
            })
        );
    }
}
