// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Renderer-only MAP diagnostics for waypoints, player starts, and polygon areas.
//!
//! Waypoint/start classification and polygon field order follow the immutable source-derived
//! values documented in `docs/provenance/map.md` at `GeneralsGameCode` revision
//! `9f7abb866f5afd446db14149979e744c7216baaf`. The octahedral markers, translucent zone walls,
//! waypoint-path ribbons, colors, and dimensions are project-authored diagnostics; they create no
//! simulation state.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use cic_formats::{MapPolygonArea, MapPolygonData, MapWorldObjects};

use crate::{StagedMapScene, StagedTerrain};

const MAX_OVERLAY_VERTICES: usize = 4_500_000;
const MAX_OVERLAY_INDICES: usize = 6_750_000;
const WAYPOINT_RADIUS: f32 = 4.0;
const WAYPOINT_HEIGHT: f32 = 22.0;
const SPAWN_RADIUS: f32 = 8.0;
const SPAWN_HEIGHT: f32 = 44.0;
const MARKER_GROUND_OFFSET: f32 = 0.6;
const ZONE_GROUND_OFFSET: f32 = 0.35;
const ZONE_HEIGHT: f32 = 14.0;
const PATH_GROUND_OFFSET: f32 = 1.1;
const PATH_HALF_WIDTH: f32 = 2.25;
const PATH_TERRAIN_STEP: f32 = 10.0;
const MAX_PATH_SUBDIVISIONS_PER_EDGE: usize = 4_096;
const SPAWN_COLORS: [[f32; 3]; 8] = [
    [1.0, 0.82, 0.12],
    [1.0, 0.28, 0.18],
    [0.25, 0.65, 1.0],
    [0.35, 1.0, 0.35],
    [0.92, 0.35, 1.0],
    [1.0, 0.55, 0.12],
    [0.45, 1.0, 0.88],
    [0.95, 0.95, 0.95],
];
const POLYGON_COLORS: [[f32; 3]; 6] = [
    [1.0, 0.20, 0.65],
    [0.75, 0.32, 1.0],
    [1.0, 0.55, 0.10],
    [0.35, 1.0, 0.45],
    [1.0, 0.92, 0.18],
    [0.15, 0.85, 1.0],
];

/// One world-space vertex for renderer-only MAP diagnostics.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapOverlayVertex {
    position: [f32; 3],
    color: [f32; 4],
}

impl MapOverlayVertex {
    #[must_use]
    pub const fn position(self) -> [f32; 3] {
        self.position
    }

    #[must_use]
    pub const fn color(self) -> [f32; 4] {
        self.color
    }
}

/// Bounded, source-ordered diagnostic geometry for one loaded MAP.
#[derive(Debug, Clone, PartialEq)]
pub struct StagedMapOverlays {
    vertices: Vec<MapOverlayVertex>,
    indices: Vec<u32>,
    waypoint_count: usize,
    spawn_count: usize,
    waypoint_path_count: usize,
    waypoint_path_segment_count: usize,
    polygon_count: usize,
    polygon_segment_count: usize,
}

