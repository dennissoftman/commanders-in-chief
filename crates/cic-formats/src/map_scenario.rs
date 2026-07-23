// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Immutable MAP world, placement, side, team, build-list, and script data.
//!
//! The binary field order and version gates are derived from `DataChunk.cpp`,
//! `MapUtil.cpp`, `WHeightMapEdit.cpp`, `SidesList.cpp`, and `Scripts.cpp` in
//! `GeneralsGameCode` revision `9f7abb866f5afd446db14149979e744c7216baaf`,
//! licensed under GPL-3.0-or-later with Electronic Arts Section 7 terms. Full
//! notices and permanent source links are in `docs/provenance/map.md`.
//!
//! This module intentionally does not reproduce the upstream validation, repair,
//! opcode dispatch, or object-construction paths. It returns bounded source data.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

use cic_core::{BinaryError, BinaryReader};

use crate::{MapChunk, MapFile};

const CHUNK_HEADER_BYTES: usize = 10;
const WORLD_INFO_LABEL: &[u8] = b"WorldInfo";
const OBJECTS_LIST_LABEL: &[u8] = b"ObjectsList";
const OBJECT_LABEL: &[u8] = b"Object";
const SIDES_LIST_LABEL: &[u8] = b"SidesList";
const PLAYER_SCRIPTS_LABEL: &[u8] = b"PlayerScriptsList";
const SCRIPT_LIST_LABEL: &[u8] = b"ScriptList";
const SCRIPT_GROUP_LABEL: &[u8] = b"ScriptGroup";
const SCRIPT_LABEL: &[u8] = b"Script";
const OR_CONDITION_LABEL: &[u8] = b"OrCondition";
const CONDITION_LABEL: &[u8] = b"Condition";
const ACTION_LABEL: &[u8] = b"ScriptAction";
const FALSE_ACTION_LABEL: &[u8] = b"ScriptActionFalse";
const COORD3D_PARAMETER_TYPE: i32 = 16;

/// Independent limits for immutable scenario metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapScenarioLimits {
    pub maximum_nested_chunks: usize,
    pub maximum_nested_depth: usize,
    pub maximum_dictionary_entries: usize,
    pub maximum_dictionary_entries_total: usize,
    pub maximum_string_bytes: usize,
    pub maximum_unicode_units: usize,
    pub maximum_objects: usize,
    pub maximum_sides: usize,
    pub maximum_teams: usize,
    pub maximum_build_list_entries: usize,
    pub maximum_player_script_lists: usize,
    pub maximum_script_groups: usize,
    pub maximum_scripts: usize,
    pub maximum_or_conditions: usize,
    pub maximum_conditions: usize,
    pub maximum_actions: usize,
    pub maximum_parameters: usize,
    pub maximum_opaque_bytes: usize,
}

impl Default for MapScenarioLimits {
    fn default() -> Self {
        Self {
            maximum_nested_chunks: 1_000_000,
            maximum_nested_depth: 16,
            maximum_dictionary_entries: 32_767,
            maximum_dictionary_entries_total: 1_000_000,
            maximum_string_bytes: 32_767,
            maximum_unicode_units: 32_767,
            maximum_objects: 500_000,
            maximum_sides: 64,
            maximum_teams: 65_536,
            maximum_build_list_entries: 100_000,
            maximum_player_script_lists: 64,
            maximum_script_groups: 100_000,
            maximum_scripts: 500_000,
            maximum_or_conditions: 1_000_000,
            maximum_conditions: 2_000_000,
            maximum_actions: 2_000_000,
            maximum_parameters: 8_000_000,
            maximum_opaque_bytes: 64 * 1_024 * 1_024,
        }
    }
}

/// One typed source dictionary value. Boolean bytes remain raw and Unicode is
/// retained as UTF-16 code units so malformed text is not silently rewritten.
#[derive(Debug, Clone, PartialEq)]
pub enum MapDictionaryValue {
    Bool(u8),
    Int(i32),
    Real(f32),
    Ascii(Vec<u8>),
    Unicode(Vec<u16>),
}

impl MapDictionaryValue {
    #[must_use]
    pub const fn type_code(&self) -> u8 {
        match self {
            Self::Bool(_) => 0,
            Self::Int(_) => 1,
            Self::Real(_) => 2,
            Self::Ascii(_) => 3,
            Self::Unicode(_) => 4,
        }
    }
}

/// One dictionary pair in source order.
#[derive(Debug, Clone, PartialEq)]
pub struct MapDictionaryEntry {
    key_id: u32,
    key_name: Option<Vec<u8>>,
    value: MapDictionaryValue,
}

impl MapDictionaryEntry {
    #[must_use]
    pub const fn key_id(&self) -> u32 {
        self.key_id
    }

    #[must_use]
    pub fn key_name_bytes(&self) -> Option<&[u8]> {
        self.key_name.as_deref()
    }

    #[must_use]
    pub const fn value(&self) -> &MapDictionaryValue {
        &self.value
    }
}

/// Ordered typed dictionary without runtime key interning or repair.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MapDictionary {
    entries: Vec<MapDictionaryEntry>,
}

impl MapDictionary {
    #[must_use]
    pub fn entries(&self) -> &[MapDictionaryEntry] {
        &self.entries
    }

    /// Returns the last source entry matching an ASCII key, mirroring ordinary
    /// dictionary replacement semantics while retaining all source entries.
    #[must_use]
    pub fn last_ascii_case_insensitive(&self, name: &[u8]) -> Option<&MapDictionaryEntry> {
        self.entries.iter().rev().find(|entry| {
            entry
                .key_name_bytes()
                .is_some_and(|key| key.eq_ignore_ascii_case(name))
        })
    }
}

/// One unknown nested chunk preserved without interpreting its payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapOpaqueScenarioChunk {
    id: u32,
    version: u16,
    data: Vec<u8>,
}

impl MapOpaqueScenarioChunk {
    #[must_use]
    pub const fn id(&self) -> u32 {
        self.id
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }

    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }
}

/// Immutable `WorldInfo` dictionary.
#[derive(Debug, Clone, PartialEq)]
pub struct MapWorldInfo {
    version: u16,
    properties: MapDictionary,
}

impl MapWorldInfo {
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }

    #[must_use]
    pub const fn properties(&self) -> &MapDictionary {
        &self.properties
    }
}

/// Stable object flags established by `MapObject.h`.
pub mod object_flags {
    pub const DRAWS_IN_MIRROR: u32 = 0x0000_0001;
    pub const ROAD_POINT1: u32 = 0x0000_0002;
    pub const ROAD_POINT2: u32 = 0x0000_0004;
    pub const ROAD_CORNER_ANGLED: u32 = 0x0000_0008;
    pub const BRIDGE_POINT1: u32 = 0x0000_0010;
    pub const BRIDGE_POINT2: u32 = 0x0000_0020;
    pub const ROAD_CORNER_TIGHT: u32 = 0x0000_0040;
    pub const ROAD_JOIN: u32 = 0x0000_0080;
    pub const DONT_RENDER: u32 = 0x0000_0100;
}

/// One source-ordered nested `Object` placement.
#[derive(Debug, Clone, PartialEq)]
pub struct MapObjectPlacement {
    placement_id: u32,
    version: u16,
    position: [f32; 3],
    angle: f32,
    flags: u32,
    name: Vec<u8>,
    properties: MapDictionary,
}

impl MapObjectPlacement {
    #[must_use]
    pub const fn placement_id(&self) -> u32 {
        self.placement_id
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }

    #[must_use]
    pub const fn position(&self) -> [f32; 3] {
        self.position
    }

    #[must_use]
    pub const fn angle(&self) -> f32 {
        self.angle
    }

    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    #[must_use]
    pub fn name_bytes(&self) -> &[u8] {
        &self.name
    }

    #[must_use]
    pub const fn properties(&self) -> &MapDictionary {
        &self.properties
    }

