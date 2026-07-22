//! Bounded `BlendTileData` version-6 and version-7 decoding.
//!
//! The field order and compatibility behavior are derived from `WorldHeightMap.cpp`,
//! `WHeightMapEdit.cpp`, `WorldHeightMap.h`, and `TileData.h` in `GeneralsGameCode` revision
//! `9f7abb866f5afd446db14149979e744c7216baaf`, licensed under GPL-3.0-or-later with
//! Electronic Arts Section 7 terms. Full notices are in `docs/provenance/map.md`.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

use cic_core::{BinaryError, BinaryReader};

use crate::{MapChunk, MapFile, MapHeightField, MapLimits};

const BLEND_TILE_LABEL: &[u8] = b"BlendTileData";
const MINIMUM_BLEND_TILE_VERSION: u16 = 6;
const MAXIMUM_BLEND_TILE_VERSION: u16 = 7;
const RECORD_FLAG: u32 = 0x7ADA_0000;
const DERIVED_CLIFF_HEIGHT_DELTA: u8 = 16;

/// One terrain or edge texture-class descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapTextureClass {
    first_tile: u32,
    tile_count: u32,
    width: u32,
    legacy: Option<i32>,
    name: Vec<u8>,
}

impl MapTextureClass {
    /// Returns the first tile index owned by this class.
    #[must_use]
    pub const fn first_tile(&self) -> u32 {
        self.first_tile
    }

    /// Returns the number of tiles owned by this class.
    #[must_use]
    pub const fn tile_count(&self) -> u32 {
        self.tile_count
    }

    /// Returns the source texture grid width.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the ignored legacy field, absent from edge texture classes.
    #[must_use]
    pub const fn legacy(&self) -> Option<i32> {
        self.legacy
    }

    /// Returns the losslessly preserved class-name bytes.
    #[must_use]
    pub fn name_bytes(&self) -> &[u8] {
        &self.name
    }
}

/// One file-stored blend descriptor. Table index zero is implicit and is not included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapBlendTile {
    table_index: u32,
    blend_index: i32,
    horizontal: u8,
    vertical: u8,
    right_diagonal: u8,
    left_diagonal: u8,
    inverted: u8,
    long_diagonal: u8,
    custom_edge_class: i32,
}

impl MapBlendTile {
    /// Returns this record's one-based table index.
    #[must_use]
    pub const fn table_index(self) -> u32 {
        self.table_index
    }

    /// Returns the source blend index.
    #[must_use]
    pub const fn blend_index(self) -> i32 {
        self.blend_index
    }

    /// Returns the horizontal selector byte.
    #[must_use]
    pub const fn horizontal(self) -> u8 {
        self.horizontal
    }

    /// Returns the vertical selector byte.
    #[must_use]
    pub const fn vertical(self) -> u8 {
        self.vertical
    }

    /// Returns the right-diagonal selector byte.
    #[must_use]
    pub const fn right_diagonal(self) -> u8 {
        self.right_diagonal
    }

    /// Returns the left-diagonal selector byte.
    #[must_use]
    pub const fn left_diagonal(self) -> u8 {
        self.left_diagonal
    }

    /// Returns the raw inversion and forced-flip mask byte.
    #[must_use]
    pub const fn inverted(self) -> u8 {
        self.inverted
    }

    /// Returns the long-diagonal selector byte.
    #[must_use]
    pub const fn long_diagonal(self) -> u8 {
        self.long_diagonal
    }

    /// Returns the custom edge texture-class index, where negative values are source sentinels.
    #[must_use]
    pub const fn custom_edge_class(self) -> i32 {
        self.custom_edge_class
    }
}

/// One file-stored cliff UV descriptor. Table index zero is implicit and is not included.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapCliffInfo {
    table_index: u32,
    tile_index: i32,
    uv: [f32; 8],
    flip: u8,
    mutant: u8,
}

impl MapCliffInfo {
    /// Returns this record's one-based table index.
    #[must_use]
    pub const fn table_index(self) -> u32 {
        self.table_index
    }

    /// Returns the source tile index.
    #[must_use]
    pub const fn tile_index(self) -> i32 {
        self.tile_index
    }

    /// Returns `(u0, v0, ... u3, v3)` in source order.
    #[must_use]
    pub const fn uv(self) -> [f32; 8] {
        self.uv
    }

    /// Returns the raw flip byte.
    #[must_use]
    pub const fn flip(self) -> u8 {
        self.flip
    }