impl StagedMapOverlays {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
            waypoint_count: 0,
            spawn_count: 0,
            waypoint_path_count: 0,
            waypoint_path_segment_count: 0,
            polygon_count: 0,
            polygon_segment_count: 0,
        }
    }

    /// Stages waypoint/start markers, named path ribbons, and polygon perimeter walls.
    ///
    /// Marker bases and polygon walls follow the exact already-staged terrain triangle when their
    /// XY coordinate is inside the terrain. An out-of-bounds polygon point falls back to its
    /// retained source Z so diagnostics remain inspectable without clamping it onto the map edge.
    ///
    /// # Errors
    ///
    /// Returns a structured geometry limit or scene-classification failure.
    pub fn from_map(
        world: Option<&MapWorldObjects>,
        polygons: Option<&MapPolygonData>,
        terrain: &StagedTerrain,
    ) -> Result<Self, MapOverlayError> {
        let mut staged = Self::empty();
        if let Some(world) = world {
            let scene = StagedMapScene::from_world_objects(world)
                .map_err(MapOverlayError::SceneClassification)?;
            let mut paths = BTreeMap::<Vec<u8>, Vec<(i32, u32)>>::new();
            let mut first_path_by_placement = BTreeMap::<u32, Vec<u8>>::new();
            for waypoint in scene.waypoints() {
                let object = usize::try_from(waypoint.placement_index())
                    .ok()
                    .and_then(|index| world.objects().get(index))
                    .ok_or(MapOverlayError::InvalidPlacementIndex)?;
                let mut labels = Vec::<Vec<u8>>::new();
                for slot in 0..3 {
                    let Some(label) = object.waypoint_path_label_bytes(slot) else {
                        continue;
                    };
                    let folded = ascii_fold(label);
                    if labels.contains(&folded) {
                        continue;
                    }
                    labels.push(folded.clone());
                    paths
                        .entry(folded)
                        .or_default()
                        .push((waypoint.waypoint_id(), waypoint.placement_index()));
                }
                if let Some(label) = labels.into_iter().next() {
                    first_path_by_placement.insert(waypoint.placement_index(), label);
                }
            }
            let path_colors = paths
                .keys()
                .enumerate()
                .map(|(index, label)| (label.clone(), waypoint_path_color(index)))
                .collect::<BTreeMap<_, _>>();
            for waypoint in scene.waypoints() {
                let placement = usize::try_from(waypoint.placement_index())
                    .ok()
                    .and_then(|index| scene.placements().get(index))
                    .ok_or(MapOverlayError::InvalidPlacementIndex)?;
                let position = placement.position();
                let ground = terrain
                    .height_at_world([position[0], position[1]])
                    .unwrap_or(position[2]);
                if let Some(player) = waypoint.player_start() {
                    staged.push_marker(
                        [position[0], position[1], ground + MARKER_GROUND_OFFSET],
                        SPAWN_RADIUS,
                        SPAWN_HEIGHT,
                        spawn_color(player),
                    )?;
                    staged.spawn_count += 1;
                } else {
                    let color = first_path_by_placement
                        .get(&waypoint.placement_index())
                        .and_then(|label| path_colors.get(label))
                        .copied()
                        .unwrap_or([0.18, 0.95, 0.95, 0.82]);
                    staged.push_marker(
                        [position[0], position[1], ground + MARKER_GROUND_OFFSET],
                        WAYPOINT_RADIUS,
                        WAYPOINT_HEIGHT,
                        color,
                    )?;
                }
                staged.waypoint_count += 1;
            }
            staged.waypoint_path_count = paths.len();
            for (label, mut members) in paths {
                members.sort_unstable();
                members.dedup_by_key(|member| member.1);
                let color = path_colors
                    .get(&label)
                    .copied()
                    .ok_or(MapOverlayError::MissingPathColor)?;
                for edge in members.windows(2) {
                    let first = usize::try_from(edge[0].1)
                        .ok()
                        .and_then(|index| scene.placements().get(index))
                        .ok_or(MapOverlayError::InvalidPlacementIndex)?;
                    let second = usize::try_from(edge[1].1)
                        .ok()
                        .and_then(|index| scene.placements().get(index))
                        .ok_or(MapOverlayError::InvalidPlacementIndex)?;
                    staged.push_waypoint_path_edge(
                        first.position(),
                        second.position(),
                        color,
                        terrain,
                    )?;
                }
            }
        }
        if let Some(polygons) = polygons {
            for area in polygons.areas() {
                staged.push_polygon(area, terrain)?;
                staged.polygon_count += 1;
            }
        }
        Ok(staged)
    }

    #[must_use]
    pub fn vertices(&self) -> &[MapOverlayVertex] {
        &self.vertices
    }

    #[must_use]
    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    #[must_use]
    pub const fn waypoint_count(&self) -> usize {
        self.waypoint_count
    }

    #[must_use]
    pub const fn spawn_count(&self) -> usize {
        self.spawn_count
    }

    #[must_use]
    pub const fn waypoint_path_count(&self) -> usize {
        self.waypoint_path_count
    }

    #[must_use]
    pub const fn waypoint_path_segment_count(&self) -> usize {
        self.waypoint_path_segment_count
    }

    #[must_use]
    pub const fn polygon_count(&self) -> usize {
        self.polygon_count
    }

    #[must_use]
    pub const fn polygon_segment_count(&self) -> usize {
        self.polygon_segment_count
    }

    pub(crate) fn vertex_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.vertices.len().saturating_mul(28));
        for vertex in &self.vertices {
            for value in vertex.position.into_iter().chain(vertex.color) {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        bytes
    }

    pub(crate) fn index_bytes(&self) -> Vec<u8> {
        self.indices
            .iter()
            .flat_map(|index| index.to_le_bytes())
            .collect()
    }

    fn push_marker(
        &mut self,
        base: [f32; 3],
        radius: f32,
        height: f32,
        color: [f32; 4],
    ) -> Result<(), MapOverlayError> {
        self.reserve_geometry(6, 24)?;
        let first =
            u32::try_from(self.vertices.len()).map_err(|_| MapOverlayError::GeometryTooLarge)?;
        let middle = base[2] + height * 0.55;
        self.vertices.extend([
            overlay_vertex(base, color),
            overlay_vertex([base[0] + radius, base[1], middle], color),
            overlay_vertex([base[0], base[1] + radius, middle], color),
            overlay_vertex([base[0] - radius, base[1], middle], color),
            overlay_vertex([base[0], base[1] - radius, middle], color),
            overlay_vertex([base[0], base[1], base[2] + height], color),
        ]);
        self.indices.extend([
            first,
            first + 1,
            first + 2,
            first,
            first + 2,
            first + 3,
            first,
            first + 3,
            first + 4,
            first,
            first + 4,
            first + 1,
            first + 5,
            first + 2,
            first + 1,
            first + 5,
            first + 3,
            first + 2,
            first + 5,
            first + 4,
            first + 3,
            first + 5,
            first + 1,
            first + 4,
        ]);
        Ok(())
    }

    #[allow(clippy::cast_precision_loss)]
    fn push_polygon(
        &mut self,
        area: &MapPolygonArea,
        terrain: &StagedTerrain,
    ) -> Result<(), MapOverlayError> {
        let points = area.points();
        if points.len() < 2 {
            return Ok(());
        }
        let color = polygon_color(area.source_index(), area.is_water());
        for index in 0..points.len() {
            let first = points[index].coordinates();
            let second = points[(index + 1) % points.len()].coordinates();
            if first[..2] == second[..2] {
                continue;
            }
            let first_xy = [first[0] as f32, first[1] as f32];
            let second_xy = [second[0] as f32, second[1] as f32];
            let first_z =
                terrain.height_at_world(first_xy).unwrap_or(first[2] as f32) + ZONE_GROUND_OFFSET;
            let second_z = terrain
                .height_at_world(second_xy)
                .unwrap_or(second[2] as f32)
                + ZONE_GROUND_OFFSET;
            self.push_zone_segment(first_xy, first_z, second_xy, second_z, color)?;
            self.polygon_segment_count += 1;
        }
        Ok(())
    }

    fn push_zone_segment(
        &mut self,
        first: [f32; 2],
        first_z: f32,
        second: [f32; 2],
        second_z: f32,
        color: [f32; 4],
    ) -> Result<(), MapOverlayError> {
        self.reserve_geometry(4, 6)?;
        let base =
            u32::try_from(self.vertices.len()).map_err(|_| MapOverlayError::GeometryTooLarge)?;
        self.vertices.extend([
            overlay_vertex([first[0], first[1], first_z], color),
            overlay_vertex([second[0], second[1], second_z], color),
            overlay_vertex([second[0], second[1], second_z + ZONE_HEIGHT], color),
            overlay_vertex([first[0], first[1], first_z + ZONE_HEIGHT], color),
        ]);
        self.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        Ok(())
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss
    )]
    fn push_waypoint_path_edge(
        &mut self,
        first: [f32; 3],
        second: [f32; 3],
        color: [f32; 4],
        terrain: &StagedTerrain,
    ) -> Result<(), MapOverlayError> {
        let delta = [second[0] - first[0], second[1] - first[1]];
        let distance = delta[0].hypot(delta[1]);
        if !distance.is_finite() {
            return Err(MapOverlayError::InvalidWaypointPosition);
        }
        if distance <= f32::EPSILON {
            return Ok(());
        }
        let subdivisions = (distance / PATH_TERRAIN_STEP).ceil().max(1.0) as usize;
        if subdivisions > MAX_PATH_SUBDIVISIONS_PER_EDGE {
            return Err(MapOverlayError::GeometryTooLarge);
        }
        let perpendicular = [
            -delta[1] / distance * PATH_HALF_WIDTH,
            delta[0] / distance * PATH_HALF_WIDTH,
        ];
        for step in 0..subdivisions {
            let start = step as f32 / subdivisions as f32;
            let end = (step + 1) as f32 / subdivisions as f32;
            let start_source = interpolate_position(first, second, start);
            let end_source = interpolate_position(first, second, end);
            let start_center = [
                start_source[0],
                start_source[1],
                terrain
                    .height_at_world([start_source[0], start_source[1]])
                    .unwrap_or(start_source[2])
                    + PATH_GROUND_OFFSET,
            ];
            let end_center = [
                end_source[0],
                end_source[1],
                terrain
                    .height_at_world([end_source[0], end_source[1]])
                    .unwrap_or(end_source[2])
                    + PATH_GROUND_OFFSET,
            ];
            self.push_path_ribbon(start_center, end_center, perpendicular, color)?;
            self.waypoint_path_segment_count += 1;
        }
        Ok(())
    }

    fn push_path_ribbon(
        &mut self,
        first: [f32; 3],
        second: [f32; 3],
        perpendicular: [f32; 2],
        color: [f32; 4],
    ) -> Result<(), MapOverlayError> {
        self.reserve_geometry(4, 6)?;
        let base =
            u32::try_from(self.vertices.len()).map_err(|_| MapOverlayError::GeometryTooLarge)?;
        self.vertices.extend([
            overlay_vertex(
                [
                    first[0] + perpendicular[0],
                    first[1] + perpendicular[1],
                    first[2],
                ],
                color,
            ),
            overlay_vertex(
                [
                    first[0] - perpendicular[0],
                    first[1] - perpendicular[1],
                    first[2],
                ],
                color,
            ),
            overlay_vertex(
                [
                    second[0] - perpendicular[0],
                    second[1] - perpendicular[1],
                    second[2],
                ],
                color,
            ),
            overlay_vertex(
                [
                    second[0] + perpendicular[0],
                    second[1] + perpendicular[1],
                    second[2],
                ],
                color,
            ),
        ]);
        self.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        Ok(())
    }

    fn reserve_geometry(
        &mut self,
        vertex_count: usize,
        index_count: usize,
    ) -> Result<(), MapOverlayError> {
        let vertices = self
            .vertices
            .len()
            .checked_add(vertex_count)
            .ok_or(MapOverlayError::GeometryTooLarge)?;
        let indices = self
            .indices
            .len()
            .checked_add(index_count)
            .ok_or(MapOverlayError::GeometryTooLarge)?;
        if vertices > MAX_OVERLAY_VERTICES || indices > MAX_OVERLAY_INDICES {
            return Err(MapOverlayError::GeometryTooLarge);
        }
        self.vertices.reserve(vertex_count);
        self.indices.reserve(index_count);
        Ok(())
    }
}