    #[must_use]
    pub fn waypoint_id(&self) -> Option<i32> {
        match self
            .properties
            .last_ascii_case_insensitive(b"waypointID")?
            .value()
        {
            MapDictionaryValue::Int(value) => Some(*value),
            _ => None,
        }
    }

    #[must_use]
    pub fn waypoint_name_bytes(&self) -> Option<&[u8]> {
        match self
            .properties
            .last_ascii_case_insensitive(b"waypointName")?
            .value()
        {
            MapDictionaryValue::Ascii(value) => Some(value),
            _ => None,
        }
    }

    /// Returns one of the three source waypoint-path labels, when it is a nonempty byte string.
    #[must_use]
    pub fn waypoint_path_label_bytes(&self, slot: usize) -> Option<&[u8]> {
        let names = [
            b"waypointPathLabel1".as_slice(),
            b"waypointPathLabel2".as_slice(),
            b"waypointPathLabel3".as_slice(),
        ];
        let name = names.get(slot)?;
        match self.properties.last_ascii_case_insensitive(name)?.value() {
            MapDictionaryValue::Ascii(value) if !value.is_empty() => Some(value),
            _ => None,
        }
    }

    /// Recognizes the source one-based `Player_n_Start` waypoint convention.
    #[must_use]
    pub fn player_start_number(&self) -> Option<u32> {
        let name = self.waypoint_name_bytes()?;
        let suffix = strip_ascii_prefix_case(name, b"Player_")?;
        let digits = suffix.strip_suffix(b"_Start")?;
        if digits.is_empty() || digits.iter().any(|byte| !byte.is_ascii_digit()) {
            return None;
        }
        let text = std::str::from_utf8(digits).ok()?;
        let number = text.parse::<u32>().ok()?;
        (number != 0).then_some(number)
    }
}

/// Unique world metadata and source-ordered placed objects.
#[derive(Debug, Clone, PartialEq)]
pub struct MapWorldObjects {
    world: MapWorldInfo,
    objects_version: u16,
    objects: Vec<MapObjectPlacement>,
    unknown_object_children: Vec<MapOpaqueScenarioChunk>,
}

impl MapWorldObjects {
    #[must_use]
    pub const fn world(&self) -> &MapWorldInfo {
        &self.world
    }

    #[must_use]
    pub const fn objects_version(&self) -> u16 {
        self.objects_version
    }

    #[must_use]
    pub fn objects(&self) -> &[MapObjectPlacement] {
        &self.objects
    }

    #[must_use]
    pub fn unknown_object_children(&self) -> &[MapOpaqueScenarioChunk] {
        &self.unknown_object_children
    }

    pub fn player_starts(&self) -> impl Iterator<Item = (u32, &MapObjectPlacement)> {
        self.objects
            .iter()
            .filter_map(|object| object.player_start_number().map(|slot| (slot, object)))
    }
}

/// One source build-list entry. R3 retains it as data and never instantiates it.
#[derive(Debug, Clone, PartialEq)]
pub struct MapBuildListEntry {
    building_name: Vec<u8>,
    template_name: Vec<u8>,
    position: [f32; 3],
    angle: f32,
    initially_built: u8,
    rebuild_count: i32,
    script: Option<Vec<u8>>,
    health: Option<i32>,
    whiner: Option<u8>,
    unsellable: Option<u8>,
    repairable: Option<u8>,
}

impl MapBuildListEntry {
    #[must_use]
    pub fn building_name_bytes(&self) -> &[u8] {
        &self.building_name
    }
    #[must_use]
    pub fn template_name_bytes(&self) -> &[u8] {
        &self.template_name
    }
    #[must_use]
    pub const fn position(&self) -> [f32; 3] {
        self.position
    }
    #[must_use]
    pub const fn angle(&self) -> f32 {
        self.angle
    }
    #[must_use]
    pub const fn initially_built_raw(&self) -> u8 {
        self.initially_built
    }
    #[must_use]
    pub const fn rebuild_count(&self) -> i32 {
        self.rebuild_count
    }
    #[must_use]
    pub fn script_bytes(&self) -> Option<&[u8]> {
        self.script.as_deref()
    }
    #[must_use]
    pub const fn health(&self) -> Option<i32> {
        self.health
    }
    #[must_use]
    pub const fn whiner_raw(&self) -> Option<u8> {
        self.whiner
    }
    #[must_use]
    pub const fn unsellable_raw(&self) -> Option<u8> {
        self.unsellable
    }
    #[must_use]
    pub const fn repairable_raw(&self) -> Option<u8> {
        self.repairable
    }
}

/// One side dictionary, build list, and optional corresponding player scripts.
#[derive(Debug, Clone, PartialEq)]
pub struct MapSide {
    properties: MapDictionary,
    build_list: Vec<MapBuildListEntry>,
}

impl MapSide {
    #[must_use]
    pub const fn properties(&self) -> &MapDictionary {
        &self.properties
    }
    #[must_use]
    pub fn build_list(&self) -> &[MapBuildListEntry] {
        &self.build_list
    }
}