    /// Returns the raw mutant-mapping byte.
    #[must_use]
    pub const fn mutant(self) -> u8 {
        self.mutant
    }
}

/// Immutable, renderer-neutral terrain blend and cliff values.
#[derive(Debug, Clone, PartialEq)]
pub struct MapBlendData {
    version: u16,
    width: u32,
    height: u32,
    tile_indices: Vec<i16>,
    blend_indices: Vec<i16>,
    extra_blend_indices: Vec<i16>,
    cliff_info_indices: Vec<i16>,
    cliff_flags: Vec<u8>,
    cliff_flag_stride: usize,
    bitmap_tile_count: u32,
    blended_tile_count: u32,
    cliff_info_count: u32,
    texture_classes: Vec<MapTextureClass>,
    edge_tile_count: u32,
    edge_texture_classes: Vec<MapTextureClass>,
    blend_tiles: Vec<MapBlendTile>,
    cliff_info: Vec<MapCliffInfo>,
}

impl MapBlendData {
    /// Returns the source chunk version.
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }

    /// Returns the height-grid width associated with these cells.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the height-grid height associated with these cells.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Returns the primary tile index plane in row-major order.
    #[must_use]
    pub fn tile_indices(&self) -> &[i16] {
        &self.tile_indices
    }

    /// Returns the blend index plane in row-major order.
    #[must_use]
    pub fn blend_indices(&self) -> &[i16] {
        &self.blend_indices
    }

    /// Returns the extra three-way blend index plane in row-major order.
    #[must_use]
    pub fn extra_blend_indices(&self) -> &[i16] {
        &self.extra_blend_indices
    }

    /// Returns the cliff-info index plane in row-major order.
    #[must_use]
    pub fn cliff_info_indices(&self) -> &[i16] {
        &self.cliff_info_indices
    }

    /// Returns the normalized cliff bitmap stride in bytes.
    #[must_use]
    pub const fn cliff_flag_stride(&self) -> usize {
        self.cliff_flag_stride
    }

    /// Returns the normalized row-major cliff bitmap.
    #[must_use]
    pub fn cliff_flags(&self) -> &[u8] {
        &self.cliff_flags
    }

    /// Returns whether a checked cell is marked as a cliff.
    #[must_use]
    pub fn is_cliff(&self, x: u32, y: u32) -> Option<bool> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let x = usize::try_from(x).ok()?;
        let y = usize::try_from(y).ok()?;
        let byte = self
            .cliff_flags
            .get(y.checked_mul(self.cliff_flag_stride)?.checked_add(x / 8)?)?;
        Some(byte & (1 << (x & 7)) != 0)
    }

    /// Returns the declared bitmap-tile count.
    #[must_use]
    pub const fn bitmap_tile_count(&self) -> u32 {
        self.bitmap_tile_count
    }

    /// Returns the declared blend table count, including implicit entry zero.
    #[must_use]
    pub const fn blended_tile_count(&self) -> u32 {
        self.blended_tile_count
    }

    /// Returns the declared cliff table count, including implicit entry zero.
    #[must_use]
    pub const fn cliff_info_count(&self) -> u32 {
        self.cliff_info_count
    }

    /// Returns terrain texture classes in file order.
    #[must_use]
    pub fn texture_classes(&self) -> &[MapTextureClass] {
        &self.texture_classes
    }

    /// Returns the declared edge-tile count.
    #[must_use]
    pub const fn edge_tile_count(&self) -> u32 {
        self.edge_tile_count
    }

    /// Returns edge/shore texture classes in file order.
    #[must_use]
    pub fn edge_texture_classes(&self) -> &[MapTextureClass] {
        &self.edge_texture_classes
    }

    /// Returns file-stored blend descriptors for table entries one onward.
    #[must_use]
    pub fn blend_tiles(&self) -> &[MapBlendTile] {
        &self.blend_tiles
    }

    /// Returns file-stored cliff descriptors for table entries one onward.
    #[must_use]
    pub fn cliff_info(&self) -> &[MapCliffInfo] {
        &self.cliff_info
    }
}

