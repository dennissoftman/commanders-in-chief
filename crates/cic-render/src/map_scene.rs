// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Stable pre-simulation staging for immutable MAP placements.
//!
//! Endpoint flag meanings are derived from `MapObject.h` in `GeneralsGameCode`
//! revision `9f7abb866f5afd446db14149979e744c7216baaf`, licensed under
//! GPL-3.0-or-later with Electronic Arts Section 7 terms. This module only
//! classifies source records; definition resolution and geometry remain later
//! presentation steps and no live object or script state is created.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

use cic_formats::{MapObjectPlacement, MapWorldObjects, object_flags};

const MAX_STAGED_PLACEMENTS: usize = 500_000;

/// Explicit presentation time supplied by a tool or window layer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapPresentationFrame {
    seconds: f32,
}

impl MapPresentationFrame {
    pub const ZERO: Self = Self { seconds: 0.0 };

    /// Creates a deterministic nonnegative presentation-time input.
    ///
    /// # Errors
    ///
    /// Returns [`MapSceneStagingError::InvalidPresentationTime`] for negative
    /// or non-finite seconds.
    pub fn new(seconds: f32) -> Result<Self, MapSceneStagingError> {
        if !seconds.is_finite() || seconds < 0.0 {
            return Err(MapSceneStagingError::InvalidPresentationTime);
        }
        Ok(Self { seconds })
    }

