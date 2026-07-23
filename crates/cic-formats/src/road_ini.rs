// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Bounded immutable decoder for the presentation fields of `Road` and `Bridge` INI blocks.
//!
//! Field names, constructor defaults, and value meanings are derived from
//! `TerrainRoads.h`, `TerrainRoads.cpp`, and `INITerrainRoad.cpp` in
//! `GeneralsGameCode` revision `9f7abb866f5afd446db14149979e744c7216baaf`.
//! That source is GPL-3.0-or-later with Electronic Arts Section 7 terms; full
//! notices are recorded in `docs/provenance/map.md`.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

/// Explicit allocation and input bounds for [`parse_road_ini`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoadIniLimits {
    pub max_file_bytes: usize,
    pub max_lines: usize,
    pub max_line_bytes: usize,
    pub max_definitions: usize,
    pub max_name_bytes: usize,
    pub max_texture_bytes: usize,
    pub max_model_bytes: usize,
}

impl Default for RoadIniLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: 4 * 1_024 * 1_024,
            max_lines: 100_000,
            max_line_bytes: 4_096,
            max_definitions: 16_384,
            max_name_bytes: 255,
            max_texture_bytes: 1_024,
            max_model_bytes: 1_024,
        }
    }
}

/// Source bridge body states whose assets are retained without activating damage simulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeBodyState {
    Pristine,
    Damaged,
    ReallyDamaged,
    Broken,
}

impl BridgeBodyState {
    const fn index(self) -> usize {
        match self {
            Self::Pristine => 0,
            Self::Damaged => 1,
            Self::ReallyDamaged => 2,
            Self::Broken => 3,
        }
    }
}

/// Stable source order for the four optional bridge-tower object templates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeTowerSlot {
    FromLeft,
    FromRight,
    ToLeft,
    ToRight,
}

impl BridgeTowerSlot {
    pub const ALL: [Self; 4] = [Self::FromLeft, Self::FromRight, Self::ToLeft, Self::ToRight];

    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::FromLeft => 0,
            Self::FromRight => 1,
            Self::ToLeft => 2,
            Self::ToRight => 3,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
struct BridgeStateAssets {
    model: Option<Vec<u8>>,
    texture: Option<Vec<u8>>,
}

/// One source-ordered bridge presentation definition.
///
/// Damage-state assets are immutable references only. R3 always selects [`BridgeBodyState::Pristine`]
/// and does not create bridge logic, damage transitions, collision, or repair behavior.
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeDefinition {
    name: Vec<u8>,
    states: [BridgeStateAssets; 4],
    tower_objects: [Option<Vec<u8>>; 4],
    bridge_scale: Option<f32>,
}

impl BridgeDefinition {
    #[must_use]
    pub fn name_bytes(&self) -> &[u8] {
        &self.name
    }

    #[must_use]
    pub fn model_bytes(&self) -> Option<&[u8]> {
        self.state_model_bytes(BridgeBodyState::Pristine)
    }

    #[must_use]
    pub fn state_model_bytes(&self, state: BridgeBodyState) -> Option<&[u8]> {
        self.states[state.index()].model.as_deref()
    }

    #[must_use]
    pub fn state_texture_bytes(&self, state: BridgeBodyState) -> Option<&[u8]> {
        self.states[state.index()].texture.as_deref()
    }

    #[must_use]
    pub fn tower_object_name_bytes(&self, slot: BridgeTowerSlot) -> Option<&[u8]> {
        self.tower_objects[slot.index()].as_deref()
    }

    #[must_use]
    pub fn bridge_scale(&self) -> f32 {
        self.bridge_scale.unwrap_or(1.0)
    }

    /// Returns this declaration with omitted presentation fields inherited from `DefaultBridge`.
    #[must_use]
    pub fn inherit_missing(&self, default_bridge: &Self) -> Self {
        Self {
            name: self.name.clone(),
            states: std::array::from_fn(|index| BridgeStateAssets {
                model: self.states[index]
                    .model
                    .clone()
                    .or_else(|| default_bridge.states[index].model.clone()),
                texture: self.states[index]
                    .texture
                    .clone()
                    .or_else(|| default_bridge.states[index].texture.clone()),
            }),
            // The source `newBridge` inheritance path does not copy tower object names.
            tower_objects: self.tower_objects.clone(),
            bridge_scale: self.bridge_scale.or(default_bridge.bridge_scale),
        }
    }
}

/// One source-ordered road definition. Missing numeric fields retain the source constructor zero.
#[derive(Debug, Clone, PartialEq)]
pub struct RoadDefinition {
    name: Vec<u8>,
    texture: Option<Vec<u8>>,
    road_width: f32,
    road_width_in_texture: f32,
}

impl RoadDefinition {
    #[must_use]
    pub fn name_bytes(&self) -> &[u8] {
        &self.name
    }

