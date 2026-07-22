// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Bounded decoding for MAP `GlobalLighting` versions 1 through 3.
//!
//! The field order and version additions are derived from `WorldHeightMap.cpp` and
//! `WHeightMapEdit.cpp` in `GeneralsGameCode` revision
//! `9f7abb866f5afd446db14149979e744c7216baaf`, licensed under GPL-3.0-or-later with
//! Electronic Arts Section 7 terms. Full notices are in `docs/provenance/map.md`.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

use cic_core::{BinaryError, BinaryReader};

use crate::{MapChunk, MapFile};

const GLOBAL_LIGHTING_LABEL: &[u8] = b"GlobalLighting";
const TIME_OF_DAY_COUNT: usize = 4;
const LIGHT_COMPONENTS: usize = 9;

/// Source time-of-day values stored as one-based MAP integers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapTimeOfDay {
    Morning,
    Afternoon,
    Evening,
    Night,
}

impl MapTimeOfDay {
    /// Returns the stable zero-based index into [`MapLightingData::periods`].
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Morning => 0,
            Self::Afternoon => 1,
            Self::Evening => 2,
            Self::Night => 3,
        }
    }

    /// Returns the source-facing variant name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Morning => "morning",
            Self::Afternoon => "afternoon",
            Self::Evening => "evening",
            Self::Night => "night",
        }
    }
}

/// One source-ordered ambient, diffuse, and direction record.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapLight {
    ambient: [f32; 3],
    diffuse: [f32; 3],
    direction: [f32; 3],
}

impl MapLight {
    #[must_use]
    pub const fn ambient(self) -> [f32; 3] {
        self.ambient
    }

    #[must_use]
    pub const fn diffuse(self) -> [f32; 3] {
        self.diffuse
    }

    #[must_use]
    pub const fn direction(self) -> [f32; 3] {
        self.direction
    }
}

/// Separate terrain and terrain-object lights for one ordered time-of-day variant.
#[derive(Debug, Clone, PartialEq)]
pub struct MapLightingPeriod {
    terrain: Vec<MapLight>,
    objects: Vec<MapLight>,
}

impl MapLightingPeriod {
    /// Returns the terrain sun followed by version-3 terrain accents.
    #[must_use]
    pub fn terrain_lights(&self) -> &[MapLight] {
        &self.terrain
    }

    /// Returns the object sun followed by version-2 object accents.
    #[must_use]
    pub fn object_lights(&self) -> &[MapLight] {
        &self.objects
    }
}

/// Immutable lighting data retained from one MAP.
#[derive(Debug, Clone, PartialEq)]
pub struct MapLightingData {
    version: u16,
    selected_time: MapTimeOfDay,
    periods: [MapLightingPeriod; TIME_OF_DAY_COUNT],
    shadow_color: Option<u32>,
}

impl MapLightingData {
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }

    #[must_use]
    pub const fn selected_time(&self) -> MapTimeOfDay {
        self.selected_time
    }

    /// Returns morning, afternoon, evening, and night records in source order.
    #[must_use]
    pub const fn periods(&self) -> &[MapLightingPeriod; TIME_OF_DAY_COUNT] {
        &self.periods
    }

    #[must_use]
    pub const fn selected_period(&self) -> &MapLightingPeriod {
        &self.periods[self.selected_time.index()]
    }

    /// Returns the optional packed source shadow color present at payload end.
    #[must_use]
    pub const fn shadow_color(&self) -> Option<u32> {
        self.shadow_color
    }
}

/// Identifies a light array when reporting malformed scalar input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapLightSet {
    Terrain,
    Objects,
}

/// A structured `GlobalLighting` semantic failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapLightingError {
    Binary(BinaryError),
    MissingGlobalLighting,
    DuplicateGlobalLighting,
    UnsupportedVersion(u16),
    InvalidTimeOfDay(i32),
    NonFiniteValue {
        period: u8,
        set: MapLightSet,
        light: u8,
        component: u8,
    },
    TrailingBytes(usize),
}