    #[must_use]
    pub const fn seconds(self) -> f32 {
        self.seconds
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapEndpointKind {
    RoadFirst,
    RoadSecond,
    BridgeFirst,
    BridgeSecond,
}

/// One renderer-neutral placement copied from immutable format data.
#[derive(Debug, Clone, PartialEq)]
pub struct StagedMapPlacement {
    placement_id: u32,
    position: [f32; 3],
    angle: f32,
    flags: u32,
    template_name: Vec<u8>,
}

impl StagedMapPlacement {
    fn from_object(object: &MapObjectPlacement) -> Self {
        Self {
            placement_id: object.placement_id(),
            position: object.position(),
            angle: object.angle(),
            flags: object.flags(),
            template_name: object.name_bytes().to_vec(),
        }
    }

    #[must_use]
    pub const fn placement_id(&self) -> u32 {
        self.placement_id
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
    pub fn template_name_bytes(&self) -> &[u8] {
        &self.template_name
    }
    #[must_use]
    pub const fn draws_in_mirror(&self) -> bool {
        self.flags & object_flags::DRAWS_IN_MIRROR != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StagedMapEndpoint {
    placement_index: u32,
    kind: MapEndpointKind,
}

impl StagedMapEndpoint {
    #[must_use]
    pub const fn placement_index(self) -> u32 {
        self.placement_index
    }
    #[must_use]
    pub const fn kind(self) -> MapEndpointKind {
        self.kind
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StagedWaypoint {
    placement_index: u32,
    waypoint_id: i32,
    player_start: Option<u32>,
}

impl StagedWaypoint {
    #[must_use]
    pub const fn placement_index(self) -> u32 {
        self.placement_index
    }
    #[must_use]
    pub const fn waypoint_id(self) -> i32 {
        self.waypoint_id
    }
    #[must_use]
    pub const fn player_start(self) -> Option<u32> {
        self.player_start
    }
}

/// Stable buckets that unlock later road/bridge and scenery definition resolution.
#[derive(Debug, Clone, PartialEq)]
pub struct StagedMapScene {
    placements: Vec<StagedMapPlacement>,
    endpoints: Vec<StagedMapEndpoint>,
    scenery_indices: Vec<u32>,
    hidden_indices: Vec<u32>,
    waypoints: Vec<StagedWaypoint>,
    ambiguous_endpoint_indices: Vec<u32>,
}

impl StagedMapScene {
    /// Copies and classifies placements in authoritative MAP object order.
    ///
    /// Roads and bridges remain endpoint records until bounded definition
    /// resolution supplies widths/models. All other visible records enter the
    /// scenery bucket used by buildings, vegetation, props, and decals.
    ///
    /// # Errors
    ///
    /// Returns [`MapSceneStagingError::TooManyPlacements`] when the decoded
    /// object list exceeds the renderer staging limit.
    pub fn from_world_objects(world: &MapWorldObjects) -> Result<Self, MapSceneStagingError> {
        if world.objects().len() > MAX_STAGED_PLACEMENTS {
            return Err(MapSceneStagingError::TooManyPlacements(
                world.objects().len(),
            ));
        }
        let mut placements = Vec::with_capacity(world.objects().len());
        let mut endpoints = Vec::new();
        let mut scenery_indices = Vec::new();
        let mut hidden_indices = Vec::new();
        let mut waypoints = Vec::new();
        let mut ambiguous_endpoint_indices = Vec::new();

        for object in world.objects() {
            let index = u32::try_from(placements.len())
                .map_err(|_| MapSceneStagingError::TooManyPlacements(placements.len()))?;
            placements.push(StagedMapPlacement::from_object(object));
            if let Some(waypoint_id) = object.waypoint_id() {
                waypoints.push(StagedWaypoint {
                    placement_index: index,
                    waypoint_id,
                    player_start: object.player_start_number(),
                });
            }

            let endpoint_flags = object.flags()
                & (object_flags::ROAD_POINT1
                    | object_flags::ROAD_POINT2
                    | object_flags::BRIDGE_POINT1
                    | object_flags::BRIDGE_POINT2);
            let kind = match endpoint_flags {
                object_flags::ROAD_POINT1 => Some(MapEndpointKind::RoadFirst),
                object_flags::ROAD_POINT2 => Some(MapEndpointKind::RoadSecond),
                object_flags::BRIDGE_POINT1 => Some(MapEndpointKind::BridgeFirst),
                object_flags::BRIDGE_POINT2 => Some(MapEndpointKind::BridgeSecond),
                0 => None,
                _ => {
                    ambiguous_endpoint_indices.push(index);
                    None
                }
            };
            if let Some(kind) = kind {
                endpoints.push(StagedMapEndpoint {
                    placement_index: index,
                    kind,
                });
            } else if object.flags() & object_flags::DONT_RENDER != 0 {
                hidden_indices.push(index);
            } else if object.waypoint_id().is_none() && endpoint_flags == 0 {
                scenery_indices.push(index);
            }
        }
        Ok(Self {
            placements,
            endpoints,
            scenery_indices,
            hidden_indices,
            waypoints,
            ambiguous_endpoint_indices,
        })
    }

    #[must_use]
    pub fn placements(&self) -> &[StagedMapPlacement] {
        &self.placements
    }
    #[must_use]
    pub fn endpoints(&self) -> &[StagedMapEndpoint] {
        &self.endpoints
    }
    #[must_use]
    pub fn scenery_indices(&self) -> &[u32] {
        &self.scenery_indices
    }
    #[must_use]
    pub fn hidden_indices(&self) -> &[u32] {
        &self.hidden_indices
    }
    #[must_use]
    pub fn waypoints(&self) -> &[StagedWaypoint] {
        &self.waypoints
    }
    #[must_use]
    pub fn ambiguous_endpoint_indices(&self) -> &[u32] {
        &self.ambiguous_endpoint_indices
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapSceneStagingError {
    TooManyPlacements(usize),
    InvalidPresentationTime,
}

impl Display for MapSceneStagingError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyPlacements(count) => write!(
                formatter,
                "MAP scene has {count} placements; maximum is {MAX_STAGED_PLACEMENTS}"
            ),
            Self::InvalidPresentationTime => {
                formatter.write_str("MAP presentation time must be finite and nonnegative")
            }
        }
    }
}

impl Error for MapSceneStagingError {}

#[cfg(test)]
mod tests {
    use cic_formats::{MapLimits, MapScenarioLimits, decode_map_world_objects, parse_map};

    use super::{MapEndpointKind, MapPresentationFrame, StagedMapScene};

    #[test]
    fn stages_endpoints_scenery_waypoints_and_explicit_time_in_source_order() {
        let map = parse_map(&fixture(), "scene.map", MapLimits::default()).expect("map");
        let world = decode_map_world_objects(&map, MapScenarioLimits::default()).expect("objects");
        let scene = StagedMapScene::from_world_objects(&world).expect("stage");
        assert_eq!(scene.placements().len(), 4);
        assert_eq!(scene.endpoints().len(), 1);
        assert_eq!(scene.endpoints()[0].kind(), MapEndpointKind::RoadFirst);
        assert_eq!(scene.scenery_indices(), [1]);
        assert_eq!(scene.waypoints()[0].player_start(), Some(1));
        assert_eq!(scene.hidden_indices(), [3]);
        assert_eq!(scene.ambiguous_endpoint_indices(), [2]);
        assert_eq!(
            MapPresentationFrame::new(1.25)
                .expect("time")
                .seconds()
                .to_bits(),
            1.25_f32.to_bits()
        );
        assert!(MapPresentationFrame::new(f32::NAN).is_err());
    }

    fn fixture() -> Vec<u8> {
        let symbols = [
            (1_u32, b"WorldInfo".as_slice()),
            (2, b"ObjectsList".as_slice()),
            (3, b"Object".as_slice()),
            (4, b"waypointID".as_slice()),
            (5, b"waypointName".as_slice()),
        ];
        let mut bytes = b"CkMp".to_vec();
        bytes.extend_from_slice(
            &i32::try_from(symbols.len())
                .expect("symbols fit")
                .to_le_bytes(),
        );
        for (id, name) in symbols {
            bytes.push(u8::try_from(name.len()).expect("name fits"));
            bytes.extend_from_slice(name);
            bytes.extend_from_slice(&id.to_le_bytes());
        }
        push_chunk(&mut bytes, 1, 1, &0_u16.to_le_bytes());
        let mut objects = Vec::new();
        objects.extend_from_slice(&object(0x2, b"Road", &[]));
        objects.extend_from_slice(&object(0, b"Tree", &[]));
        objects.extend_from_slice(&object(0x2 | 0x4, b"BadRoad", &[]));
        let mut waypoint = 2_u16.to_le_bytes().to_vec();
        waypoint.extend_from_slice(&((4_u32 << 8) | 1).to_le_bytes());
        waypoint.extend_from_slice(&1_i32.to_le_bytes());
        waypoint.extend_from_slice(&((5_u32 << 8) | 3).to_le_bytes());
        waypoint.extend_from_slice(&ascii(b"Player_1_Start"));
        objects.extend_from_slice(&object(0x100, b"Waypoint", &waypoint));
        push_chunk(&mut bytes, 2, 3, &objects);
        bytes
    }

    fn object(flags: u32, name: &[u8], dict: &[u8]) -> Vec<u8> {
        let mut payload = Vec::new();
        for value in [1.0_f32, 2.0, 3.0, 0.0] {
            payload.extend_from_slice(&value.to_le_bytes());
        }
        payload.extend_from_slice(&flags.to_le_bytes());
        payload.extend_from_slice(&ascii(name));
        if dict.is_empty() {
            payload.extend_from_slice(&0_u16.to_le_bytes());
        } else {
            payload.extend_from_slice(dict);
        }
        let mut chunk = 3_u32.to_le_bytes().to_vec();
        chunk.extend_from_slice(&3_u16.to_le_bytes());
        chunk.extend_from_slice(
            &i32::try_from(payload.len())
                .expect("payload fits")
                .to_le_bytes(),
        );
        chunk.extend_from_slice(&payload);
        chunk
    }

    fn ascii(value: &[u8]) -> Vec<u8> {
        let mut bytes = u16::try_from(value.len())
            .expect("string fits")
            .to_le_bytes()
            .to_vec();
        bytes.extend_from_slice(value);
        bytes
    }

    fn push_chunk(bytes: &mut Vec<u8>, id: u32, version: u16, payload: &[u8]) {
        bytes.extend_from_slice(&id.to_le_bytes());
        bytes.extend_from_slice(&version.to_le_bytes());
        bytes.extend_from_slice(
            &i32::try_from(payload.len())
                .expect("payload fits")
                .to_le_bytes(),
        );
        bytes.extend_from_slice(payload);
    }
}
