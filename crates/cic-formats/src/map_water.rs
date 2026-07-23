//! Bounded water-only decoding from MAP `PolygonTriggers` versions 2 through 4.
//!
//! The field order and water/river flags are derived from `PolygonTrigger.cpp` and
//! `PolygonTrigger.h` in `GeneralsGameCode` revision
//! `9f7abb866f5afd446db14149979e744c7216baaf`, licensed under GPL-3.0-or-later with
//! Electronic Arts Section 7 terms. Full notices are in `docs/provenance/map.md`.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

use cic_core::{BinaryError, BinaryReader};

use crate::{MapChunk, MapFile, MapLimits};

const POLYGON_TRIGGERS_LABEL: &[u8] = b"PolygonTriggers";

/// One integer world-space point retained from a water trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapWaterPoint([i32; 3]);

impl MapWaterPoint {
    /// Returns the source `(x, y, z)` coordinates.
    #[must_use]
    pub const fn coordinates(self) -> [i32; 3] {
        self.0
    }
}

/// One immutable water footprint retained without general script-trigger semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapWaterArea {
    source_index: u32,
    name: Vec<u8>,
    layer_name: Vec<u8>,
    trigger_id: i32,
    river: bool,
    river_start: i32,
    points: Vec<MapWaterPoint>,
}

impl MapWaterArea {
    /// Returns the trigger's stable file-order index, including discarded non-water triggers.
    #[must_use]
    pub const fn source_index(&self) -> u32 {
        self.source_index
    }
    /// Returns the source trigger name bytes.
    #[must_use]
    pub fn name_bytes(&self) -> &[u8] {
        &self.name
    }
    /// Returns the source `WorldBuilder` layer-name bytes, empty before version 4.
    #[must_use]
    pub fn layer_name_bytes(&self) -> &[u8] {
        &self.layer_name
    }
    /// Returns the source trigger identifier.
    #[must_use]
    pub const fn trigger_id(&self) -> i32 {
        self.trigger_id
    }
    /// Returns whether the footprint is a paired river strip.
    #[must_use]
    pub const fn is_river(&self) -> bool {
        self.river
    }
    /// Returns the source river seam index.
    #[must_use]
    pub const fn river_start(&self) -> i32 {
        self.river_start
    }
    /// Returns source points in file order.
    #[must_use]
    pub fn points(&self) -> &[MapWaterPoint] {
        &self.points
    }
}

/// Water footprints retained from one MAP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapWaterData {
    version: u16,
    source_trigger_count: u32,
    areas: Vec<MapWaterArea>,
}

impl MapWaterData {
    /// Returns the source `PolygonTriggers` version.
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }
    /// Returns the total trigger count, including discarded non-water triggers.
    #[must_use]
    pub const fn source_trigger_count(&self) -> u32 {
        self.source_trigger_count
    }
    /// Returns water footprints in stable source order.
    #[must_use]
    pub fn areas(&self) -> &[MapWaterArea] {
        &self.areas
    }
}

/// A structured water-only semantic failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapWaterError {
    /// A bounded binary read failed.
    Binary(BinaryError),
    /// The chunk is absent.
    MissingPolygonTriggers,
    /// More than one matching top-level chunk exists.
    DuplicatePolygonTriggers,
    /// The known schema does not cover this chunk version.
    UnsupportedVersion(u16),
    /// A signed count was negative.
    NegativeValue { field: &'static str, value: i32 },
    /// Bytes remain after the declared records.
    TrailingBytes(usize),
}

impl Display for MapWaterError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binary(error) => Display::fmt(error, f),
            Self::MissingPolygonTriggers => f.write_str("MAP has no PolygonTriggers chunk"),
            Self::DuplicatePolygonTriggers => {
                f.write_str("MAP has more than one PolygonTriggers chunk")
            }
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported PolygonTriggers version {version}")
            }
            Self::NegativeValue { field, value } => {
                write!(f, "PolygonTriggers {field} is negative: {value}")
            }
            Self::TrailingBytes(count) => write!(f, "PolygonTriggers has {count} trailing bytes"),
        }
    }
}

impl Error for MapWaterError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Binary(error) => Some(error),
            _ => None,
        }
    }
}

impl From<BinaryError> for MapWaterError {
    fn from(error: BinaryError) -> Self {
        Self::Binary(error)
    }
}