impl Display for MapLightingError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binary(error) => Display::fmt(error, formatter),
            Self::MissingGlobalLighting => formatter.write_str("MAP has no GlobalLighting chunk"),
            Self::DuplicateGlobalLighting => {
                formatter.write_str("MAP has more than one GlobalLighting chunk")
            }
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported GlobalLighting version {version}")
            }
            Self::InvalidTimeOfDay(value) => {
                write!(formatter, "GlobalLighting time of day is invalid: {value}")
            }
            Self::NonFiniteValue {
                period,
                set,
                light,
                component,
            } => write!(
                formatter,
                "GlobalLighting period {period} {set:?} light {light} component {component} is not finite"
            ),
            Self::TrailingBytes(count) => {
                write!(formatter, "GlobalLighting has {count} trailing bytes")
            }
        }
    }
}

impl Error for MapLightingError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Binary(error) => Some(error),
            _ => None,
        }
    }
}

impl From<BinaryError> for MapLightingError {
    fn from(error: BinaryError) -> Self {
        Self::Binary(error)
    }
}

/// Decodes the unique top-level `GlobalLighting` payload.
///
/// # Errors
///
/// Returns [`MapLightingError`] when the chunk is missing, duplicated, unsupported, truncated,
/// contains an invalid time/scalar, or does not close exactly.
pub fn decode_map_lighting(map: &MapFile) -> Result<MapLightingData, MapLightingError> {
    let chunk = lighting_chunk(map)?;
    if !(1..=3).contains(&chunk.version()) {
        return Err(MapLightingError::UnsupportedVersion(chunk.version()));
    }
    let mut reader = BinaryReader::new(
        chunk.data(),
        format!("GlobalLighting@{}", chunk.offset() + 10),
    );
    let raw_time = i32::from_le_bytes(reader.read_u32_le()?.to_le_bytes());
    let selected_time = match raw_time {
        1 => MapTimeOfDay::Morning,
        2 => MapTimeOfDay::Afternoon,
        3 => MapTimeOfDay::Evening,
        4 => MapTimeOfDay::Night,
        value => return Err(MapLightingError::InvalidTimeOfDay(value)),
    };
    let periods = [
        read_period(&mut reader, chunk.version(), 0)?,
        read_period(&mut reader, chunk.version(), 1)?,
        read_period(&mut reader, chunk.version(), 2)?,
        read_period(&mut reader, chunk.version(), 3)?,
    ];
    let shadow_color = if reader.remaining() == 0 {
        None
    } else {
        Some(reader.read_u32_le()?)
    };
    if reader.remaining() != 0 {
        return Err(MapLightingError::TrailingBytes(reader.remaining()));
    }
    Ok(MapLightingData {
        version: chunk.version(),
        selected_time,
        periods,
        shadow_color,
    })
}

fn read_period(
    reader: &mut BinaryReader<'_>,
    version: u16,
    period: usize,
) -> Result<MapLightingPeriod, MapLightingError> {
    let mut terrain = vec![read_light(reader, period, MapLightSet::Terrain, 0)?];
    let mut objects = vec![read_light(reader, period, MapLightSet::Objects, 0)?];
    if version >= 2 {
        for light in 1..3 {
            objects.push(read_light(reader, period, MapLightSet::Objects, light)?);
        }
    }
    if version >= 3 {
        for light in 1..3 {
            terrain.push(read_light(reader, period, MapLightSet::Terrain, light)?);
        }
    }
    Ok(MapLightingPeriod { terrain, objects })
}

fn read_light(
    reader: &mut BinaryReader<'_>,
    period: usize,
    set: MapLightSet,
    light: usize,
) -> Result<MapLight, MapLightingError> {
    let mut values = [0.0; LIGHT_COMPONENTS];
    for (component, value) in values.iter_mut().enumerate() {
        *value = f32::from_bits(reader.read_u32_le()?);
        if !value.is_finite() {
            return Err(MapLightingError::NonFiniteValue {
                period: u8::try_from(period).expect("period fits u8"),
                set,
                light: u8::try_from(light).expect("light fits u8"),
                component: u8::try_from(component).expect("component fits u8"),
            });
        }
    }
    Ok(MapLight {
        ambient: values[0..3].try_into().expect("three ambient values"),
        diffuse: values[3..6].try_into().expect("three diffuse values"),
        direction: values[6..9].try_into().expect("three direction values"),
    })
}