/// Script parameter payload preserved without opcode-specific interpretation.
#[derive(Debug, Clone, PartialEq)]
pub enum MapScriptParameterValue {
    Coordinate([f32; 3]),
    Scalar {
        integer: i32,
        real: f32,
        string: Vec<u8>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct MapScriptParameter {
    parameter_type: i32,
    value: MapScriptParameterValue,
}

impl MapScriptParameter {
    #[must_use]
    pub const fn parameter_type(&self) -> i32 {
        self.parameter_type
    }
    #[must_use]
    pub const fn value(&self) -> &MapScriptParameterValue {
        &self.value
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MapScriptCondition {
    version: u16,
    opcode: i32,
    parameters: Vec<MapScriptParameter>,
}

impl MapScriptCondition {
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }
    #[must_use]
    pub const fn opcode(&self) -> i32 {
        self.opcode
    }
    #[must_use]
    pub fn parameters(&self) -> &[MapScriptParameter] {
        &self.parameters
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MapScriptOrCondition {
    version: u16,
    conditions: Vec<MapScriptCondition>,
    unknown_children: Vec<MapOpaqueScenarioChunk>,
}

impl MapScriptOrCondition {
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }
    #[must_use]
    pub fn conditions(&self) -> &[MapScriptCondition] {
        &self.conditions
    }
    #[must_use]
    pub fn unknown_children(&self) -> &[MapOpaqueScenarioChunk] {
        &self.unknown_children
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MapScriptAction {
    version: u16,
    opcode: i32,
    parameters: Vec<MapScriptParameter>,
}

impl MapScriptAction {
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }
    #[must_use]
    pub const fn opcode(&self) -> i32 {
        self.opcode
    }
    #[must_use]
    pub fn parameters(&self) -> &[MapScriptParameter] {
        &self.parameters
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MapScript {
    version: u16,
    name: Vec<u8>,
    comment: Vec<u8>,
    condition_comment: Vec<u8>,
    action_comment: Vec<u8>,
    active: u8,
    one_shot: u8,
    easy: u8,
    normal: u8,
    hard: u8,
    subroutine: u8,
    evaluation_delay_seconds: Option<i32>,
    or_conditions: Vec<MapScriptOrCondition>,
    actions: Vec<MapScriptAction>,
    false_actions: Vec<MapScriptAction>,
    unknown_children: Vec<MapOpaqueScenarioChunk>,
}

impl MapScript {
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }
    #[must_use]
    pub fn name_bytes(&self) -> &[u8] {
        &self.name
    }
    #[must_use]
    pub fn comment_bytes(&self) -> &[u8] {
        &self.comment
    }
    #[must_use]
    pub fn condition_comment_bytes(&self) -> &[u8] {
        &self.condition_comment
    }
    #[must_use]
    pub fn action_comment_bytes(&self) -> &[u8] {
        &self.action_comment
    }
    #[must_use]
    pub const fn active_raw(&self) -> u8 {
        self.active
    }
    #[must_use]
    pub const fn one_shot_raw(&self) -> u8 {
        self.one_shot
    }
    #[must_use]
    pub const fn easy_raw(&self) -> u8 {
        self.easy
    }
    #[must_use]
    pub const fn normal_raw(&self) -> u8 {
        self.normal
    }
    #[must_use]
    pub const fn hard_raw(&self) -> u8 {
        self.hard
    }
    #[must_use]
    pub const fn subroutine_raw(&self) -> u8 {
        self.subroutine
    }
    #[must_use]
    pub const fn evaluation_delay_seconds(&self) -> Option<i32> {
        self.evaluation_delay_seconds
    }
    #[must_use]
    pub fn or_conditions(&self) -> &[MapScriptOrCondition] {
        &self.or_conditions
    }
    #[must_use]
    pub fn actions(&self) -> &[MapScriptAction] {
        &self.actions
    }
    #[must_use]
    pub fn false_actions(&self) -> &[MapScriptAction] {
        &self.false_actions
    }
    #[must_use]
    pub fn unknown_children(&self) -> &[MapOpaqueScenarioChunk] {
        &self.unknown_children
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MapScriptGroup {
    version: u16,
    name: Vec<u8>,
    active: u8,
    subroutine: Option<u8>,
    scripts: Vec<MapScript>,
    unknown_children: Vec<MapOpaqueScenarioChunk>,
}

impl MapScriptGroup {
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }
    #[must_use]
    pub fn name_bytes(&self) -> &[u8] {
        &self.name
    }
    #[must_use]
    pub const fn active_raw(&self) -> u8 {
        self.active
    }
    #[must_use]
    pub const fn subroutine_raw(&self) -> Option<u8> {
        self.subroutine
    }
    #[must_use]
    pub fn scripts(&self) -> &[MapScript] {
        &self.scripts
    }
    #[must_use]
    pub fn unknown_children(&self) -> &[MapOpaqueScenarioChunk] {
        &self.unknown_children
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MapScriptList {
    version: u16,
    scripts: Vec<MapScript>,
    groups: Vec<MapScriptGroup>,
    unknown_children: Vec<MapOpaqueScenarioChunk>,
}

impl MapScriptList {
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }
    #[must_use]
    pub fn scripts(&self) -> &[MapScript] {
        &self.scripts
    }
    #[must_use]
    pub fn groups(&self) -> &[MapScriptGroup] {
        &self.groups
    }
    #[must_use]
    pub fn unknown_children(&self) -> &[MapOpaqueScenarioChunk] {
        &self.unknown_children
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MapPlayerScripts {
    version: u16,
    lists: Vec<MapScriptList>,
    unknown_children: Vec<MapOpaqueScenarioChunk>,
}

impl MapPlayerScripts {
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }
    #[must_use]
    pub fn lists(&self) -> &[MapScriptList] {
        &self.lists
    }
    #[must_use]
    pub fn unknown_children(&self) -> &[MapOpaqueScenarioChunk] {
        &self.unknown_children
    }
}

/// Immutable `SidesList`, including data-only nested scripts.
#[derive(Debug, Clone, PartialEq)]
pub struct MapSidesData {
    version: u16,
    sides: Vec<MapSide>,
    teams: Vec<MapDictionary>,
    player_scripts: Option<MapPlayerScripts>,
    unknown_children: Vec<MapOpaqueScenarioChunk>,
}

impl MapSidesData {
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }
    #[must_use]
    pub fn sides(&self) -> &[MapSide] {
        &self.sides
    }
    #[must_use]
    pub fn teams(&self) -> &[MapDictionary] {
        &self.teams
    }
    #[must_use]
    pub const fn player_scripts(&self) -> Option<&MapPlayerScripts> {
        self.player_scripts.as_ref()
    }
    #[must_use]
    pub fn unknown_children(&self) -> &[MapOpaqueScenarioChunk] {
        &self.unknown_children
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapScenarioError {
    Binary(BinaryError),
    MissingChunk(&'static str),
    DuplicateChunk(&'static str),
    UnsupportedVersion {
        chunk: &'static str,
        version: u16,
    },
    NegativeCount {
        field: &'static str,
        value: i32,
    },
    LimitExceeded {
        what: &'static str,
        actual: usize,
        maximum: usize,
    },
    UnsupportedDictionaryType(u8),
    NonFiniteValue {
        field: &'static str,
        index: usize,
    },
    TruncatedNestedHeader {
        parent: &'static str,
        remaining: usize,
    },
    NegativeNestedSize {
        parent: &'static str,
        value: i32,
    },
    TrailingBytes {
        chunk: &'static str,
        count: usize,
    },
}

impl Display for MapScenarioError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binary(error) => Display::fmt(error, formatter),
            Self::MissingChunk(chunk) => write!(formatter, "MAP has no {chunk} chunk"),
            Self::DuplicateChunk(chunk) => write!(formatter, "MAP has more than one {chunk} chunk"),
            Self::UnsupportedVersion { chunk, version } => {
                write!(formatter, "unsupported {chunk} version {version}")
            }
            Self::NegativeCount { field, value } => {
                write!(formatter, "{field} is negative: {value}")
            }
            Self::LimitExceeded {
                what,
                actual,
                maximum,
            } => write!(
                formatter,
                "{what} value {actual} exceeds the configured limit {maximum}"
            ),
            Self::UnsupportedDictionaryType(value) => {
                write!(formatter, "unsupported MAP dictionary type {value}")
            }
            Self::NonFiniteValue { field, index } => {
                write!(formatter, "{field} value {index} is not finite")
            }
            Self::TruncatedNestedHeader { parent, remaining } => write!(
                formatter,
                "{parent} has {remaining} trailing bytes, fewer than one nested chunk header"
            ),
            Self::NegativeNestedSize { parent, value } => {
                write!(formatter, "{parent} nested chunk size is negative: {value}")
            }
            Self::TrailingBytes { chunk, count } => {
                write!(formatter, "{chunk} has {count} trailing bytes")
            }
        }
    }
}

impl Error for MapScenarioError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Binary(error) => Some(error),
            _ => None,
        }
    }
}

impl From<BinaryError> for MapScenarioError {
    fn from(error: BinaryError) -> Self {
        Self::Binary(error)
    }
}

#[derive(Debug, Clone)]
struct NestedChunk {
    id: u32,
    version: u16,
    data: Vec<u8>,
}

#[derive(Default)]
struct DecodeCounts {
    nested_chunks: usize,
    dictionary_entries: usize,
    opaque_bytes: usize,
    build_entries: usize,
    script_groups: usize,
    scripts: usize,
    or_conditions: usize,
    conditions: usize,
    actions: usize,
    parameters: usize,
}

/// Decodes unique `WorldInfo` and `ObjectsList` chunks into immutable values.
///
/// # Errors
///
/// Returns a structured error for missing, duplicate, unsupported, malformed,
/// non-finite, or configured-limit-exceeding source data.
pub fn decode_map_world_objects(
    map: &MapFile,
    limits: MapScenarioLimits,
) -> Result<MapWorldObjects, MapScenarioError> {
    let world_chunk = unique_chunk(map, WORLD_INFO_LABEL, "WorldInfo")?;
    if world_chunk.version() != 1 {
        return Err(MapScenarioError::UnsupportedVersion {
            chunk: "WorldInfo",
            version: world_chunk.version(),
        });
    }
    let objects_chunk = unique_chunk(map, OBJECTS_LIST_LABEL, "ObjectsList")?;
    if !(1..=3).contains(&objects_chunk.version()) {
        return Err(MapScenarioError::UnsupportedVersion {
            chunk: "ObjectsList",
            version: objects_chunk.version(),
        });
    }
    let mut counts = DecodeCounts::default();
    let mut world_reader = reader_for(world_chunk, "WorldInfo");
    let properties = read_dictionary(&mut world_reader, map, limits, &mut counts, "WorldInfo")?;
    ensure_closed(&world_reader, "WorldInfo")?;
    let world = MapWorldInfo {
        version: world_chunk.version(),
        properties,
    };

    let mut objects_reader = reader_for(objects_chunk, "ObjectsList");
    let children = read_nested_chunks(&mut objects_reader, "ObjectsList", 1, limits, &mut counts)?;
    let mut objects = Vec::new();
    let mut unknown_object_children = Vec::new();
    for child in children {
        if map.symbol_name(child.id) == Some(OBJECT_LABEL) {
            check_next_limit("MAP objects", objects.len(), limits.maximum_objects)?;
            let placement_id =
                u32::try_from(objects.len()).map_err(|_| MapScenarioError::LimitExceeded {
                    what: "MAP objects",
                    actual: objects.len(),
                    maximum: limits.maximum_objects,
                })?;
            objects.push(read_object(&child, placement_id, map, limits, &mut counts)?);
        } else {
            unknown_object_children.push(make_opaque(child, limits, &mut counts)?);
        }
    }
    Ok(MapWorldObjects {
        world,
        objects_version: objects_chunk.version(),
        objects,
        unknown_object_children,
    })
}

/// Decodes unique `SidesList` versions 1 through 3 and its complete nested
/// script tree without evaluating, repairing, or dispatching it.
///
/// # Errors
///
/// Returns a structured error for missing, duplicate, unsupported, malformed,
/// non-finite, or configured-limit-exceeding source data.
pub fn decode_map_sides(
    map: &MapFile,
    limits: MapScenarioLimits,
) -> Result<MapSidesData, MapScenarioError> {
    let chunk = unique_chunk(map, SIDES_LIST_LABEL, "SidesList")?;
    if !(1..=3).contains(&chunk.version()) {
        return Err(MapScenarioError::UnsupportedVersion {
            chunk: "SidesList",
            version: chunk.version(),
        });
    }
    let mut counts = DecodeCounts::default();
    let mut reader = reader_for(chunk, "SidesList");
    let side_count = read_count(&mut reader, "side count", limits.maximum_sides)?;
    let mut sides = Vec::with_capacity(side_count);
    for _ in 0..side_count {
        let properties = read_dictionary(&mut reader, map, limits, &mut counts, "side dictionary")?;
        let build_count = read_count(
            &mut reader,
            "build-list count",
            limits.maximum_build_list_entries,
        )?;
        let following = counts.build_entries.checked_add(build_count).ok_or(
            MapScenarioError::LimitExceeded {
                what: "build-list entries",
                actual: usize::MAX,
                maximum: limits.maximum_build_list_entries,
            },
        )?;
        check_limit(
            "build-list entries",
            following,
            limits.maximum_build_list_entries,
        )?;
        counts.build_entries = following;
        let mut build_list = Vec::with_capacity(build_count);
        for _ in 0..build_count {
            build_list.push(read_build_entry(&mut reader, chunk.version(), limits)?);
        }
        sides.push(MapSide {
            properties,
            build_list,
        });
    }
    let teams = if chunk.version() >= 2 {
        let team_count = read_count(&mut reader, "team count", limits.maximum_teams)?;
        let mut teams = Vec::with_capacity(team_count);
        for _ in 0..team_count {
            teams.push(read_dictionary(
                &mut reader,
                map,
                limits,
                &mut counts,
                "team dictionary",
            )?);
        }
        teams
    } else {
        Vec::new()
    };

    let children = read_nested_chunks(&mut reader, "SidesList", 1, limits, &mut counts)?;
    let mut player_scripts = None;
    let mut unknown_children = Vec::new();
    for child in children {
        if map.symbol_name(child.id) == Some(PLAYER_SCRIPTS_LABEL) {
            if player_scripts.is_some() {
                return Err(MapScenarioError::DuplicateChunk("PlayerScriptsList"));
            }
            player_scripts = Some(read_player_scripts(&child, map, limits, &mut counts, 2)?);
        } else {
            unknown_children.push(make_opaque(child, limits, &mut counts)?);
        }
    }
    Ok(MapSidesData {
        version: chunk.version(),
        sides,
        teams,
        player_scripts,
        unknown_children,
    })
}

fn read_object(
    chunk: &NestedChunk,
    placement_id: u32,
    map: &MapFile,
    limits: MapScenarioLimits,
    counts: &mut DecodeCounts,
) -> Result<MapObjectPlacement, MapScenarioError> {
    if !(1..=3).contains(&chunk.version) {
        return Err(MapScenarioError::UnsupportedVersion {
            chunk: "Object",
            version: chunk.version,
        });
    }
    let mut reader = BinaryReader::new(&chunk.data, format!("Object[{placement_id}]"));
    let position = read_finite3(&mut reader, "object position")?;
    let angle = read_finite(&mut reader, "object angle", placement_id as usize)?;
    let flags = reader.read_u32_le()?;
    let name = read_ascii(&mut reader, limits, "object name")?;
    let properties = if chunk.version >= 2 {
        read_dictionary(&mut reader, map, limits, counts, "object dictionary")?
    } else {
        MapDictionary::default()
    };
    ensure_closed(&reader, "Object")?;
    Ok(MapObjectPlacement {
        placement_id,
        version: chunk.version,
        position,
        angle,
        flags,
        name,
        properties,
    })
}

fn read_build_entry(
    reader: &mut BinaryReader<'_>,
    version: u16,
    limits: MapScenarioLimits,
) -> Result<MapBuildListEntry, MapScenarioError> {
    let building_name = read_ascii(reader, limits, "build-list building name")?;
    let template_name = read_ascii(reader, limits, "build-list template name")?;
    let position = read_finite3(reader, "build-list position")?;
    let angle = read_finite(reader, "build-list angle", 0)?;
    let initially_built = reader.read_u8()?;
    let rebuild_count = read_i32(reader)?;
    let (script, health, whiner, unsellable, repairable) = if version >= 3 {
        (
            Some(read_ascii(reader, limits, "build-list script")?),
            Some(read_i32(reader)?),
            Some(reader.read_u8()?),
            Some(reader.read_u8()?),
            Some(reader.read_u8()?),
        )
    } else {
        (None, None, None, None, None)
    };
    Ok(MapBuildListEntry {
        building_name,
        template_name,
        position,
        angle,
        initially_built,
        rebuild_count,
        script,
        health,
        whiner,
        unsellable,
        repairable,
    })
}

fn read_player_scripts(
    chunk: &NestedChunk,
    map: &MapFile,
    limits: MapScenarioLimits,
    counts: &mut DecodeCounts,
    depth: usize,
) -> Result<MapPlayerScripts, MapScenarioError> {
    require_version("PlayerScriptsList", chunk.version, 1, 1)?;
    let mut reader = BinaryReader::new(&chunk.data, "PlayerScriptsList");
    let children = read_nested_chunks(&mut reader, "PlayerScriptsList", depth, limits, counts)?;
    let mut lists = Vec::new();
    let mut unknown_children = Vec::new();
    for child in children {
        if map.symbol_name(child.id) == Some(SCRIPT_LIST_LABEL) {
            check_next_limit(
                "player script lists",
                lists.len(),
                limits.maximum_player_script_lists,
            )?;
            lists.push(read_script_list(&child, map, limits, counts, depth + 1)?);
        } else {
            unknown_children.push(make_opaque(child, limits, counts)?);
        }
    }
    Ok(MapPlayerScripts {
        version: chunk.version,
        lists,
        unknown_children,
    })
}

fn read_script_list(
    chunk: &NestedChunk,
    map: &MapFile,
    limits: MapScenarioLimits,
    counts: &mut DecodeCounts,
    depth: usize,
) -> Result<MapScriptList, MapScenarioError> {
    require_version("ScriptList", chunk.version, 1, 1)?;
    let mut reader = BinaryReader::new(&chunk.data, "ScriptList");
    let children = read_nested_chunks(&mut reader, "ScriptList", depth, limits, counts)?;
    let mut scripts = Vec::new();
    let mut groups = Vec::new();
    let mut unknown_children = Vec::new();
    for child in children {
        match map.symbol_name(child.id) {
            Some(SCRIPT_LABEL) => {
                scripts.push(read_script(&child, map, limits, counts, depth + 1)?);
            }
            Some(SCRIPT_GROUP_LABEL) => {
                groups.push(read_script_group(&child, map, limits, counts, depth + 1)?);
            }
            _ => unknown_children.push(make_opaque(child, limits, counts)?),
        }
    }
    Ok(MapScriptList {
        version: chunk.version,
        scripts,
        groups,
        unknown_children,
    })
}

fn read_script_group(
    chunk: &NestedChunk,
    map: &MapFile,
    limits: MapScenarioLimits,
    counts: &mut DecodeCounts,
    depth: usize,
) -> Result<MapScriptGroup, MapScenarioError> {
    require_version("ScriptGroup", chunk.version, 1, 2)?;
    check_next_limit(
        "script groups",
        counts.script_groups,
        limits.maximum_script_groups,
    )?;
    counts.script_groups += 1;
    let mut reader = BinaryReader::new(&chunk.data, "ScriptGroup");
    let name = read_ascii(&mut reader, limits, "script-group name")?;
    let active = reader.read_u8()?;
    let subroutine = (chunk.version >= 2).then(|| reader.read_u8()).transpose()?;
    let children = read_nested_chunks(&mut reader, "ScriptGroup", depth, limits, counts)?;
    let mut scripts = Vec::new();
    let mut unknown_children = Vec::new();
    for child in children {
        if map.symbol_name(child.id) == Some(SCRIPT_LABEL) {
            scripts.push(read_script(&child, map, limits, counts, depth + 1)?);
        } else {
            unknown_children.push(make_opaque(child, limits, counts)?);
        }
    }
    Ok(MapScriptGroup {
        version: chunk.version,
        name,
        active,
        subroutine,
        scripts,
        unknown_children,
    })
}

fn read_script(
    chunk: &NestedChunk,
    map: &MapFile,
    limits: MapScenarioLimits,
    counts: &mut DecodeCounts,
    depth: usize,
) -> Result<MapScript, MapScenarioError> {
    require_version("Script", chunk.version, 1, 2)?;
    check_next_limit("scripts", counts.scripts, limits.maximum_scripts)?;
    counts.scripts += 1;
    let mut reader = BinaryReader::new(&chunk.data, "Script");
    let name = read_ascii(&mut reader, limits, "script name")?;
    let comment = read_ascii(&mut reader, limits, "script comment")?;
    let condition_comment = read_ascii(&mut reader, limits, "script condition comment")?;
    let action_comment = read_ascii(&mut reader, limits, "script action comment")?;
    let active = reader.read_u8()?;
    let one_shot = reader.read_u8()?;
    let easy = reader.read_u8()?;
    let normal = reader.read_u8()?;
    let hard = reader.read_u8()?;
    let subroutine = reader.read_u8()?;
    let evaluation_delay_seconds = (chunk.version >= 2)
        .then(|| read_i32(&mut reader))
        .transpose()?;
    let children = read_nested_chunks(&mut reader, "Script", depth, limits, counts)?;
    let mut or_conditions = Vec::new();
    let mut actions = Vec::new();
    let mut false_actions = Vec::new();
    let mut unknown_children = Vec::new();
    for child in children {
        match map.symbol_name(child.id) {
            Some(OR_CONDITION_LABEL) => {
                or_conditions.push(read_or_condition(&child, map, limits, counts, depth + 1)?);
            }
            Some(ACTION_LABEL) => {
                actions.push(read_action(&child, limits, counts, "ScriptAction")?);
            }
            Some(FALSE_ACTION_LABEL) => {
                false_actions.push(read_action(&child, limits, counts, "ScriptActionFalse")?);
            }
            _ => unknown_children.push(make_opaque(child, limits, counts)?),
        }
    }
    Ok(MapScript {
        version: chunk.version,
        name,
        comment,
        condition_comment,
        action_comment,
        active,
        one_shot,
        easy,
        normal,
        hard,
        subroutine,
        evaluation_delay_seconds,
        or_conditions,
        actions,
        false_actions,
        unknown_children,
    })
}

fn read_or_condition(
    chunk: &NestedChunk,
    map: &MapFile,
    limits: MapScenarioLimits,
    counts: &mut DecodeCounts,
    depth: usize,
) -> Result<MapScriptOrCondition, MapScenarioError> {
    require_version("OrCondition", chunk.version, 1, 1)?;
    check_next_limit(
        "OR conditions",
        counts.or_conditions,
        limits.maximum_or_conditions,
    )?;
    counts.or_conditions += 1;
    let mut reader = BinaryReader::new(&chunk.data, "OrCondition");
    let children = read_nested_chunks(&mut reader, "OrCondition", depth, limits, counts)?;
    let mut conditions = Vec::new();
    let mut unknown_children = Vec::new();
    for child in children {
        if map.symbol_name(child.id) == Some(CONDITION_LABEL) {
            conditions.push(read_condition(&child, limits, counts)?);
        } else {
            unknown_children.push(make_opaque(child, limits, counts)?);
        }
    }
    Ok(MapScriptOrCondition {
        version: chunk.version,
        conditions,
        unknown_children,
    })
}

fn read_condition(
    chunk: &NestedChunk,
    limits: MapScenarioLimits,
    counts: &mut DecodeCounts,
) -> Result<MapScriptCondition, MapScenarioError> {
    require_version("Condition", chunk.version, 1, 3)?;
    check_next_limit("conditions", counts.conditions, limits.maximum_conditions)?;
    counts.conditions += 1;
    let mut reader = BinaryReader::new(&chunk.data, "Condition");
    let opcode = read_i32(&mut reader)?;
    let parameters = read_parameters(&mut reader, limits, counts, "condition parameter count")?;
    ensure_closed(&reader, "Condition")?;
    Ok(MapScriptCondition {
        version: chunk.version,
        opcode,
        parameters,
    })
}

fn read_action(
    chunk: &NestedChunk,
    limits: MapScenarioLimits,
    counts: &mut DecodeCounts,
    label: &'static str,
) -> Result<MapScriptAction, MapScenarioError> {
    require_version(label, chunk.version, 1, 1)?;
    check_next_limit("actions", counts.actions, limits.maximum_actions)?;
    counts.actions += 1;
    let mut reader = BinaryReader::new(&chunk.data, label);
    let opcode = read_i32(&mut reader)?;
    let parameters = read_parameters(&mut reader, limits, counts, "action parameter count")?;
    ensure_closed(&reader, label)?;
    Ok(MapScriptAction {
        version: chunk.version,
        opcode,
        parameters,
    })
}

fn read_parameters(
    reader: &mut BinaryReader<'_>,
    limits: MapScenarioLimits,
    counts: &mut DecodeCounts,
    count_field: &'static str,
) -> Result<Vec<MapScriptParameter>, MapScenarioError> {
    let count = read_count(reader, count_field, limits.maximum_parameters)?;
    let following =
        counts
            .parameters
            .checked_add(count)
            .ok_or(MapScenarioError::LimitExceeded {
                what: "script parameters",
                actual: usize::MAX,
                maximum: limits.maximum_parameters,
            })?;
    check_limit("script parameters", following, limits.maximum_parameters)?;
    counts.parameters = following;
    let mut parameters = Vec::with_capacity(count);
    for index in 0..count {
        let parameter_type = read_i32(reader)?;
        let value = if parameter_type == COORD3D_PARAMETER_TYPE {
            MapScriptParameterValue::Coordinate(read_finite3(
                reader,
                "script coordinate parameter",
            )?)
        } else {
            MapScriptParameterValue::Scalar {
                integer: read_i32(reader)?,
                real: read_finite(reader, "script scalar parameter", index)?,
                string: read_ascii(reader, limits, "script parameter string")?,
            }
        };
        parameters.push(MapScriptParameter {
            parameter_type,
            value,
        });
    }
    Ok(parameters)
}

fn read_dictionary(
    reader: &mut BinaryReader<'_>,
    map: &MapFile,
    limits: MapScenarioLimits,
    counts: &mut DecodeCounts,
    context: &'static str,
) -> Result<MapDictionary, MapScenarioError> {
    let count = usize::from(reader.read_u16_le()?);
    check_limit(
        "dictionary entries",
        count,
        limits.maximum_dictionary_entries,
    )?;
    let following =
        counts
            .dictionary_entries
            .checked_add(count)
            .ok_or(MapScenarioError::LimitExceeded {
                what: "total dictionary entries",
                actual: usize::MAX,
                maximum: limits.maximum_dictionary_entries_total,
            })?;
    check_limit(
        "total dictionary entries",
        following,
        limits.maximum_dictionary_entries_total,
    )?;
    counts.dictionary_entries = following;
    let mut entries = Vec::with_capacity(count);
    for index in 0..count {
        let key_and_type = reader.read_u32_le()?;
        let type_code = (key_and_type & 0xff) as u8;
        let key_id = key_and_type >> 8;
        let value = match type_code {
            0 => MapDictionaryValue::Bool(reader.read_u8()?),
            1 => MapDictionaryValue::Int(read_i32(reader)?),
            2 => MapDictionaryValue::Real(read_finite(reader, context, index)?),
            3 => MapDictionaryValue::Ascii(read_ascii(reader, limits, "dictionary ASCII string")?),
            4 => MapDictionaryValue::Unicode(read_unicode(reader, limits)?),
            value => return Err(MapScenarioError::UnsupportedDictionaryType(value)),
        };
        entries.push(MapDictionaryEntry {
            key_id,
            key_name: map.symbol_name(key_id).map(<[u8]>::to_vec),
            value,
        });
    }
    Ok(MapDictionary { entries })
}

fn read_nested_chunks(
    reader: &mut BinaryReader<'_>,
    parent: &'static str,
    depth: usize,
    limits: MapScenarioLimits,
    counts: &mut DecodeCounts,
) -> Result<Vec<NestedChunk>, MapScenarioError> {
    check_limit("scenario nesting depth", depth, limits.maximum_nested_depth)?;
    let mut chunks = Vec::new();
    while reader.remaining() != 0 {
        if reader.remaining() < CHUNK_HEADER_BYTES {
            return Err(MapScenarioError::TruncatedNestedHeader {
                parent,
                remaining: reader.remaining(),
            });
        }
        check_next_limit(
            "nested chunks",
            counts.nested_chunks,
            limits.maximum_nested_chunks,
        )?;
        counts.nested_chunks += 1;
        let id = reader.read_u32_le()?;
        let version = reader.read_u16_le()?;
        let raw_size = read_i32(reader)?;
        if raw_size < 0 {
            return Err(MapScenarioError::NegativeNestedSize {
                parent,
                value: raw_size,
            });
        }
        let size = usize::try_from(raw_size).expect("nonnegative i32 fits usize");
        let data = reader.read_exact(size)?.to_vec();
        chunks.push(NestedChunk { id, version, data });
    }
    Ok(chunks)
}

fn make_opaque(
    chunk: NestedChunk,
    limits: MapScenarioLimits,
    counts: &mut DecodeCounts,
) -> Result<MapOpaqueScenarioChunk, MapScenarioError> {
    let following = counts.opaque_bytes.checked_add(chunk.data.len()).ok_or(
        MapScenarioError::LimitExceeded {
            what: "opaque scenario bytes",
            actual: usize::MAX,
            maximum: limits.maximum_opaque_bytes,
        },
    )?;
    check_limit(
        "opaque scenario bytes",
        following,
        limits.maximum_opaque_bytes,
    )?;
    counts.opaque_bytes = following;
    Ok(MapOpaqueScenarioChunk {
        id: chunk.id,
        version: chunk.version,
        data: chunk.data,
    })
}

fn unique_chunk<'a>(
    map: &'a MapFile,
    label: &[u8],
    display: &'static str,
) -> Result<&'a MapChunk, MapScenarioError> {
    let mut matches = map
        .chunks()
        .iter()
        .filter(|chunk| map.symbol_name(chunk.id()) == Some(label));
    let chunk = matches
        .next()
        .ok_or(MapScenarioError::MissingChunk(display))?;
    if matches.next().is_some() {
        return Err(MapScenarioError::DuplicateChunk(display));
    }
    Ok(chunk)
}

fn reader_for<'a>(chunk: &'a MapChunk, label: &str) -> BinaryReader<'a> {
    BinaryReader::new(
        chunk.data(),
        format!("{label}@{}", chunk.offset() + CHUNK_HEADER_BYTES),
    )
}

fn read_ascii(
    reader: &mut BinaryReader<'_>,
    limits: MapScenarioLimits,
    _field: &'static str,
) -> Result<Vec<u8>, MapScenarioError> {
    let length = usize::from(reader.read_u16_le()?);
    check_limit(
        "MAP scenario string bytes",
        length,
        limits.maximum_string_bytes,
    )?;
    Ok(reader.read_exact(length)?.to_vec())
}

fn read_unicode(
    reader: &mut BinaryReader<'_>,
    limits: MapScenarioLimits,
) -> Result<Vec<u16>, MapScenarioError> {
    let units = usize::from(reader.read_u16_le()?);
    check_limit(
        "MAP scenario Unicode units",
        units,
        limits.maximum_unicode_units,
    )?;
    let bytes = units
        .checked_mul(2)
        .ok_or(MapScenarioError::LimitExceeded {
            what: "MAP scenario Unicode bytes",
            actual: usize::MAX,
            maximum: limits.maximum_unicode_units.saturating_mul(2),
        })?;
    let raw = reader.read_exact(bytes)?;
    Ok(raw
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect())
}

fn read_finite3(
    reader: &mut BinaryReader<'_>,
    field: &'static str,
) -> Result<[f32; 3], MapScenarioError> {
    Ok([
        read_finite(reader, field, 0)?,
        read_finite(reader, field, 1)?,
        read_finite(reader, field, 2)?,
    ])
}

fn read_finite(
    reader: &mut BinaryReader<'_>,
    field: &'static str,
    index: usize,
) -> Result<f32, MapScenarioError> {
    let value = f32::from_bits(reader.read_u32_le()?);
    if !value.is_finite() {
        return Err(MapScenarioError::NonFiniteValue { field, index });
    }
    Ok(value)
}

fn read_i32(reader: &mut BinaryReader<'_>) -> Result<i32, BinaryError> {
    Ok(i32::from_le_bytes(reader.read_u32_le()?.to_le_bytes()))
}

fn read_count(
    reader: &mut BinaryReader<'_>,
    field: &'static str,
    maximum: usize,
) -> Result<usize, MapScenarioError> {
    let value = read_i32(reader)?;
    if value < 0 {
        return Err(MapScenarioError::NegativeCount { field, value });
    }
    let count = usize::try_from(value).expect("nonnegative i32 fits usize");
    check_limit(field, count, maximum)?;
    Ok(count)
}

fn ensure_closed(reader: &BinaryReader<'_>, chunk: &'static str) -> Result<(), MapScenarioError> {
    if reader.remaining() == 0 {
        Ok(())
    } else {
        Err(MapScenarioError::TrailingBytes {
            chunk,
            count: reader.remaining(),
        })
    }
}

fn require_version(
    chunk: &'static str,
    version: u16,
    minimum: u16,
    maximum: u16,
) -> Result<(), MapScenarioError> {
    if (minimum..=maximum).contains(&version) {
        Ok(())
    } else {
        Err(MapScenarioError::UnsupportedVersion { chunk, version })
    }
}

fn check_limit(what: &'static str, actual: usize, maximum: usize) -> Result<(), MapScenarioError> {
    if actual > maximum {
        Err(MapScenarioError::LimitExceeded {
            what,
            actual,
            maximum,
        })
    } else {
        Ok(())
    }
}

fn check_next_limit(
    what: &'static str,
    current: usize,
    maximum: usize,
) -> Result<(), MapScenarioError> {
    let actual = current
        .checked_add(1)
        .ok_or(MapScenarioError::LimitExceeded {
            what,
            actual: usize::MAX,
            maximum,
        })?;
    check_limit(what, actual, maximum)
}

fn strip_ascii_prefix_case<'a>(value: &'a [u8], prefix: &[u8]) -> Option<&'a [u8]> {
    let candidate = value.get(..prefix.len())?;
    candidate
        .eq_ignore_ascii_case(prefix)
        .then(|| &value[prefix.len()..])
}