/// A structured `BlendTileData` semantic failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapBlendError {
    /// A bounded binary read or resource limit failed.
    Binary(BinaryError),
    /// No top-level blend chunk exists.
    MissingBlendData,
    /// More than one top-level blend chunk exists.
    DuplicateBlendData,
    /// The blend chunk version is not implemented by this gate.
    UnsupportedVersion(u16),
    /// A signed field contained a negative value.
    NegativeValue { field: &'static str, value: i32 },
    /// A required declared table count was zero.
    ZeroCount(&'static str),
    /// The cell count did not equal the decoded height sample count.
    CellCountMismatch { declared: usize, expected: usize },
    /// Checked size arithmetic overflowed.
    SizeOverflow(&'static str),
    /// A texture class addressed beyond its declared tile table.
    TextureRange {
        edge: bool,
        index: usize,
        first: u32,
        count: u32,
        available: u32,
    },
    /// A blend record did not contain the required source sentinel.
    InvalidBlendFlag { index: u32, actual: u32 },
    /// A cliff UV component was not finite.
    NonFiniteCliffUv {
        index: u32,
        component: usize,
        bits: u32,
    },
    /// Bytes remained after the exact versioned layout.
    TrailingBytes(usize),
}

impl Display for MapBlendError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binary(error) => Display::fmt(error, formatter),
            Self::MissingBlendData => formatter.write_str("MAP has no BlendTileData chunk"),
            Self::DuplicateBlendData => {
                formatter.write_str("MAP has more than one BlendTileData chunk")
            }
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported BlendTileData version {version}")
            }
            Self::NegativeValue { field, value } => {
                write!(formatter, "BlendTileData {field} is negative: {value}")
            }
            Self::ZeroCount(field) => write!(formatter, "BlendTileData {field} is zero"),
            Self::CellCountMismatch { declared, expected } => write!(
                formatter,
                "BlendTileData declares {declared} cells but height data requires {expected}"
            ),
            Self::SizeOverflow(field) => write!(formatter, "BlendTileData {field} size overflowed"),
            Self::TextureRange {
                edge,
                index,
                first,
                count,
                available,
            } => write!(
                formatter,
                "BlendTileData {} texture class {index} range {first}+{count} exceeds {available} tiles",
                if *edge { "edge" } else { "terrain" }
            ),
            Self::InvalidBlendFlag { index, actual } => write!(
                formatter,
                "BlendTileData blend record {index} has flag 0x{actual:08X}; expected 0x{RECORD_FLAG:08X}"
            ),
            Self::NonFiniteCliffUv {
                index,
                component,
                bits,
            } => write!(
                formatter,
                "BlendTileData cliff record {index} UV component {component} is non-finite (0x{bits:08X})"
            ),
            Self::TrailingBytes(count) => {
                write!(formatter, "BlendTileData has {count} trailing bytes")
            }
        }
    }
}

impl Error for MapBlendError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Binary(error) => Some(error),
            _ => None,
        }
    }
}

impl From<BinaryError> for MapBlendError {
    fn from(error: BinaryError) -> Self {
        Self::Binary(error)
    }
}

/// Decodes the unique version-6 or version-7 `BlendTileData` payload against its height grid.
///
/// # Errors
///
/// Returns [`MapBlendError`] for a missing or duplicate chunk, unsupported version, inconsistent
/// cell count, invalid count/range/sentinel/float, limit excess, truncation, or trailing bytes.
pub fn decode_map_blend(
    map: &MapFile,
    height: &MapHeightField,
    limits: MapLimits,
) -> Result<MapBlendData, MapBlendError> {
    let chunk = blend_chunk(map)?;
    if !(MINIMUM_BLEND_TILE_VERSION..=MAXIMUM_BLEND_TILE_VERSION).contains(&chunk.version()) {
        return Err(MapBlendError::UnsupportedVersion(chunk.version()));
    }
    let mut reader = BinaryReader::new(
        chunk.data(),
        format!("BlendTileData@{}", chunk.offset() + 10),
    );
    let cell_count = read_usize(&mut reader, "cell count")?;
    let expected_cells = height.samples().len();
    if cell_count != expected_cells {
        return Err(MapBlendError::CellCountMismatch {
            declared: cell_count,
            expected: expected_cells,
        });
    }

    let tile_indices = read_i16_plane(&mut reader, cell_count)?;
    let blend_indices = read_i16_plane(&mut reader, cell_count)?;
    let extra_blend_indices = read_i16_plane(&mut reader, cell_count)?;
    let cliff_info_indices = read_i16_plane(&mut reader, cell_count)?;
    let (cliff_flags, cliff_flag_stride) = if chunk.version() == 7 {
        read_version_seven_cliff_flags(&mut reader, height.width(), height.height())?
    } else {
        derive_version_six_cliff_flags(height)?
    };

    let bitmap_tile_count = read_limited_count(
        &mut reader,
        "bitmap tile count",
        limits.maximum_bitmap_tiles,
        true,
    )?;
    let blended_tile_count = read_limited_count(
        &mut reader,
        "blended tile count",
        limits.maximum_blended_tiles,
        true,
    )?;
    let cliff_info_count = read_limited_count(
        &mut reader,
        "cliff info count",
        limits.maximum_cliff_records,
        false,
    )?;
    let texture_class_count = read_limited_count(
        &mut reader,
        "texture class count",
        limits.maximum_texture_classes,
        true,
    )?;
    let texture_classes = read_texture_classes(
        &mut reader,
        texture_class_count,
        bitmap_tile_count,
        false,
        limits,
    )?;

    let edge_tile_count = read_limited_count(
        &mut reader,
        "edge tile count",
        limits.maximum_edge_tiles,
        false,
    )?;
    let edge_texture_class_count = read_limited_count(
        &mut reader,
        "edge texture class count",
        limits.maximum_texture_classes,
        false,
    )?;
    let edge_texture_classes = read_texture_classes(
        &mut reader,
        edge_texture_class_count,
        edge_tile_count,
        true,
        limits,
    )?;
    let blend_tiles = read_blend_tiles(&mut reader, blended_tile_count)?;
    let cliff_info = read_cliff_info(&mut reader, cliff_info_count)?;
    if reader.remaining() != 0 {
        return Err(MapBlendError::TrailingBytes(reader.remaining()));
    }

    Ok(MapBlendData {
        version: chunk.version(),
        width: height.width(),
        height: height.height(),
        tile_indices,
        blend_indices,
        extra_blend_indices,
        cliff_info_indices,
        cliff_flags,
        cliff_flag_stride,
        bitmap_tile_count,
        blended_tile_count,
        cliff_info_count,
        texture_classes,
        edge_tile_count,
        edge_texture_classes,
        blend_tiles,
        cliff_info,
    })
}