fn lighting_chunk(map: &MapFile) -> Result<&MapChunk, MapLightingError> {
    let mut matches = map
        .chunks()
        .iter()
        .filter(|chunk| map.symbol_name(chunk.id()) == Some(GLOBAL_LIGHTING_LABEL));
    let chunk = matches
        .next()
        .ok_or(MapLightingError::MissingGlobalLighting)?;
    if matches.next().is_some() {
        return Err(MapLightingError::DuplicateGlobalLighting);
    }
    Ok(chunk)
}

#[cfg(test)]
mod tests {
    use super::{MapLightSet, MapLightingError, MapTimeOfDay, decode_map_lighting};
    use crate::{MapLimits, parse_map};

    fn payload(version: u16) -> Vec<u8> {
        let mut bytes = 3_i32.to_le_bytes().to_vec();
        let light_count = match version {
            1 => 2,
            2 => 4,
            _ => 6,
        };
        for period in 0..4 {
            for light in 0..light_count {
                for component in 0..9 {
                    let scalar = u16::try_from(period * 100 + light * 10 + component)
                        .expect("test scalar fits u16");
                    let value = f32::from(scalar) / 100.0;
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
            }
        }
        bytes.extend_from_slice(&0x8040_2010_u32.to_le_bytes());
        bytes
    }

    fn map(version: u16, payload: &[u8]) -> Vec<u8> {
        let mut bytes = b"CkMp".to_vec();
        bytes.extend_from_slice(&1_i32.to_le_bytes());
        bytes.push(14);
        bytes.extend_from_slice(b"GlobalLighting");
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

    fn decode(version: u16, payload: &[u8]) -> Result<super::MapLightingData, MapLightingError> {
        let parsed = parse_map(&map(version, payload), "light.map", MapLimits::default())
            .expect("MAP inventory");
        decode_map_lighting(&parsed)
    }

    #[test]
    fn dispatches_versions_and_preserves_ordered_light_sets() {
        for version in 1..=3 {
            let lighting = decode(version, &payload(version)).expect("lighting");
            assert_eq!(lighting.version(), version);
            assert_eq!(lighting.selected_time(), MapTimeOfDay::Evening);
            assert_eq!(lighting.periods().len(), 4);
            assert_eq!(
                lighting.periods()[2].terrain_lights().len(),
                if version >= 3 { 3 } else { 1 }
            );
            assert_eq!(
                lighting.periods()[2].object_lights().len(),
                if version >= 2 { 3 } else { 1 }
            );
            assert_eq!(lighting.shadow_color(), Some(0x8040_2010));
            assert_eq!(
                lighting.periods()[1].terrain_lights()[0].ambient()[0].to_bits(),
                1.0_f32.to_bits()
            );
        }
    }

    #[test]
    fn rejects_every_truncated_payload_and_unestablished_version() {
        let complete = payload(3);
        let core_length = complete.len() - 4;
        for length in (0..core_length).chain(core_length + 1..complete.len()) {
            assert!(decode(3, &complete[..length]).is_err(), "prefix {length}");
        }
        assert_eq!(decode(4, &[]), Err(MapLightingError::UnsupportedVersion(4)));
    }

    #[test]
    fn rejects_invalid_time_non_finite_values_and_trailing_bytes() {
        let mut invalid_time = payload(1);
        invalid_time[0..4].copy_from_slice(&0_i32.to_le_bytes());
        assert_eq!(
            decode(1, &invalid_time),
            Err(MapLightingError::InvalidTimeOfDay(0))
        );

        let mut non_finite = payload(1);
        non_finite[4..8].copy_from_slice(&f32::NAN.to_le_bytes());
        assert_eq!(
            decode(1, &non_finite),
            Err(MapLightingError::NonFiniteValue {
                period: 0,
                set: MapLightSet::Terrain,
                light: 0,
                component: 0,
            })
        );

        let mut trailing = payload(1);
        trailing.push(0);
        assert_eq!(
            decode(1, &trailing),
            Err(MapLightingError::TrailingBytes(1))
        );
    }
}