#[cfg(test)]
mod tests {
    use super::{
        MapDictionaryValue, MapScenarioError, MapScenarioLimits, MapScriptParameterValue,
        decode_map_sides, decode_map_world_objects, object_flags,
    };
    use crate::{MapLimits, parse_map};

    const WORLD_ID: u32 = 1;
    const OBJECTS_ID: u32 = 2;
    const OBJECT_ID: u32 = 3;
    const SIDES_ID: u32 = 4;
    const PLAYER_SCRIPTS_ID: u32 = 5;
    const SCRIPT_LIST_ID: u32 = 6;
    const SCRIPT_GROUP_ID: u32 = 7;
    const SCRIPT_ID: u32 = 8;
    const OR_ID: u32 = 9;
    const CONDITION_ID: u32 = 10;
    const ACTION_ID: u32 = 11;
    const FALSE_ACTION_ID: u32 = 12;
    const WAYPOINT_ID_KEY: u32 = 20;
    const WAYPOINT_NAME_KEY: u32 = 21;
    const BOOL_KEY: u32 = 22;
    const REAL_KEY: u32 = 23;
    const UNICODE_KEY: u32 = 24;
    const WAYPOINT_PATH_1_KEY: u32 = 25;
    const WAYPOINT_PATH_2_KEY: u32 = 26;
    const WAYPOINT_PATH_3_KEY: u32 = 27;
    const UNKNOWN_CHILD_ID: u32 = 30;