fn blend_chunk(map: &MapFile) -> Result<&MapChunk, MapBlendError> {
    let mut matches = map
        .chunks()
        .iter()
        .filter(|chunk| map.symbol_name(chunk.id()) == Some(BLEND_TILE_LABEL));
    let chunk = matches.next().ok_or(MapBlendError::MissingBlendData)?;
    if matches.next().is_some() {
        return Err(MapBlendError::DuplicateBlendData);
    }
    Ok(chunk)
}

fn read_i16_plane(reader: &mut BinaryReader<'_>, count: usize) -> Result<Vec<i16>, MapBlendError> {
    let byte_count = count
        .checked_mul(2)
        .ok_or(MapBlendError::SizeOverflow("index plane"))?;
    let mut plane = Vec::with_capacity(count);
    let mut region = reader.read_region(byte_count)?;
    for _ in 0..count {
        plane.push(i16::from_le_bytes(region.read_u16_le()?.to_le_bytes()));
    }
    Ok(plane)
}

fn read_version_seven_cliff_flags(
    reader: &mut BinaryReader<'_>,
    width: u32,
    height: u32,
) -> Result<(Vec<u8>, usize), MapBlendError> {
    let width = usize::try_from(width).map_err(|_| MapBlendError::SizeOverflow("cliff width"))?;
    let height =
        usize::try_from(height).map_err(|_| MapBlendError::SizeOverflow("cliff height"))?;
    let stored_stride = width
        .checked_add(1)
        .ok_or(MapBlendError::SizeOverflow("stored cliff stride"))?
        / 8;
    let normalized_stride = width
        .checked_add(7)
        .ok_or(MapBlendError::SizeOverflow("cliff stride"))?
        / 8;
    let stored_length = stored_stride
        .checked_mul(height)
        .ok_or(MapBlendError::SizeOverflow("stored cliff bitmap"))?;
    let normalized_length = normalized_stride
        .checked_mul(height)
        .ok_or(MapBlendError::SizeOverflow("cliff bitmap"))?;
    let stored = reader.read_exact(stored_length)?;
    let mut normalized = vec![0_u8; normalized_length];
    if stored_stride == 0 {
        return Ok((normalized, normalized_stride));
    }
    for (source_row, target_row) in stored
        .chunks_exact(stored_stride)
        .zip(normalized.chunks_exact_mut(normalized_stride))
    {
        let target = target_row
            .get_mut(..stored_stride)
            .ok_or(MapBlendError::SizeOverflow("normalized cliff row"))?;
        target.copy_from_slice(source_row);
    }
    Ok((normalized, normalized_stride))
}