fn overlay_vertex(position: [f32; 3], color: [f32; 4]) -> MapOverlayVertex {
    MapOverlayVertex { position, color }
}

fn ascii_fold(value: &[u8]) -> Vec<u8> {
    value.iter().map(u8::to_ascii_lowercase).collect()
}

fn interpolate_position(first: [f32; 3], second: [f32; 3], factor: f32) -> [f32; 3] {
    [
        first[0] + (second[0] - first[0]) * factor,
        first[1] + (second[1] - first[1]) * factor,
        first[2] + (second[2] - first[2]) * factor,
    ]
}

#[allow(clippy::cast_precision_loss)]
fn waypoint_path_color(index: usize) -> [f32; 4] {
    let hue = (index as f32 * 0.618_034).fract();
    let scaled = hue * 6.0;
    let fraction = scaled.fract();
    let low = 0.15;
    let rising = low + (1.0 - low) * fraction;
    let falling = 1.0 - (1.0 - low) * fraction;
    let rgb = if scaled < 1.0 {
        [1.0, rising, low]
    } else if scaled < 2.0 {
        [falling, 1.0, low]
    } else if scaled < 3.0 {
        [low, 1.0, rising]
    } else if scaled < 4.0 {
        [low, falling, 1.0]
    } else if scaled < 5.0 {
        [rising, low, 1.0]
    } else {
        [1.0, low, falling]
    };
    [rgb[0], rgb[1], rgb[2], 0.88]
}