    fn symbols() -> Vec<(u32, &'static [u8])> {
        vec![
            (WORLD_ID, b"WorldInfo"),
            (OBJECTS_ID, b"ObjectsList"),
            (OBJECT_ID, b"Object"),
            (SIDES_ID, b"SidesList"),
            (PLAYER_SCRIPTS_ID, b"PlayerScriptsList"),
            (SCRIPT_LIST_ID, b"ScriptList"),
            (SCRIPT_GROUP_ID, b"ScriptGroup"),
            (SCRIPT_ID, b"Script"),
            (OR_ID, b"OrCondition"),
            (CONDITION_ID, b"Condition"),
            (ACTION_ID, b"ScriptAction"),
            (FALSE_ACTION_ID, b"ScriptActionFalse"),
            (WAYPOINT_ID_KEY, b"waypointID"),
            (WAYPOINT_NAME_KEY, b"waypointName"),
            (BOOL_KEY, b"rawBool"),
            (REAL_KEY, b"realValue"),
            (UNICODE_KEY, b"unicodeValue"),
            (WAYPOINT_PATH_1_KEY, b"waypointPathLabel1"),
            (WAYPOINT_PATH_2_KEY, b"waypointPathLabel2"),
            (WAYPOINT_PATH_3_KEY, b"waypointPathLabel3"),
            (UNKNOWN_CHILD_ID, b"FutureChunk"),
        ]
    }