fn derive_version_six_cliff_flags(
    height: &MapHeightField,
) -> Result<(Vec<u8>, usize), MapBlendError> {
    let width =
        usize::try_from(height.width()).map_err(|_| MapBlendError::SizeOverflow("cliff width"))?;
    let height_count = usize::try_from(height.height())
        .map_err(|_| MapBlendError::SizeOverflow("cliff height"))?;
    let stride = width
        .checked_add(7)
        .ok_or(MapBlendError::SizeOverflow("cliff stride"))?
        / 8;
    let length = stride
        .checked_mul(height_count)
        .ok_or(MapBlendError::SizeOverflow("cliff bitmap"))?;
    let mut flags = vec![0_u8; length];

    for y in 0..height_count.saturating_sub(1) {
        let row = y
            .checked_mul(width)
            .ok_or(MapBlendError::SizeOverflow("cliff row"))?;
        let next_row = row
            .checked_add(width)
            .ok_or(MapBlendError::SizeOverflow("next cliff row"))?;
        for x in 0..width.saturating_sub(1) {
            let next_x = x
                .checked_add(1)
                .ok_or(MapBlendError::SizeOverflow("next cliff column"))?;
            let samples = height.samples();
            let values = [
                *samples
                    .get(
                        row.checked_add(x)
                            .ok_or(MapBlendError::SizeOverflow("cliff sample index"))?,
                    )
                    .ok_or(MapBlendError::SizeOverflow("cliff sample"))?,
                *samples
                    .get(
                        row.checked_add(next_x)
                            .ok_or(MapBlendError::SizeOverflow("cliff sample index"))?,
                    )
                    .ok_or(MapBlendError::SizeOverflow("cliff sample"))?,
                *samples
                    .get(
                        next_row
                            .checked_add(x)
                            .ok_or(MapBlendError::SizeOverflow("cliff sample index"))?,
                    )
                    .ok_or(MapBlendError::SizeOverflow("cliff sample"))?,
                *samples
                    .get(
                        next_row
                            .checked_add(next_x)
                            .ok_or(MapBlendError::SizeOverflow("cliff sample index"))?,
                    )
                    .ok_or(MapBlendError::SizeOverflow("cliff sample"))?,
            ];
            let minimum = values
                .iter()
                .copied()
                .min()
                .ok_or(MapBlendError::SizeOverflow("cliff minimum"))?;
            let maximum = values
                .iter()
                .copied()
                .max()
                .ok_or(MapBlendError::SizeOverflow("cliff maximum"))?;
            if maximum - minimum >= DERIVED_CLIFF_HEIGHT_DELTA {
                let byte_index = y
                    .checked_mul(stride)
                    .and_then(|offset| offset.checked_add(x / 8))
                    .ok_or(MapBlendError::SizeOverflow("cliff flag index"))?;
                let flag = flags
                    .get_mut(byte_index)
                    .ok_or(MapBlendError::SizeOverflow("cliff flag"))?;
                *flag |= 1 << (x & 7);
            }
        }
    }
    Ok((flags, stride))
}

fn read_texture_classes(
    reader: &mut BinaryReader<'_>,
    count: u32,
    available_tiles: u32,
    edge: bool,
    limits: MapLimits,
) -> Result<Vec<MapTextureClass>, MapBlendError> {
    let count =
        usize::try_from(count).map_err(|_| MapBlendError::SizeOverflow("texture class count"))?;
    let mut classes = Vec::with_capacity(count);
    for index in 0..count {
        let first_tile = read_u32(reader, "texture class first tile")?;
        let tile_count = read_u32(reader, "texture class tile count")?;
        let width = read_u32(reader, "texture class width")?;
        if width == 0 {
            return Err(MapBlendError::ZeroCount("texture class width"));
        }
        let legacy = (!edge).then(|| read_i32(reader)).transpose()?;
        let name_length = usize::from(reader.read_u16_le()?);
        enforce_limit(
            "MAP texture class name length",
            name_length,
            limits.maximum_texture_name_bytes,
        )?;
        let name = reader.read_exact(name_length)?.to_vec();
        let end = first_tile
            .checked_add(tile_count)
            .ok_or(MapBlendError::TextureRange {
                edge,
                index,
                first: first_tile,
                count: tile_count,
                available: available_tiles,
            })?;
        if end > available_tiles {
            return Err(MapBlendError::TextureRange {
                edge,
                index,
                first: first_tile,
                count: tile_count,
                available: available_tiles,
            });
        }
        classes.push(MapTextureClass {
            first_tile,
            tile_count,
            width,
            legacy,
            name,
        });
    }
    Ok(classes)
}