fn spawn_color(player: u32) -> [f32; 4] {
    let index = usize::try_from(player.saturating_sub(1)).unwrap_or(0) % SPAWN_COLORS.len();
    let color = SPAWN_COLORS[index];
    [color[0], color[1], color[2], 0.94]
}

fn polygon_color(source_index: u32, water: bool) -> [f32; 4] {
    if water {
        return [0.12, 0.55, 1.0, 0.24];
    }
    let index = usize::try_from(source_index).unwrap_or(0) % POLYGON_COLORS.len();
    let color = POLYGON_COLORS[index];
    [color[0], color[1], color[2], 0.28]
}

/// A structured diagnostic-overlay staging failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapOverlayError {
    SceneClassification(crate::MapSceneStagingError),
    InvalidPlacementIndex,
    InvalidWaypointPosition,
    MissingPathColor,
    GeometryTooLarge,
}

impl Display for MapOverlayError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::SceneClassification(error) => Display::fmt(error, formatter),
            Self::InvalidPlacementIndex => {
                formatter.write_str("MAP waypoint references an invalid placement index")
            }
            Self::InvalidWaypointPosition => {
                formatter.write_str("MAP waypoint path contains a non-finite position")
            }
            Self::MissingPathColor => {
                formatter.write_str("MAP waypoint path has no deterministic diagnostic color")
            }
            Self::GeometryTooLarge => {
                formatter.write_str("MAP diagnostic overlays exceed geometry limits")
            }
        }
    }
}