    fn map_bytes(chunks: &[(u32, u16, Vec<u8>)]) -> Vec<u8> {
        let symbols = symbols();
        let mut bytes = b"CkMp".to_vec();
        bytes.extend_from_slice(
            &i32::try_from(symbols.len())
                .expect("symbols fit")
                .to_le_bytes(),
        );
        for (id, name) in symbols {
            bytes.push(u8::try_from(name.len()).expect("symbol length fits"));
            bytes.extend_from_slice(name);
            bytes.extend_from_slice(&id.to_le_bytes());
        }
        for (id, version, data) in chunks {
            bytes.extend_from_slice(&id.to_le_bytes());
            bytes.extend_from_slice(&version.to_le_bytes());
            bytes.extend_from_slice(
                &i32::try_from(data.len())
                    .expect("payload fits")
                    .to_le_bytes(),
            );
            bytes.extend_from_slice(data);
        }
        bytes
    }

    fn nested(id: u32, version: u16, data: &[u8]) -> Vec<u8> {
        let mut bytes = id.to_le_bytes().to_vec();
        bytes.extend_from_slice(&version.to_le_bytes());
        bytes.extend_from_slice(
            &i32::try_from(data.len())
                .expect("nested payload fits")
                .to_le_bytes(),
        );
        bytes.extend_from_slice(data);
        bytes
    }