    #[must_use]
    pub fn texture_bytes(&self) -> Option<&[u8]> {
        self.texture.as_deref()
    }

    #[must_use]
    pub const fn road_width(&self) -> f32 {
        self.road_width
    }

    #[must_use]
    pub const fn road_width_in_texture(&self) -> f32 {
        self.road_width_in_texture
    }
}

/// Immutable source-order declarations from one INI provider.
#[derive(Debug, Clone, PartialEq)]
pub struct RoadIni {
    definitions: Vec<RoadDefinition>,
    bridges: Vec<BridgeDefinition>,
}

impl RoadIni {
    #[must_use]
    pub fn definitions(&self) -> &[RoadDefinition] {
        &self.definitions
    }

    #[must_use]
    pub fn bridges(&self) -> &[BridgeDefinition] {
        &self.bridges
    }
}

/// Structured road-definition decoding failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoadIniError {
    FileTooLarge {
        size: usize,
        limit: usize,
    },
    TooManyLines {
        limit: usize,
    },
    LineTooLong {
        line: usize,
        size: usize,
        limit: usize,
    },
    TooManyDefinitions {
        line: usize,
        limit: usize,
    },
    MissingRoadName {
        line: usize,
    },
    MissingBridgeName {
        line: usize,
    },
    RoadNameTooLong {
        line: usize,
        size: usize,
        limit: usize,
    },
    NestedRoad {
        line: usize,
    },
    NestedDefinition {
        line: usize,
    },
    UnterminatedRoad {
        line: usize,
    },
    UnterminatedBridge {
        line: usize,
    },
    MissingValue {
        line: usize,
        field: &'static str,
    },
    ValueTooLong {
        line: usize,
        field: &'static str,
        size: usize,
        limit: usize,
    },
    InvalidReal {
        line: usize,
        field: &'static str,
    },
}

impl Display for RoadIniError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::FileTooLarge { size, limit } => {
                write!(formatter, "Road INI is {size} bytes; limit is {limit}")
            }
            Self::TooManyLines { limit } => {
                write!(formatter, "Road INI exceeds the {limit}-line limit")
            }
            Self::LineTooLong { line, size, limit } => write!(
                formatter,
                "Road INI line {line} is {size} bytes; limit is {limit}"
            ),
            Self::TooManyDefinitions { line, limit } => write!(
                formatter,
                "Road INI exceeds the {limit}-definition limit at line {line}"
            ),
            Self::MissingRoadName { line } => {
                write!(formatter, "Road INI line {line} has no road name")
            }
            Self::MissingBridgeName { line } => {
                write!(formatter, "Road INI line {line} has no bridge name")
            }
            Self::RoadNameTooLong { line, size, limit } => write!(
                formatter,
                "Road name on line {line} is {size} bytes; limit is {limit}"
            ),
            Self::NestedRoad { line } => {
                write!(
                    formatter,
                    "Road INI starts a road before End at line {line}"
                )
            }
            Self::NestedDefinition { line } => write!(
                formatter,
                "Road INI starts a road or bridge before End at line {line}"
            ),
            Self::UnterminatedRoad { line } => {
                write!(formatter, "Road INI road opened on line {line} has no End")
            }
            Self::UnterminatedBridge { line } => {
                write!(
                    formatter,
                    "Road INI bridge opened on line {line} has no End"
                )
            }
            Self::MissingValue { line, field } => {
                write!(formatter, "Road INI {field} on line {line} has no value")
            }
            Self::ValueTooLong {
                line,
                field,
                size,
                limit,
            } => write!(
                formatter,
                "Road INI {field} on line {line} is {size} bytes; limit is {limit}"
            ),
            Self::InvalidReal { line, field } => write!(
                formatter,
                "Road INI {field} on line {line} is not a finite real"
            ),
        }
    }
}

impl Error for RoadIniError {}

#[derive(Debug)]
struct ActiveRoad {
    line: usize,
    name: Vec<u8>,
    texture: Option<Vec<u8>>,
    road_width: f32,
    road_width_in_texture: f32,
}