fn read_blend_tiles(
    reader: &mut BinaryReader<'_>,
    declared_count: u32,
) -> Result<Vec<MapBlendTile>, MapBlendError> {
    let stored_count = declared_count - 1;
    let capacity = usize::try_from(stored_count)
        .map_err(|_| MapBlendError::SizeOverflow("blend record count"))?;
    let mut records = Vec::with_capacity(capacity);
    for table_index in 1..declared_count {
        let record = MapBlendTile {
            table_index,
            blend_index: read_i32(reader)?,
            horizontal: reader.read_u8()?,
            vertical: reader.read_u8()?,
            right_diagonal: reader.read_u8()?,
            left_diagonal: reader.read_u8()?,
            inverted: reader.read_u8()?,
            long_diagonal: reader.read_u8()?,
            custom_edge_class: read_i32(reader)?,
        };
        let flag = reader.read_u32_le()?;
        if flag != RECORD_FLAG {
            return Err(MapBlendError::InvalidBlendFlag {
                index: table_index,
                actual: flag,
            });
        }
        records.push(record);
    }
    Ok(records)
}

fn read_cliff_info(
    reader: &mut BinaryReader<'_>,
    declared_count: u32,
) -> Result<Vec<MapCliffInfo>, MapBlendError> {
    // The source reader iterates `1..declared_count` and therefore tolerates a historical zero
    // count as an empty table. Runtime index repair then maps every unusable entry to sentinel zero.
    let stored_count = declared_count.saturating_sub(1);
    let capacity = usize::try_from(stored_count)
        .map_err(|_| MapBlendError::SizeOverflow("cliff record count"))?;
    let mut records = Vec::with_capacity(capacity);
    for table_index in 1..declared_count {
        let tile_index = read_i32(reader)?;
        let mut uv = [0_f32; 8];
        for (component, value) in uv.iter_mut().enumerate() {
            let bits = reader.read_u32_le()?;
            *value = f32::from_bits(bits);
            if !value.is_finite() {
                return Err(MapBlendError::NonFiniteCliffUv {
                    index: table_index,
                    component,
                    bits,
                });
            }
        }
        records.push(MapCliffInfo {
            table_index,
            tile_index,
            uv,
            flip: reader.read_u8()?,
            mutant: reader.read_u8()?,
        });
    }
    Ok(records)
}

fn read_limited_count(
    reader: &mut BinaryReader<'_>,
    field: &'static str,
    maximum: usize,
    require_nonzero: bool,
) -> Result<u32, MapBlendError> {
    let value = read_u32(reader, field)?;
    if require_nonzero && value == 0 {
        return Err(MapBlendError::ZeroCount(field));
    }
    let actual = usize::try_from(value).map_err(|_| MapBlendError::SizeOverflow(field))?;
    enforce_limit(field, actual, maximum)?;
    Ok(value)
}

fn read_usize(reader: &mut BinaryReader<'_>, field: &'static str) -> Result<usize, MapBlendError> {
    let value = read_u32(reader, field)?;
    usize::try_from(value).map_err(|_| MapBlendError::SizeOverflow(field))
}

fn read_u32(reader: &mut BinaryReader<'_>, field: &'static str) -> Result<u32, MapBlendError> {
    let value = read_i32(reader)?;
    u32::try_from(value).map_err(|_| MapBlendError::NegativeValue { field, value })
}

fn read_i32(reader: &mut BinaryReader<'_>) -> Result<i32, BinaryError> {
    Ok(i32::from_le_bytes(reader.read_u32_le()?.to_le_bytes()))
}