    fn ascii(bytes: &[u8]) -> Vec<u8> {
        let mut output = u16::try_from(bytes.len())
            .expect("test string fits")
            .to_le_bytes()
            .to_vec();
        output.extend_from_slice(bytes);
        output
    }

    fn dict(entries: &[(u32, u8, Vec<u8>)]) -> Vec<u8> {
        let mut bytes = u16::try_from(entries.len())
            .expect("dict fits")
            .to_le_bytes()
            .to_vec();
        for (key, value_type, value) in entries {
            bytes.extend_from_slice(&((key << 8) | u32::from(*value_type)).to_le_bytes());
            bytes.extend_from_slice(value);
        }
        bytes
    }

    fn object_payload() -> Vec<u8> {
        let mut data = Vec::new();
        for value in [10.0_f32, 20.0, 3.5, 1.25] {
            data.extend_from_slice(&value.to_le_bytes());
        }
        data.extend_from_slice(
            &(object_flags::DRAWS_IN_MIRROR | object_flags::ROAD_POINT1).to_le_bytes(),
        );
        data.extend_from_slice(&ascii(b"SyntheticMarker"));
        data.extend_from_slice(&dict(&[
            (WAYPOINT_ID_KEY, 1, 7_i32.to_le_bytes().to_vec()),
            (WAYPOINT_NAME_KEY, 3, ascii(b"Player_2_Start")),
            (WAYPOINT_PATH_1_KEY, 3, ascii(b"Patrol East")),
            (WAYPOINT_PATH_2_KEY, 3, ascii(b"Patrol Shared")),
            (WAYPOINT_PATH_3_KEY, 3, ascii(b"")),
        ]));
        data
    }

    fn world_payload() -> Vec<u8> {
        let mut unicode = 2_u16.to_le_bytes().to_vec();
        unicode.extend_from_slice(&[b'A', 0, 0x00, 0xD8]);
        dict(&[
            (BOOL_KEY, 0, vec![2]),
            (REAL_KEY, 2, 1.5_f32.to_le_bytes().to_vec()),
            (UNICODE_KEY, 4, unicode),
        ])
    }

    fn parse(chunks: &[(u32, u16, Vec<u8>)]) -> crate::MapFile {
        parse_map(&map_bytes(chunks), "scenario.map", MapLimits::default()).expect("MAP inventory")
    }

    #[test]
    fn decodes_world_objects_waypoints_and_unknown_children_losslessly() {
        let mut object_list = nested(OBJECT_ID, 3, &object_payload());
        object_list.extend_from_slice(&nested(UNKNOWN_CHILD_ID, 7, &[1, 2, 3]));
        let map = parse(&[(WORLD_ID, 1, world_payload()), (OBJECTS_ID, 3, object_list)]);
        let decoded =
            decode_map_world_objects(&map, MapScenarioLimits::default()).expect("world objects");
        assert_eq!(decoded.world().properties().entries().len(), 3);
        assert_eq!(
            decoded.world().properties().entries()[0].value(),
            &MapDictionaryValue::Bool(2)
        );
        assert_eq!(decoded.objects().len(), 1);
        let object = &decoded.objects()[0];
        assert_eq!(object.placement_id(), 0);
        assert_eq!(
            object.position().map(f32::to_bits),
            [10.0_f32.to_bits(), 20.0_f32.to_bits(), 3.5_f32.to_bits()]
        );
        assert_eq!(object.waypoint_id(), Some(7));
        assert_eq!(
            object.waypoint_name_bytes(),
            Some(b"Player_2_Start".as_slice())
        );
        assert_eq!(object.player_start_number(), Some(2));
        assert_eq!(
            object.waypoint_path_label_bytes(0),
            Some(b"Patrol East".as_slice())
        );
        assert_eq!(
            object.waypoint_path_label_bytes(1),
            Some(b"Patrol Shared".as_slice())
        );
        assert_eq!(object.waypoint_path_label_bytes(2), None);
        assert_eq!(object.waypoint_path_label_bytes(3), None);
        assert_eq!(
            decoded
                .player_starts()
                .map(|(slot, _)| slot)
                .collect::<Vec<_>>(),
            [2]
        );
        assert_eq!(decoded.unknown_object_children()[0].data(), [1, 2, 3]);
    }