#[derive(Debug)]
struct ActiveBridge {
    line: usize,
    name: Vec<u8>,
    states: [BridgeStateAssets; 4],
    tower_objects: [Option<Vec<u8>>; 4],
    bridge_scale: Option<f32>,
}

/// Decodes the renderer-facing subset of `Road` blocks and ignores unrelated INI blocks.
///
/// Unknown fields inside a road are ignored. Repeated known fields use source parse order, so the
/// last occurrence in that declaration wins. This function does not resolve provider overlays or
/// texture resources.
///
/// # Errors
///
/// Returns a structured error for malformed road structure, invalid finite reals, or any explicit
/// input/allocation limit.
#[allow(clippy::too_many_lines)]
pub fn parse_road_ini(bytes: &[u8], limits: RoadIniLimits) -> Result<RoadIni, RoadIniError> {
    if bytes.len() > limits.max_file_bytes {
        return Err(RoadIniError::FileTooLarge {
            size: bytes.len(),
            limit: limits.max_file_bytes,
        });
    }
    let mut definitions = Vec::new();
    let mut bridges = Vec::new();
    let mut active = None;
    let mut active_bridge = None;
    for (zero_based_line, raw_line) in bytes.split(|byte| *byte == b'\n').enumerate() {
        let line_number = zero_based_line
            .checked_add(1)
            .ok_or(RoadIniError::TooManyLines {
                limit: limits.max_lines,
            })?;
        if line_number > limits.max_lines {
            return Err(RoadIniError::TooManyLines {
                limit: limits.max_lines,
            });
        }
        if raw_line.len() > limits.max_line_bytes {
            return Err(RoadIniError::LineTooLong {
                line: line_number,
                size: raw_line.len(),
                limit: limits.max_line_bytes,
            });
        }
        let line = trim_ascii(strip_comment(raw_line));
        if line.is_empty() {
            continue;
        }
        if line.eq_ignore_ascii_case(b"Road") {
            return Err(RoadIniError::MissingRoadName { line: line_number });
        }
        if token_eq(line, b"Road") {
            if active.is_some() || active_bridge.is_some() {
                return Err(if active.is_some() {
                    RoadIniError::NestedRoad { line: line_number }
                } else {
                    RoadIniError::NestedDefinition { line: line_number }
                });
            }
            let name = trim_ascii(&line[b"Road".len()..]);
            if name.is_empty() {
                return Err(RoadIniError::MissingRoadName { line: line_number });
            }
            if name.len() > limits.max_name_bytes {
                return Err(RoadIniError::RoadNameTooLong {
                    line: line_number,
                    size: name.len(),
                    limit: limits.max_name_bytes,
                });
            }
            active = Some(ActiveRoad {
                line: line_number,
                name: name.to_vec(),
                texture: None,
                road_width: 0.0,
                road_width_in_texture: 0.0,
            });
            continue;
        }
        if line.eq_ignore_ascii_case(b"Bridge") {
            return Err(RoadIniError::MissingBridgeName { line: line_number });
        }
        if token_eq(line, b"Bridge") {
            if active.is_some() || active_bridge.is_some() {
                return Err(RoadIniError::NestedDefinition { line: line_number });
            }
            let name = trim_ascii(&line[b"Bridge".len()..]);
            if name.is_empty() {
                return Err(RoadIniError::MissingBridgeName { line: line_number });
            }
            if name.len() > limits.max_name_bytes {
                return Err(RoadIniError::RoadNameTooLong {
                    line: line_number,
                    size: name.len(),
                    limit: limits.max_name_bytes,
                });
            }
            active_bridge = Some(ActiveBridge {
                line: line_number,
                name: name.to_vec(),
                states: std::array::from_fn(|_| BridgeStateAssets::default()),
                tower_objects: std::array::from_fn(|_| None),
                bridge_scale: None,
            });
            continue;
        }
        if line.eq_ignore_ascii_case(b"End") {
            if active.is_none() && active_bridge.is_none() {
                continue;
            }
            if definitions.len().saturating_add(bridges.len()) >= limits.max_definitions {
                return Err(RoadIniError::TooManyDefinitions {
                    line: line_number,
                    limit: limits.max_definitions,
                });
            }
            if let Some(road) = active.take() {
                definitions.push(RoadDefinition {
                    name: road.name,
                    texture: road.texture,
                    road_width: road.road_width,
                    road_width_in_texture: road.road_width_in_texture,
                });
            } else if let Some(bridge) = active_bridge.take() {
                bridges.push(BridgeDefinition {
                    name: bridge.name,
                    states: bridge.states,
                    tower_objects: bridge.tower_objects,
                    bridge_scale: bridge.bridge_scale,
                });
            }
            continue;
        }
        if active.is_none() && active_bridge.is_none() {
            continue;
        }
        let Some((field, value)) = split_assignment(line) else {
            continue;
        };
        if let Some(road) = active.as_mut() {
            if field.eq_ignore_ascii_case(b"Texture") {
                road.texture = Some(read_string(
                    value,
                    line_number,
                    "Texture",
                    limits.max_texture_bytes,
                )?);
            } else if field.eq_ignore_ascii_case(b"RoadWidth") {
                road.road_width = read_real(value, line_number, "RoadWidth")?;
            } else if field.eq_ignore_ascii_case(b"RoadWidthInTexture") {
                road.road_width_in_texture = read_real(value, line_number, "RoadWidthInTexture")?;
            }
        } else if let Some(bridge) = active_bridge.as_mut() {
            if field.eq_ignore_ascii_case(b"BridgeModelName") {
                bridge.states[BridgeBodyState::Pristine.index()].model = Some(read_string(
                    value,
                    line_number,
                    "BridgeModelName",
                    limits.max_model_bytes,
                )?);
            } else if field.eq_ignore_ascii_case(b"Texture") {
                bridge.states[BridgeBodyState::Pristine.index()].texture = Some(read_string(
                    value,
                    line_number,
                    "Texture",
                    limits.max_texture_bytes,
                )?);
            } else if field.eq_ignore_ascii_case(b"BridgeModelNameDamaged") {
                bridge.states[BridgeBodyState::Damaged.index()].model = Some(read_string(
                    value,
                    line_number,
                    "BridgeModelNameDamaged",
                    limits.max_model_bytes,
                )?);
            } else if field.eq_ignore_ascii_case(b"TextureDamaged") {
                bridge.states[BridgeBodyState::Damaged.index()].texture = Some(read_string(
                    value,
                    line_number,
                    "TextureDamaged",
                    limits.max_texture_bytes,
                )?);
            } else if field.eq_ignore_ascii_case(b"BridgeModelNameReallyDamaged") {
                bridge.states[BridgeBodyState::ReallyDamaged.index()].model = Some(read_string(
                    value,
                    line_number,
                    "BridgeModelNameReallyDamaged",
                    limits.max_model_bytes,
                )?);
            } else if field.eq_ignore_ascii_case(b"TextureReallyDamaged") {
                bridge.states[BridgeBodyState::ReallyDamaged.index()].texture = Some(read_string(
                    value,
                    line_number,
                    "TextureReallyDamaged",
                    limits.max_texture_bytes,
                )?);
            } else if field.eq_ignore_ascii_case(b"BridgeModelNameBroken") {
                bridge.states[BridgeBodyState::Broken.index()].model = Some(read_string(
                    value,
                    line_number,
                    "BridgeModelNameBroken",
                    limits.max_model_bytes,
                )?);
            } else if field.eq_ignore_ascii_case(b"TextureBroken") {
                bridge.states[BridgeBodyState::Broken.index()].texture = Some(read_string(
                    value,
                    line_number,
                    "TextureBroken",
                    limits.max_texture_bytes,
                )?);
            } else if field.eq_ignore_ascii_case(b"TowerObjectNameFromLeft") {
                bridge.tower_objects[BridgeTowerSlot::FromLeft.index()] = Some(read_string(
                    value,
                    line_number,
                    "TowerObjectNameFromLeft",
                    limits.max_name_bytes,
                )?);
            } else if field.eq_ignore_ascii_case(b"TowerObjectNameFromRight") {
                bridge.tower_objects[BridgeTowerSlot::FromRight.index()] = Some(read_string(
                    value,
                    line_number,
                    "TowerObjectNameFromRight",
                    limits.max_name_bytes,
                )?);
            } else if field.eq_ignore_ascii_case(b"TowerObjectNameToLeft") {
                bridge.tower_objects[BridgeTowerSlot::ToLeft.index()] = Some(read_string(
                    value,
                    line_number,
                    "TowerObjectNameToLeft",
                    limits.max_name_bytes,
                )?);
            } else if field.eq_ignore_ascii_case(b"TowerObjectNameToRight") {
                bridge.tower_objects[BridgeTowerSlot::ToRight.index()] = Some(read_string(
                    value,
                    line_number,
                    "TowerObjectNameToRight",
                    limits.max_name_bytes,
                )?);
            } else if field.eq_ignore_ascii_case(b"BridgeScale") {
                bridge.bridge_scale = Some(read_real(value, line_number, "BridgeScale")?);
            }
        }
    }
    if let Some(road) = active {
        return Err(RoadIniError::UnterminatedRoad { line: road.line });
    }
    if let Some(bridge) = active_bridge {
        return Err(RoadIniError::UnterminatedBridge { line: bridge.line });
    }
    Ok(RoadIni {
        definitions,
        bridges,
    })
}