impl Error for MapOverlayError {}

#[cfg(test)]
mod tests {
    use super::{StagedMapOverlays, polygon_color, spawn_color, waypoint_path_color};

    #[test]
    fn marker_geometry_and_palette_are_stable() {
        let mut overlays = StagedMapOverlays::empty();
        overlays
            .push_marker([10.0, 20.0, 30.0], 8.0, 44.0, spawn_color(1))
            .expect("marker");
        assert_eq!(overlays.vertices().len(), 6);
        assert_eq!(overlays.indices().len(), 24);
        assert_eq!(
            overlays.vertices()[0].position().map(f32::to_bits),
            [10.0, 20.0, 30.0].map(f32::to_bits)
        );
        assert_eq!(
            overlays.vertices()[5].position().map(f32::to_bits),
            [10.0, 20.0, 74.0].map(f32::to_bits)
        );
        assert_ne!(
            spawn_color(1).map(f32::to_bits),
            spawn_color(2).map(f32::to_bits)
        );
        assert_ne!(
            polygon_color(0, false).map(f32::to_bits),
            polygon_color(1, false).map(f32::to_bits)
        );
        assert_eq!(
            polygon_color(0, true).map(f32::to_bits),
            polygon_color(3, true).map(f32::to_bits)
        );
        assert_ne!(
            waypoint_path_color(0).map(f32::to_bits),
            waypoint_path_color(1).map(f32::to_bits)
        );
    }

    #[test]
    fn zone_segments_are_bounded_quads() {
        let mut overlays = StagedMapOverlays::empty();
        overlays
            .push_zone_segment([0.0, 0.0], 2.0, [10.0, 0.0], 4.0, polygon_color(0, false))
            .expect("zone");
        assert_eq!(overlays.vertices().len(), 4);
        assert_eq!(overlays.indices(), [0, 1, 2, 0, 2, 3]);
        assert_eq!(
            overlays.vertices()[2].position().map(f32::to_bits),
            [10.0, 0.0, 18.0].map(f32::to_bits)
        );

        overlays
            .push_path_ribbon(
                [0.0, 0.0, 3.0],
                [10.0, 0.0, 5.0],
                [0.0, 2.25],
                waypoint_path_color(2),
            )
            .expect("path ribbon");
        assert_eq!(overlays.vertices().len(), 8);
        assert_eq!(overlays.indices().len(), 12);
        assert_eq!(
            overlays.vertices()[4].position().map(f32::to_bits),
            [0.0, 2.25, 3.0].map(f32::to_bits)
        );
        assert_eq!(
            overlays.vertices()[7].position().map(f32::to_bits),
            [10.0, 2.25, 5.0].map(f32::to_bits)
        );
    }
}