    #[test]
    fn object_decoder_rejects_every_truncated_semantic_prefix_and_limits() {
        let complete = object_payload();
        for length in 0..complete.len() {
            let map = parse(&[
                (WORLD_ID, 1, world_payload()),
                (OBJECTS_ID, 3, nested(OBJECT_ID, 3, &complete[..length])),
            ]);
            assert!(
                decode_map_world_objects(&map, MapScenarioLimits::default()).is_err(),
                "prefix {length}"
            );
        }
        let map = parse(&[
            (WORLD_ID, 1, world_payload()),
            (OBJECTS_ID, 3, nested(OBJECT_ID, 3, &complete)),
        ]);
        let limits = MapScenarioLimits {
            maximum_objects: 0,
            ..MapScenarioLimits::default()
        };
        assert!(matches!(
            decode_map_world_objects(&map, limits),
            Err(MapScenarioError::LimitExceeded {
                what: "MAP objects",
                ..
            })
        ));
    }

    fn parameter_scalar(kind: i32, integer: i32, real: f32, string: &[u8]) -> Vec<u8> {
        let mut bytes = kind.to_le_bytes().to_vec();
        bytes.extend_from_slice(&integer.to_le_bytes());
        bytes.extend_from_slice(&real.to_le_bytes());
        bytes.extend_from_slice(&ascii(string));
        bytes
    }

    fn parameter_coord(position: [f32; 3]) -> Vec<u8> {
        let mut bytes = 16_i32.to_le_bytes().to_vec();
        for value in position {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    fn opcode_payload(opcode: i32, parameters: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes = opcode.to_le_bytes().to_vec();
        bytes.extend_from_slice(
            &i32::try_from(parameters.len())
                .expect("parameter count fits")
                .to_le_bytes(),
        );
        for parameter in parameters {
            bytes.extend_from_slice(parameter);
        }
        bytes
    }

    fn script_payload() -> Vec<u8> {
        let mut payload = Vec::new();
        for text in [
            b"Inspect only".as_slice(),
            b"comment",
            b"condition",
            b"action",
        ] {
            payload.extend_from_slice(&ascii(text));
        }
        payload.extend_from_slice(&[1, 0, 1, 1, 0, 1]);
        payload.extend_from_slice(&5_i32.to_le_bytes());
        let condition = opcode_payload(123, &[parameter_scalar(99, 4, 2.5, b"raw")]);
        let or_condition = nested(CONDITION_ID, 1, &condition);
        payload.extend_from_slice(&nested(OR_ID, 1, &or_condition));
        let action = opcode_payload(456, &[parameter_coord([1.0, 2.0, 3.0])]);
        payload.extend_from_slice(&nested(ACTION_ID, 1, &action));
        payload.extend_from_slice(&nested(FALSE_ACTION_ID, 1, &opcode_payload(789, &[])));
        payload
    }

    fn sides_payload() -> Vec<u8> {
        let mut payload = 1_i32.to_le_bytes().to_vec();
        payload.extend_from_slice(&dict(&[(BOOL_KEY, 0, vec![1])]));
        payload.extend_from_slice(&1_i32.to_le_bytes());
        payload.extend_from_slice(&ascii(b"BuildOne"));
        payload.extend_from_slice(&ascii(b"SyntheticBuilding"));
        for value in [8.0_f32, 9.0, 0.0, 0.5] {
            payload.extend_from_slice(&value.to_le_bytes());
        }
        payload.push(1);
        payload.extend_from_slice(&2_i32.to_le_bytes());
        payload.extend_from_slice(&ascii(b"OnBuilt"));
        payload.extend_from_slice(&75_i32.to_le_bytes());
        payload.extend_from_slice(&[1, 0, 1]);
        payload.extend_from_slice(&1_i32.to_le_bytes());
        payload.extend_from_slice(&dict(&[(BOOL_KEY, 0, vec![1])]));
        let mut list = nested(SCRIPT_ID, 2, &script_payload());
        let mut group = ascii(b"Group");
        group.extend_from_slice(&[1, 1]);
        group.extend_from_slice(&nested(SCRIPT_ID, 2, &script_payload()));
        list.extend_from_slice(&nested(SCRIPT_GROUP_ID, 2, &group));
        let player_scripts = nested(SCRIPT_LIST_ID, 1, &list);
        payload.extend_from_slice(&nested(PLAYER_SCRIPTS_ID, 1, &player_scripts));
        payload
    }

    #[test]
    fn decodes_sides_teams_build_lists_and_scripts_without_execution_or_repair() {
        let map = parse(&[(SIDES_ID, 3, sides_payload())]);
        let decoded = decode_map_sides(&map, MapScenarioLimits::default()).expect("sides");
        assert_eq!(decoded.sides().len(), 1);
        assert_eq!(decoded.teams().len(), 1);
        let build = &decoded.sides()[0].build_list()[0];
        assert_eq!(build.template_name_bytes(), b"SyntheticBuilding");
        assert_eq!(build.health(), Some(75));
        let lists = decoded.player_scripts().expect("scripts").lists();
        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].groups().len(), 1);
        let script = &lists[0].scripts()[0];
        assert_eq!(script.name_bytes(), b"Inspect only");
        assert_eq!(script.evaluation_delay_seconds(), Some(5));
        assert_eq!(script.or_conditions()[0].conditions()[0].opcode(), 123);
        assert_eq!(
            script.or_conditions()[0].conditions()[0].parameters().len(),
            1
        );
        assert_eq!(script.actions()[0].opcode(), 456);
        assert!(
            matches!(script.actions()[0].parameters()[0].value(), MapScriptParameterValue::Coordinate(position) if position.map(f32::to_bits) == [1.0_f32.to_bits(), 2.0_f32.to_bits(), 3.0_f32.to_bits()])
        );
        assert_eq!(script.false_actions()[0].opcode(), 789);
    }

    #[test]
    fn sides_decoder_rejects_truncation_non_finite_values_and_script_limits() {
        let complete = sides_payload();
        // The first 92 bytes are the required side/build/team records. A payload
        // ending there is valid because nested PlayerScriptsList data is optional.
        for length in 0..92 {
            let map = parse(&[(SIDES_ID, 3, complete[..length].to_vec())]);
            assert!(
                decode_map_sides(&map, MapScenarioLimits::default()).is_err(),
                "prefix {length}"
            );
        }
        let truncated_nested = parse(&[(SIDES_ID, 3, complete[..complete.len() - 1].to_vec())]);
        assert!(decode_map_sides(&truncated_nested, MapScenarioLimits::default()).is_err());
        let map = parse(&[(SIDES_ID, 3, complete)]);
        let limits = MapScenarioLimits {
            maximum_scripts: 0,
            ..MapScenarioLimits::default()
        };
        assert!(matches!(
            decode_map_sides(&map, limits),
            Err(MapScenarioError::LimitExceeded {
                what: "scripts",
                ..
            })
        ));

        let mut invalid = sides_payload();
        let position = invalid
            .windows(4)
            .position(|bytes| bytes == 8.0_f32.to_le_bytes())
            .expect("build position");
        invalid[position..position + 4].copy_from_slice(&f32::NAN.to_le_bytes());
        let map = parse(&[(SIDES_ID, 3, invalid)]);
        assert!(matches!(
            decode_map_sides(&map, MapScenarioLimits::default()),
            Err(MapScenarioError::NonFiniteValue {
                field: "build-list position",
                ..
            })
        ));
    }
}