fn read_string(
    value: &[u8],
    line: usize,
    field: &'static str,
    limit: usize,
) -> Result<Vec<u8>, RoadIniError> {
    let value = unquote(trim_ascii(value));
    if value.is_empty() {
        return Err(RoadIniError::MissingValue { line, field });
    }
    if value.len() > limit {
        return Err(RoadIniError::ValueTooLong {
            line,
            field,
            size: value.len(),
            limit,
        });
    }
    Ok(value.to_vec())
}

fn read_real(value: &[u8], line: usize, field: &'static str) -> Result<f32, RoadIniError> {
    std::str::from_utf8(trim_ascii(value))
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|value| value.is_finite())
        .ok_or(RoadIniError::InvalidReal { line, field })
}

fn strip_comment(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    line.iter()
        .position(|byte| *byte == b';')
        .map_or(line, |index| &line[..index])
}

fn trim_ascii(mut bytes: &[u8]) -> &[u8] {
    while bytes.first().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[1..];
    }
    while bytes.last().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

fn token_eq(line: &[u8], token: &[u8]) -> bool {
    line.get(..token.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(token))
        && line.get(token.len()).is_some_and(u8::is_ascii_whitespace)
}

fn split_assignment(line: &[u8]) -> Option<(&[u8], &[u8])> {
    let equals = line.iter().position(|byte| *byte == b'=')?;
    Some((trim_ascii(&line[..equals]), trim_ascii(&line[equals + 1..])))
}