/// Decodes only water-typed records from the unique `PolygonTriggers` chunk.
///
/// General trigger semantics, scripts, and object loading are intentionally excluded. Non-water
/// records are bounded and skipped so the retained value stays renderer-neutral and water-only.
///
/// # Errors
///
/// Returns [`MapWaterError`] when the chunk is missing, duplicated, unsupported, truncated,
/// malformed, over a configured limit, or does not close exactly.
pub fn decode_map_water(map: &MapFile, limits: MapLimits) -> Result<MapWaterData, MapWaterError> {
    let chunk = water_chunk(map)?;
    if !(2..=4).contains(&chunk.version()) {
        return Err(MapWaterError::UnsupportedVersion(chunk.version()));
    }
    let mut reader = BinaryReader::new(
        chunk.data(),
        format!("PolygonTriggers@{}", chunk.offset() + 10),
    );
    let trigger_count = read_nonnegative(&mut reader, "trigger count")?;
    enforce_limit(
        "MAP polygon trigger count",
        trigger_count as usize,
        limits.maximum_polygon_triggers,
    )?;
    let mut areas = Vec::new();
    let mut retained_points = 0_usize;
    for source_index in 0..trigger_count {
        let name_length = usize::from(reader.read_u16_le()?);
        enforce_limit(
            "MAP polygon trigger name length",
            name_length,
            limits.maximum_trigger_name_bytes,
        )?;
        let name = reader.read_exact(name_length)?.to_vec();
        let layer_name = if chunk.version() >= 4 {
            let length = usize::from(reader.read_u16_le()?);
            enforce_limit(
                "MAP polygon trigger layer name length",
                length,
                limits.maximum_trigger_name_bytes,
            )?;
            reader.read_exact(length)?.to_vec()
        } else {
            Vec::new()
        };
        let trigger_id = read_i32(&mut reader)?;
        let is_water = reader.read_u8()? != 0;
        let (river, river_start) = if chunk.version() >= 3 {
            (reader.read_u8()? != 0, read_i32(&mut reader)?)
        } else {
            (false, 0)
        };
        let point_count = read_nonnegative(&mut reader, "point count")?;
        enforce_limit(
            "MAP polygon point count",
            point_count as usize,
            limits.maximum_polygon_points,
        )?;
        if !is_water {
            let byte_count =
                (point_count as usize)
                    .checked_mul(12)
                    .ok_or(BinaryError::LimitExceeded {
                        what: "MAP skipped polygon point bytes",
                        actual: usize::MAX,
                        maximum: limits.maximum_chunk_bytes,
                    })?;
            reader.read_exact(byte_count)?;
            continue;
        }
        retained_points = retained_points.checked_add(point_count as usize).ok_or(
            BinaryError::LimitExceeded {
                what: "MAP retained water point count",
                actual: usize::MAX,
                maximum: limits.maximum_water_points,
            },
        )?;
        enforce_limit(
            "MAP retained water point count",
            retained_points,
            limits.maximum_water_points,
        )?;
        let mut points = Vec::with_capacity(point_count as usize);
        for _ in 0..point_count {
            points.push(MapWaterPoint([
                read_i32(&mut reader)?,
                read_i32(&mut reader)?,
                read_i32(&mut reader)?,
            ]));
        }
        areas.push(MapWaterArea {
            source_index,
            name,
            layer_name,
            trigger_id,
            river,
            river_start,
            points,
        });
    }
    if reader.remaining() != 0 {
        return Err(MapWaterError::TrailingBytes(reader.remaining()));
    }
    Ok(MapWaterData {
        version: chunk.version(),
        source_trigger_count: trigger_count,
        areas,
    })
}

fn water_chunk(map: &MapFile) -> Result<&MapChunk, MapWaterError> {
    let mut matches = map
        .chunks()
        .iter()
        .filter(|chunk| map.symbol_name(chunk.id()) == Some(POLYGON_TRIGGERS_LABEL));
    let chunk = matches
        .next()
        .ok_or(MapWaterError::MissingPolygonTriggers)?;
    if matches.next().is_some() {
        return Err(MapWaterError::DuplicatePolygonTriggers);
    }
    Ok(chunk)
}

fn read_i32(reader: &mut BinaryReader<'_>) -> Result<i32, BinaryError> {
    Ok(i32::from_le_bytes(reader.read_u32_le()?.to_le_bytes()))
}

fn read_nonnegative(
    reader: &mut BinaryReader<'_>,
    field: &'static str,
) -> Result<u32, MapWaterError> {
    let value = read_i32(reader)?;
    u32::try_from(value).map_err(|_| MapWaterError::NegativeValue { field, value })
}