fn enforce_limit(what: &'static str, actual: usize, maximum: usize) -> Result<(), MapBlendError> {
    if actual > maximum {
        Err(MapBlendError::Binary(BinaryError::LimitExceeded {
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
    use cic_core::{BinaryError, BinaryReader};

    use super::{MapBlendError, RECORD_FLAG, decode_map_blend, read_version_seven_cliff_flags};
    use crate::{MapLimits, decode_map_height, parse_map};

    const BLEND_CHUNK_OFFSET: usize = 98;
    const BLEND_PAYLOAD_OFFSET: usize = 108;
    const BLEND_PAYLOAD_LENGTH: usize = 255;
    const VERSION_SEVEN_CLIFF_OFFSET: usize = BLEND_PAYLOAD_OFFSET + 4 + 16 * 2 * 4;
    const CLIFF_INFO_COUNT_OFFSET: usize = VERSION_SEVEN_CLIFF_OFFSET + 2 + 8;
    const CLIFF_INFO_RECORD_BYTES: usize = 4 + 8 * 4 + 2;

    fn fixture() -> Vec<u8> {
        let hex = include_str!("../tests/fixtures/blend.map.hex");
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

    fn version_six_fixture() -> Vec<u8> {
        let mut bytes = fixture();
        bytes[BLEND_CHUNK_OFFSET + 4..BLEND_CHUNK_OFFSET + 6].copy_from_slice(&6_u16.to_le_bytes());
        bytes.drain(VERSION_SEVEN_CLIFF_OFFSET..VERSION_SEVEN_CLIFF_OFFSET + 2);
        bytes[BLEND_CHUNK_OFFSET + 6..BLEND_CHUNK_OFFSET + 10].copy_from_slice(
            &i32::try_from(BLEND_PAYLOAD_LENGTH - 2)
                .expect("fixture length")
                .to_le_bytes(),
        );
        bytes
    }

    fn decode(bytes: &[u8], limits: MapLimits) -> Result<super::MapBlendData, MapBlendError> {
        let map = parse_map(bytes, "blend.map", limits).expect("valid MAP inventory");
        let height = decode_map_height(&map, limits).expect("valid height field");
        decode_map_blend(&map, &height, limits)
    }

    #[test]
    fn decodes_version_seven_cell_tables_edges_and_cliffs() {
        let blend = decode(&fixture(), MapLimits::default()).expect("valid blend data");

        assert_eq!(blend.version(), 7);
        assert_eq!((blend.width(), blend.height()), (8, 2));
        assert_eq!(
            blend.tile_indices(),
            [0, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3]
        );
        assert_eq!(blend.blend_indices()[5], 1);
        assert_eq!(blend.extra_blend_indices()[6], 1);
        assert_eq!(blend.cliff_info_indices()[0], 1);
        assert_eq!(blend.cliff_flag_stride(), 1);
        assert_eq!(blend.cliff_flags(), [1, 128]);
        assert_eq!(blend.is_cliff(0, 0), Some(true));
        assert_eq!(blend.is_cliff(7, 1), Some(true));
        assert_eq!(blend.is_cliff(8, 0), None);
        assert_eq!(blend.bitmap_tile_count(), 4);
        assert_eq!(blend.blended_tile_count(), 2);
        assert_eq!(blend.cliff_info_count(), 2);

        let terrain = &blend.texture_classes()[0];
        assert_eq!(
            (terrain.first_tile(), terrain.tile_count(), terrain.width()),
            (0, 4, 2)
        );
        assert_eq!(terrain.legacy(), Some(0));
        assert_eq!(terrain.name_bytes(), b"Base");
        assert_eq!(blend.edge_tile_count(), 2);
        let edge = &blend.edge_texture_classes()[0];
        assert_eq!(
            (edge.first_tile(), edge.tile_count(), edge.width()),
            (0, 2, 1)
        );
        assert_eq!(edge.legacy(), None);
        assert_eq!(edge.name_bytes(), b"Shore");

        let tile = blend.blend_tiles()[0];
        assert_eq!(tile.table_index(), 1);
        assert_eq!(tile.blend_index(), 1);
        assert_eq!((tile.horizontal(), tile.vertical()), (1, 0));
        assert_eq!((tile.right_diagonal(), tile.left_diagonal()), (1, 0));
        assert_eq!((tile.inverted(), tile.long_diagonal()), (3, 1));
        assert_eq!(tile.custom_edge_class(), 0);

        let cliff = blend.cliff_info()[0];
        assert_eq!(cliff.table_index(), 1);
        assert_eq!(cliff.tile_index(), 3);
        assert_eq!(
            cliff.uv().map(f32::to_bits),
            [
                0,
                0,
                0,
                0x3F80_0000,
                0x3F80_0000,
                0x3F80_0000,
                0x3F80_0000,
                0
            ]
        );
        assert_eq!((cliff.flip(), cliff.mutant()), (1, 0));
    }

    #[test]
    fn version_six_derives_cliffs_from_neighboring_heights() {
        let blend = decode(&version_six_fixture(), MapLimits::default())
            .expect("valid version-six blend data");

        assert_eq!(blend.version(), 6);
        assert_eq!(blend.cliff_flag_stride(), 1);
        assert_eq!(blend.cliff_flags(), [0x7F, 0]);
        assert_eq!(blend.is_cliff(0, 0), Some(true));
        assert_eq!(blend.is_cliff(6, 0), Some(true));
        assert_eq!(blend.is_cliff(7, 0), Some(false));
        assert_eq!(blend.is_cliff(0, 1), Some(false));
        assert_eq!(blend.extra_blend_indices()[6], 1);
        assert_eq!(blend.cliff_info()[0].tile_index(), 3);
    }

    #[test]
    fn every_truncated_blend_payload_returns_an_error() {
        for original in [fixture(), version_six_fixture()] {
            let payload_length = original.len() - BLEND_PAYLOAD_OFFSET;
            for length in 0..payload_length {
                let mut bytes = original[..BLEND_PAYLOAD_OFFSET + length].to_vec();
                bytes[BLEND_CHUNK_OFFSET + 6..BLEND_CHUNK_OFFSET + 10]
                    .copy_from_slice(&i32::try_from(length).expect("fixture length").to_le_bytes());
                assert!(
                    decode(&bytes, MapLimits::default()).is_err(),
                    "version {} blend prefix of {length} bytes unexpectedly decoded",
                    u16::from_le_bytes([
                        original[BLEND_CHUNK_OFFSET + 4],
                        original[BLEND_CHUNK_OFFSET + 5]
                    ])
                );
            }
        }
    }

    #[test]
    fn narrow_version_seven_cliff_rows_normalize_to_zero_bits() {
        let mut reader = BinaryReader::new(&[], "narrow cliff flags");
        let (flags, stride) = read_version_seven_cliff_flags(&mut reader, 6, 2)
            .expect("legacy zero-byte rows normalize");
        assert_eq!(stride, 1);
        assert_eq!(flags, [0, 0]);
    }

    #[test]
    fn accepts_source_compatible_zero_cliff_info_count() {
        let mut bytes = fixture();
        bytes[CLIFF_INFO_COUNT_OFFSET..CLIFF_INFO_COUNT_OFFSET + 4]
            .copy_from_slice(&0_i32.to_le_bytes());
        bytes.truncate(bytes.len() - CLIFF_INFO_RECORD_BYTES);
        bytes[BLEND_CHUNK_OFFSET + 6..BLEND_CHUNK_OFFSET + 10].copy_from_slice(
            &i32::try_from(BLEND_PAYLOAD_LENGTH - CLIFF_INFO_RECORD_BYTES)
                .expect("fixture length")
                .to_le_bytes(),
        );

        let blend = decode(&bytes, MapLimits::default()).expect("zero cliff table");
        assert_eq!(blend.cliff_info_count(), 0);
        assert!(blend.cliff_info().is_empty());
    }

    #[test]
    fn rejects_version_counts_limits_flags_and_nonfinite_uvs() {
        for version in [5_u16, 8] {
            let mut unsupported = fixture();
            unsupported[BLEND_CHUNK_OFFSET + 4..BLEND_CHUNK_OFFSET + 6]
                .copy_from_slice(&version.to_le_bytes());
            assert_eq!(
                decode(&unsupported, MapLimits::default()),
                Err(MapBlendError::UnsupportedVersion(version))
            );
        }

        let mut count = fixture();
        count[BLEND_PAYLOAD_OFFSET..BLEND_PAYLOAD_OFFSET + 4]
            .copy_from_slice(&15_i32.to_le_bytes());
        assert_eq!(
            decode(&count, MapLimits::default()),
            Err(MapBlendError::CellCountMismatch {
                declared: 15,
                expected: 16
            })
        );

        assert!(matches!(
            decode(
                &fixture(),
                MapLimits {
                    maximum_bitmap_tiles: 3,
                    ..MapLimits::default()
                }
            ),
            Err(MapBlendError::Binary(BinaryError::LimitExceeded {
                what: "bitmap tile count",
                ..
            }))
        ));

        let mut flag = fixture();
        let flag_offset = flag
            .windows(4)
            .position(|window| window == RECORD_FLAG.to_le_bytes())
            .expect("blend flag");
        flag[flag_offset] ^= 1;
        assert!(matches!(
            decode(&flag, MapLimits::default()),
            Err(MapBlendError::InvalidBlendFlag { .. })
        ));

        let mut uv = fixture();
        let first_cliff_float = flag_offset + 4 + 4;
        uv[first_cliff_float..first_cliff_float + 4]
            .copy_from_slice(&f32::NAN.to_bits().to_le_bytes());
        assert!(matches!(
            decode(&uv, MapLimits::default()),
            Err(MapBlendError::NonFiniteCliffUv {
                index: 1,
                component: 0,
                ..
            })
        ));
    }
}