fn unquote(value: &[u8]) -> &[u8] {
    if value.len() >= 2 && value.first() == Some(&b'"') && value.last() == Some(&b'"') {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::{BridgeBodyState, BridgeTowerSlot, RoadIniError, RoadIniLimits, parse_road_ini};

    #[test]
    fn decodes_road_and_bridge_fields_in_source_order() {
        let ini = parse_road_ini(
            b"Bridge SyntheticBridge\n BridgeScale = 1.25\n BridgeModelName = synthetic_bridge.w3d\n Texture = bridge.tga\n BridgeModelNameDamaged = damaged.w3d\n TextureDamaged = damaged.tga\n BridgeModelNameReallyDamaged = really_damaged.w3d\n TextureReallyDamaged = really_damaged.tga\n BridgeModelNameBroken = broken.w3d\n TextureBroken = broken.tga\n TowerObjectNameFromLeft = TowerFL\n TowerObjectNameFromRight = TowerFR\n TowerObjectNameToLeft = TowerTL\n TowerObjectNameToRight = TowerTR\nEnd\nRoad DirtRoad\n Texture = old.tga\n RoadWidth = 30\n RoadWidthInTexture = 1.25\n Texture = \"road.dds\"\n Unknown = retained-elsewhere\nEnd\n",
            RoadIniLimits::default(),
        )
        .expect("road INI");
        assert_eq!(ini.definitions().len(), 1);
        let road = &ini.definitions()[0];
        assert_eq!(road.name_bytes(), b"DirtRoad");
        assert_eq!(road.texture_bytes(), Some(b"road.dds".as_slice()));
        assert_eq!(road.road_width().to_bits(), 30.0_f32.to_bits());
        assert_eq!(road.road_width_in_texture().to_bits(), 1.25_f32.to_bits());
        let bridge = &ini.bridges()[0];
        assert_eq!(bridge.name_bytes(), b"SyntheticBridge");
        assert_eq!(
            bridge.model_bytes(),
            Some(b"synthetic_bridge.w3d".as_slice())
        );
        assert_eq!(bridge.bridge_scale().to_bits(), 1.25_f32.to_bits());
        assert_eq!(
            bridge.state_model_bytes(BridgeBodyState::Damaged),
            Some(b"damaged.w3d".as_slice())
        );
        assert_eq!(
            bridge.state_texture_bytes(BridgeBodyState::ReallyDamaged),
            Some(b"really_damaged.tga".as_slice())
        );
        assert_eq!(
            bridge.state_model_bytes(BridgeBodyState::Broken),
            Some(b"broken.w3d".as_slice())
        );
        assert_eq!(
            bridge.tower_object_name_bytes(BridgeTowerSlot::FromLeft),
            Some(b"TowerFL".as_slice())
        );
        assert_eq!(
            bridge.tower_object_name_bytes(BridgeTowerSlot::ToRight),
            Some(b"TowerTR".as_slice())
        );

        let inherited = parse_road_ini(
            b"Bridge DefaultBridge\n BridgeScale = 2\n BridgeModelName = default.w3d\n BridgeModelNameDamaged = default_damaged.w3d\n TextureBroken = default_broken.tga\n TowerObjectNameFromLeft = DefaultTower\nEnd\nBridge Child\n TowerObjectNameToRight = ChildTower\nEnd\n",
            RoadIniLimits::default(),
        )
        .expect("bridge defaults");
        let child = inherited.bridges()[1].inherit_missing(&inherited.bridges()[0]);
        assert_eq!(child.model_bytes(), Some(b"default.w3d".as_slice()));
        assert_eq!(child.bridge_scale().to_bits(), 2.0_f32.to_bits());
        assert_eq!(
            child.state_model_bytes(BridgeBodyState::Damaged),
            Some(b"default_damaged.w3d".as_slice())
        );
        assert_eq!(
            child.state_texture_bytes(BridgeBodyState::Broken),
            Some(b"default_broken.tga".as_slice())
        );
        assert_eq!(
            child.tower_object_name_bytes(BridgeTowerSlot::FromLeft),
            None,
            "source default-bridge inheritance does not copy tower names"
        );
        assert_eq!(
            child.tower_object_name_bytes(BridgeTowerSlot::ToRight),
            Some(b"ChildTower".as_slice())
        );
    }

    #[test]
    fn rejects_structure_reals_and_limits() {
        assert_eq!(
            parse_road_ini(b"Road A\nRoad B\n", RoadIniLimits::default()),
            Err(RoadIniError::NestedRoad { line: 2 })
        );
        assert!(matches!(
            parse_road_ini(b"Road A\n RoadWidth = NaN\nEnd\n", RoadIniLimits::default()),
            Err(RoadIniError::InvalidReal { .. })
        ));
        assert!(matches!(
            parse_road_ini(
                b"Bridge A\n BridgeScale = inf\nEnd\n",
                RoadIniLimits::default()
            ),
            Err(RoadIniError::InvalidReal { .. })
        ));
        let limits = RoadIniLimits {
            max_definitions: 0,
            ..RoadIniLimits::default()
        };
        assert!(matches!(
            parse_road_ini(b"Road A\nEnd\n", limits),
            Err(RoadIniError::TooManyDefinitions { .. })
        ));
    }

    #[test]
    fn bridge_state_and_tower_references_share_explicit_string_limits() {
        let cases = [
            (
                b"Bridge A\n BridgeModelNameDamaged = Four\nEnd\n".as_slice(),
                RoadIniLimits {
                    max_model_bytes: 3,
                    ..RoadIniLimits::default()
                },
                "BridgeModelNameDamaged",
            ),
            (
                b"Bridge A\n TextureBroken = Four\nEnd\n".as_slice(),
                RoadIniLimits {
                    max_texture_bytes: 3,
                    ..RoadIniLimits::default()
                },
                "TextureBroken",
            ),
            (
                b"Bridge A\n TowerObjectNameFromLeft = Four\nEnd\n".as_slice(),
                RoadIniLimits {
                    max_name_bytes: 3,
                    ..RoadIniLimits::default()
                },
                "TowerObjectNameFromLeft",
            ),
        ];
        for (bytes, limits, expected_field) in cases {
            assert!(matches!(
                parse_road_ini(bytes, limits),
                Err(RoadIniError::ValueTooLong { field, .. }) if field == expected_field
            ));
        }
        assert!(matches!(
            parse_road_ini(
                b"Bridge A\n TowerObjectNameToRight =\nEnd\n",
                RoadIniLimits::default()
            ),
            Err(RoadIniError::MissingValue {
                field: "TowerObjectNameToRight",
                ..
            })
        ));
    }
}