fn enforce_limit(what: &'static str, actual: usize, maximum: usize) -> Result<(), MapWaterError> {
    if actual > maximum {
        Err(BinaryError::LimitExceeded {
            what,
            actual,
            maximum,
        }
        .into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use cic_core::BinaryError;

    use super::{MapWaterError, decode_map_water};
    use crate::{MapLimits, parse_map};

    fn map(version: u16, payload: &[u8]) -> Vec<u8> {
        let mut bytes = b"CkMp".to_vec();
        bytes.extend_from_slice(&1_i32.to_le_bytes());
        bytes.push(15);
        bytes.extend_from_slice(b"PolygonTriggers");
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&version.to_le_bytes());
        bytes.extend_from_slice(
            &i32::try_from(payload.len())
                .expect("test payload fits i32")
                .to_le_bytes(),
        );
        bytes.extend_from_slice(payload);
        bytes
    }

    fn payload(version: u16) -> Vec<u8> {
        payload_with_names(version, b"river", b"Water")
    }

    fn payload_with_names(version: u16, name: &[u8], layer_name: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1_i32.to_le_bytes());
        bytes.extend_from_slice(
            &u16::try_from(name.len())
                .expect("test name length")
                .to_le_bytes(),
        );
        bytes.extend_from_slice(name);
        if version >= 4 {
            bytes.extend_from_slice(
                &u16::try_from(layer_name.len())
                    .expect("test layer length")
                    .to_le_bytes(),
            );
            bytes.extend_from_slice(layer_name);
        }
        bytes.extend_from_slice(&9_i32.to_le_bytes());
        bytes.push(1);
        if version >= 3 {
            bytes.push(1);
            bytes.extend_from_slice(&1_i32.to_le_bytes());
        }
        bytes.extend_from_slice(&4_i32.to_le_bytes());
        for point in [[0_i32, 0, 5], [0, 10, 5], [20, 0, 5], [20, 10, 5]] {
            for value in point {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        bytes
    }

    fn decode(bytes: &[u8], limits: MapLimits) -> Result<super::MapWaterData, MapWaterError> {
        let parsed = parse_map(bytes, "water.map", limits).expect("inventory");
        decode_map_water(&parsed, limits)
    }

    #[test]
    fn retains_water_and_dispatches_versions() {
        for version in [2, 3, 4] {
            let map = parse_map(
                &map(version, &payload(version)),
                "water.map",
                MapLimits::default(),
            )
            .expect("inventory");
            let water = decode_map_water(&map, MapLimits::default()).expect("water");
            assert_eq!(water.version(), version);
            assert_eq!(water.source_trigger_count(), 1);
            assert_eq!(water.areas()[0].points()[2].coordinates(), [20, 0, 5]);
            assert_eq!(water.areas()[0].is_river(), version >= 3);
            assert_eq!(
                water.areas()[0].layer_name_bytes(),
                if version == 4 {
                    b"Water".as_slice()
                } else {
                    b"".as_slice()
                }
            );
        }
    }

    #[test]
    fn version_defaults_and_v4_structure_match_the_source_reader() {
        let version_two = decode(&map(2, &payload(2)), MapLimits::default()).expect("version two");
        let area = &version_two.areas()[0];
        assert!(!area.is_river());
        assert_eq!(area.river_start(), 0);
        assert!(area.layer_name_bytes().is_empty());

        let version_three =
            decode(&map(3, &payload(3)), MapLimits::default()).expect("version three");
        assert!(version_three.areas()[0].layer_name_bytes().is_empty());

        let version_four_payload = payload(4);
        assert_eq!(
            version_four_payload.len(),
            payload(3).len() + 2 + b"Water".len()
        );
        let version_four =
            decode(&map(4, &version_four_payload), MapLimits::default()).expect("version four");
        let area = &version_four.areas()[0];
        assert_eq!(area.source_index(), 0);
        assert_eq!(area.name_bytes(), b"river");
        assert_eq!(area.layer_name_bytes(), b"Water");
        assert_eq!(area.trigger_id(), 9);
        assert!(area.is_river());
        assert_eq!(area.river_start(), 1);
        assert_eq!(
            area.points()
                .iter()
                .map(|point| point.coordinates())
                .collect::<Vec<_>>(),
            [[0, 0, 5], [0, 10, 5], [20, 0, 5], [20, 10, 5]]
        );
    }

    #[test]
    fn nonwater_records_are_skipped_without_renumbering_water_sources() {
        let source = payload(4);
        let record = &source[4..];
        let mut nonwater = record.to_vec();
        nonwater[18] = 0;
        let mut two_records = 2_i32.to_le_bytes().to_vec();
        two_records.extend_from_slice(&nonwater);
        two_records.extend_from_slice(record);

        let water =
            decode(&map(4, &two_records), MapLimits::default()).expect("mixed polygon triggers");
        assert_eq!(water.source_trigger_count(), 2);
        assert_eq!(water.areas().len(), 1);
        assert_eq!(water.areas()[0].source_index(), 1);
        assert_eq!(water.areas()[0].name_bytes(), b"river");
    }

    #[test]
    fn rejects_every_truncated_payload_and_unestablished_version() {
        let complete = payload(4);
        for length in 0..complete.len() {
            let parsed = parse_map(
                &map(4, &complete[..length]),
                "truncated.map",
                MapLimits::default(),
            )
            .expect("inventory");
            assert!(
                decode_map_water(&parsed, MapLimits::default()).is_err(),
                "prefix {length}"
            );
        }
        let parsed = parse_map(&map(1, &[]), "v1.map", MapLimits::default()).expect("inventory");
        assert_eq!(
            decode_map_water(&parsed, MapLimits::default()),
            Err(MapWaterError::UnsupportedVersion(1))
        );
        let parsed = parse_map(&map(5, &[]), "v5.map", MapLimits::default()).expect("inventory");
        assert_eq!(
            decode_map_water(&parsed, MapLimits::default()),
            Err(MapWaterError::UnsupportedVersion(5))
        );
    }

    #[test]
    fn rejects_missing_duplicate_negative_and_trailing_structures() {
        let mut missing = map(4, &payload(4));
        missing[9] = b'X';
        let parsed =
            parse_map(&missing, "missing.map", MapLimits::default()).expect("other inventory");
        assert_eq!(
            decode_map_water(&parsed, MapLimits::default()),
            Err(MapWaterError::MissingPolygonTriggers)
        );

        let mut duplicate = map(4, &payload(4));
        let chunk_offset = 4 + 4 + 1 + b"PolygonTriggers".len() + 4;
        duplicate.extend_from_within(chunk_offset..);
        assert_eq!(
            decode(&duplicate, MapLimits::default()),
            Err(MapWaterError::DuplicatePolygonTriggers)
        );

        let mut negative_count = payload(4);
        negative_count[..4].copy_from_slice(&(-1_i32).to_le_bytes());
        assert_eq!(
            decode(&map(4, &negative_count), MapLimits::default()),
            Err(MapWaterError::NegativeValue {
                field: "trigger count",
                value: -1
            })
        );

        let mut negative_points = payload(4);
        negative_points[28..32].copy_from_slice(&(-1_i32).to_le_bytes());
        assert_eq!(
            decode(&map(4, &negative_points), MapLimits::default()),
            Err(MapWaterError::NegativeValue {
                field: "point count",
                value: -1
            })
        );

        let mut trailing = map(4, &payload(4));
        trailing.push(0xAA);
        let payload_length_offset = chunk_offset + 6;
        let new_length = i32::try_from(payload(4).len() + 1).expect("payload length");
        trailing[payload_length_offset..payload_length_offset + 4]
            .copy_from_slice(&new_length.to_le_bytes());
        assert_eq!(
            decode(&trailing, MapLimits::default()),
            Err(MapWaterError::TrailingBytes(1))
        );
    }

    #[test]
    fn preserves_degenerate_water_safely_and_rejects_limits() {
        let complete = payload(3);
        let parsed =
            parse_map(&map(3, &complete), "limit.map", MapLimits::default()).expect("inventory");
        assert!(
            decode_map_water(
                &parsed,
                MapLimits {
                    maximum_polygon_triggers: 0,
                    ..MapLimits::default()
                }
            )
            .is_err()
        );
        for (limits, what) in [
            (
                MapLimits {
                    maximum_trigger_name_bytes: 4,
                    ..MapLimits::default()
                },
                "name/layer",
            ),
            (
                MapLimits {
                    maximum_polygon_points: 3,
                    ..MapLimits::default()
                },
                "points",
            ),
            (
                MapLimits {
                    maximum_water_points: 3,
                    ..MapLimits::default()
                },
                "retained points",
            ),
        ] {
            assert!(
                matches!(
                    decode(&map(4, &payload(4)), limits),
                    Err(MapWaterError::Binary(BinaryError::LimitExceeded { .. }))
                ),
                "{what} limit unexpectedly accepted"
            );
        }
        assert!(matches!(
            decode(
                &map(4, &payload_with_names(4, b"road", b"Water")),
                MapLimits {
                    maximum_trigger_name_bytes: 4,
                    ..MapLimits::default()
                }
            ),
            Err(MapWaterError::Binary(BinaryError::LimitExceeded { .. }))
        ));
        let mut degenerate = complete;
        degenerate[21..25].copy_from_slice(&0_i32.to_le_bytes());
        degenerate.truncate(25);
        let parsed =
            parse_map(&map(3, &degenerate), "river.map", MapLimits::default()).expect("inventory");
        assert!(
            decode_map_water(&parsed, MapLimits::default())
                .expect("bounded empty river")
                .areas()[0]
                .points()
                .is_empty()
        );
    }
}
