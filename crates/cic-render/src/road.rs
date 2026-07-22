// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Deterministic terrain-fitted staging for regular MAP road segments.
//!
//! Consecutive endpoint pairing, physical width, regular-segment texture coordinates, terrain
//! sampling interval, and the small height offset are derived from `W3DRoadBuffer.cpp` in
//! `GeneralsGameCode` revision `9f7abb866f5afd446db14149979e744c7216baaf`, licensed under
//! GPL-3.0-or-later with Electronic Arts Section 7 terms. This bounded implementation stages the
//! established regular strip. Corner and junction edge polygons are deterministic project-authored
//! presentation driven by retained source flags; they are not claimed as a line-for-line port of
//! the source fixed-function tee/curve inserter.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use cic_formats::{MapHeightField, MapWorldObjects, RoadDefinition, object_flags};

use crate::{TERRAIN_HEIGHT_SCALE, TERRAIN_XY_SCALE, TextureResourceManager};

const MAX_ROAD_VERTICES: usize = 2_000_000;
const MAX_ROAD_INDICES: usize = 6_000_000;
const MAX_COLUMNS_PER_SEGMENT: usize = 100_000;
const MAX_ACROSS_SAMPLES: usize = 100;
const SOURCE_REGULAR_V_CENTER: f32 = 85.0 / 512.0;
const FLOAT_ABOVE_TERRAIN: f32 = TERRAIN_HEIGHT_SCALE / 8.0;

/// One road vertex compatible with the terrain-viewer vertex layout.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoadVertex {
    position: [f32; 3],
    uv: [f32; 2],
}

impl RoadVertex {
    #[must_use]
    pub const fn position(self) -> [f32; 3] {
        self.position
    }

    #[must_use]
    pub const fn uv(self) -> [f32; 2] {
        self.uv
    }
}

/// One decoded straight-alpha road texture owned by immutable staging output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoadTexture {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl RoadTexture {
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    #[must_use]
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }
}

/// One source road type retained in first-use order.
#[derive(Debug, Clone, PartialEq)]
pub struct StagedRoadMaterial {
    name: Vec<u8>,
    road_width: f32,
    road_width_in_texture: f32,
    texture: RoadTexture,
}

impl StagedRoadMaterial {
    #[must_use]
    pub fn name_bytes(&self) -> &[u8] {
        &self.name
    }

    #[must_use]
    pub const fn road_width(&self) -> f32 {
        self.road_width
    }

    #[must_use]
    pub const fn road_width_in_texture(&self) -> f32 {
        self.road_width_in_texture
    }

    #[must_use]
    pub const fn texture(&self) -> &RoadTexture {
        &self.texture
    }
}

/// One source-paired draw range in authoritative object order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StagedRoadDraw {
    kind: RoadDrawKind,
    material_index: u32,
    first_index: u32,
    index_count: u32,
    first_placement_id: u32,
    second_placement_id: u32,
    retains_corner_or_join_flags: bool,
}

/// Geometry role for one stable road draw range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoadDrawKind {
    Segment,
    Corner,
    Junction,
}

impl StagedRoadDraw {
    #[must_use]
    pub const fn kind(self) -> RoadDrawKind {
        self.kind
    }

    #[must_use]
    pub const fn material_index(self) -> u32 {
        self.material_index
    }

    #[must_use]
    pub const fn first_index(self) -> u32 {
        self.first_index
    }

    #[must_use]
    pub const fn index_count(self) -> u32 {
        self.index_count
    }

    #[must_use]
    pub const fn placement_ids(self) -> [u32; 2] {
        [self.first_placement_id, self.second_placement_id]
    }

    /// True when source corner/join policy was retained but this regular-strip gate did not yet
    /// insert its curve, tee, or alpha-join geometry.
    #[must_use]
    pub const fn retains_corner_or_join_flags(self) -> bool {
        self.retains_corner_or_join_flags
    }
}

/// Stable reason that an endpoint record did not produce exact road geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoadDiagnosticKind {
    MissingSecondEndpoint,
    UnexpectedSecondEndpoint,
    MissingDefinition,
    InvalidDefinition,
    MissingTexture,
    ZeroLength,
}

#[derive(Debug, Clone, Copy)]
struct RoadEndpoint {
    source_order: u32,
    placement_id: u32,
    position: [f32; 2],
    direction_out: [f32; 2],
    half_width: f32,
    road_width: f32,
    material_index: u32,
}

/// One source-order road staging diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoadDiagnostic {
    placement_id: u32,
    kind: RoadDiagnosticKind,
    name: Vec<u8>,
}

impl RoadDiagnostic {
    #[must_use]
    pub const fn placement_id(&self) -> u32 {
        self.placement_id
    }

    #[must_use]
    pub const fn kind(&self) -> RoadDiagnosticKind {
        self.kind
    }

    #[must_use]
    pub fn name_bytes(&self) -> &[u8] {
        &self.name
    }
}

/// Immutable road materials, regular geometry, draw order, and unresolved diagnostics.
#[derive(Debug, Clone, PartialEq)]
pub struct StagedRoads {
    vertices: Vec<RoadVertex>,
    indices: Vec<u32>,
    materials: Vec<StagedRoadMaterial>,
    draws: Vec<StagedRoadDraw>,
    diagnostics: Vec<RoadDiagnostic>,
}

impl StagedRoads {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
            materials: Vec::new(),
            draws: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    /// Resolves road definitions by ASCII-insensitive name and stages consecutive Point1/Point2
    /// pairs as terrain-fitted regular strips.
    ///
    /// Duplicate definitions use the last declaration supplied by the caller. Invalid or missing
    /// definitions and textures become stable diagnostics rather than guessed geometry.
    ///
    /// # Errors
    ///
    /// Returns [`RoadStagingError::GeometryTooLarge`] when tessellation or checked index conversion
    /// exceeds the explicit road staging bounds.
    #[allow(clippy::too_many_lines)]
    pub fn from_map(
        world: &MapWorldObjects,
        height: &MapHeightField,
        definitions: &[RoadDefinition],
        textures: &TextureResourceManager,
    ) -> Result<Self, RoadStagingError> {
        let definitions = definitions
            .iter()
            .map(|definition| (ascii_fold(definition.name_bytes()), definition))
            .collect::<BTreeMap<_, _>>();
        let mut staged = Self::empty();
        let mut material_indices = BTreeMap::new();
        let mut endpoints = Vec::new();
        let objects = world.objects();
        let mut object_index = 0;
        while object_index < objects.len() {
            let first = &objects[object_index];
            if first.flags() & object_flags::ROAD_POINT2 != 0
                && first.flags() & object_flags::ROAD_POINT1 == 0
            {
                staged.push_diagnostic(first, RoadDiagnosticKind::UnexpectedSecondEndpoint);
                object_index += 1;
                continue;
            }
            if first.flags() & object_flags::ROAD_POINT1 == 0 {
                object_index += 1;
                continue;
            }
            let Some(second) = objects.get(object_index + 1) else {
                staged.push_diagnostic(first, RoadDiagnosticKind::MissingSecondEndpoint);
                break;
            };
            if second.flags() & object_flags::ROAD_POINT2 == 0 {
                staged.push_diagnostic(first, RoadDiagnosticKind::MissingSecondEndpoint);
                object_index += 1;
                continue;
            }
            let key = ascii_fold(first.name_bytes());
            let Some(definition) = definitions.get(&key).copied() else {
                staged.push_diagnostic(first, RoadDiagnosticKind::MissingDefinition);
                object_index += 2;
                continue;
            };
            if definition.road_width() <= 0.0 || definition.road_width_in_texture() <= 0.0 {
                staged.push_diagnostic(first, RoadDiagnosticKind::InvalidDefinition);
                object_index += 2;
                continue;
            }
            let Some(texture_name) = definition.texture_bytes() else {
                staged.push_diagnostic(first, RoadDiagnosticKind::MissingTexture);
                object_index += 2;
                continue;
            };
            let Some(image) = textures.image(texture_name) else {
                staged.push_diagnostic(first, RoadDiagnosticKind::MissingTexture);
                object_index += 2;
                continue;
            };
            let first_xy = [first.position()[0], first.position()[1]];
            let second_xy = [second.position()[0], second.position()[1]];
            let delta = [second_xy[0] - first_xy[0], second_xy[1] - first_xy[1]];
            let length = delta[0].hypot(delta[1]);
            if !length.is_finite() || length <= f32::EPSILON {
                staged.push_diagnostic(first, RoadDiagnosticKind::ZeroLength);
                object_index += 2;
                continue;
            }
            let material_index = if let Some(index) = material_indices.get(&key) {
                *index
            } else {
                let index = u32::try_from(staged.materials.len())
                    .map_err(|_| RoadStagingError::GeometryTooLarge)?;
                staged.materials.push(StagedRoadMaterial {
                    name: definition.name_bytes().to_vec(),
                    road_width: definition.road_width(),
                    road_width_in_texture: definition.road_width_in_texture(),
                    texture: RoadTexture {
                        width: image.width(),
                        height: image.height(),
                        rgba: image.rgba().to_vec(),
                    },
                });
                material_indices.insert(key, index);
                index
            };
            let retains_corner_or_join_flags = (first.flags() | second.flags())
                & (object_flags::ROAD_CORNER_ANGLED
                    | object_flags::ROAD_CORNER_TIGHT
                    | object_flags::ROAD_JOIN)
                != 0;
            staged.push_regular_segment(
                height,
                first_xy,
                second_xy,
                definition.road_width(),
                definition.road_width_in_texture(),
                material_index,
                first.placement_id(),
                second.placement_id(),
                retains_corner_or_join_flags,
            )?;
            let direction = [delta[0] / length, delta[1] / length];
            let half_width = definition.road_width() * definition.road_width_in_texture() * 0.5;
            endpoints.push(RoadEndpoint {
                source_order: u32::try_from(object_index)
                    .map_err(|_| RoadStagingError::GeometryTooLarge)?,
                placement_id: first.placement_id(),
                position: first_xy,
                direction_out: direction,
                half_width,
                road_width: definition.road_width(),
                material_index,
            });
            endpoints.push(RoadEndpoint {
                source_order: u32::try_from(object_index + 1)
                    .map_err(|_| RoadStagingError::GeometryTooLarge)?,
                placement_id: second.placement_id(),
                position: second_xy,
                direction_out: [-direction[0], -direction[1]],
                half_width,
                road_width: definition.road_width(),
                material_index,
            });
            object_index += 2;
        }
        staged.push_endpoint_joins(height, &endpoints)?;
        Ok(staged)
    }

    #[must_use]
    pub fn vertices(&self) -> &[RoadVertex] {
        &self.vertices
    }

    #[must_use]
    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    #[must_use]
    pub fn materials(&self) -> &[StagedRoadMaterial] {
        &self.materials
    }

    #[must_use]
    pub fn draws(&self) -> &[StagedRoadDraw] {
        &self.draws
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[RoadDiagnostic] {
        &self.diagnostics
    }

    pub(crate) fn vertex_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.vertices.len() * 32);
        for vertex in &self.vertices {
            for value in vertex.position {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            for value in vertex.uv {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            for value in [0.0_f32, 0.0, 1.0] {
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

    fn push_diagnostic(
        &mut self,
        object: &cic_formats::MapObjectPlacement,
        kind: RoadDiagnosticKind,
    ) {
        self.diagnostics.push(RoadDiagnostic {
            placement_id: object.placement_id(),
            kind,
            name: object.name_bytes().to_vec(),
        });
    }

    fn push_endpoint_joins(
        &mut self,
        height: &MapHeightField,
        endpoints: &[RoadEndpoint],
    ) -> Result<(), RoadStagingError> {
        let mut grouped = BTreeMap::<(u32, u32), Vec<RoadEndpoint>>::new();
        for endpoint in endpoints {
            grouped
                .entry((
                    endpoint.position[0].to_bits(),
                    endpoint.position[1].to_bits(),
                ))
                .or_default()
                .push(*endpoint);
        }
        let mut groups = grouped.into_values().collect::<Vec<_>>();
        groups.sort_by_key(|group| {
            group
                .iter()
                .map(|endpoint| endpoint.source_order)
                .min()
                .unwrap_or(u32::MAX)
        });
        for mut group in groups {
            if group.len() < 2 {
                continue;
            }
            group.sort_by_key(|endpoint| endpoint.source_order);
            if group.len() == 2 && group[0].material_index == group[1].material_index {
                let dot = group[0].direction_out[0] * group[1].direction_out[0]
                    + group[0].direction_out[1] * group[1].direction_out[1];
                if dot < -0.999 {
                    continue;
                }
                let points = join_outline(&group);
                self.push_join_fan(height, &group, RoadDrawKind::Corner, &points)?;
                continue;
            }
            let points = join_outline(&group);
            let mut by_material = BTreeMap::<u32, Vec<RoadEndpoint>>::new();
            for endpoint in group {
                by_material
                    .entry(endpoint.material_index)
                    .or_default()
                    .push(endpoint);
            }
            let mut material_groups = by_material.into_values().collect::<Vec<_>>();
            material_groups.sort_by_key(|material_group| {
                material_group
                    .iter()
                    .map(|endpoint| endpoint.source_order)
                    .min()
                    .unwrap_or(u32::MAX)
            });
            for material_group in material_groups {
                self.push_join_fan(height, &material_group, RoadDrawKind::Junction, &points)?;
            }
        }
        Ok(())
    }

    fn push_join_fan(
        &mut self,
        height: &MapHeightField,
        endpoints: &[RoadEndpoint],
        kind: RoadDrawKind,
        points: &[[f32; 2]],
    ) -> Result<(), RoadStagingError> {
        if points.len() < 3 {
            return Ok(());
        }
        let added_vertices = points.len() + 1;
        let added_indices = points
            .len()
            .checked_mul(3)
            .ok_or(RoadStagingError::GeometryTooLarge)?;
        if self.vertices.len().saturating_add(added_vertices) > MAX_ROAD_VERTICES
            || self.indices.len().saturating_add(added_indices) > MAX_ROAD_INDICES
        {
            return Err(RoadStagingError::GeometryTooLarge);
        }
        let reference = endpoints[0];
        let center = reference.position;
        let base_vertex =
            u32::try_from(self.vertices.len()).map_err(|_| RoadStagingError::GeometryTooLarge)?;
        let first_index =
            u32::try_from(self.indices.len()).map_err(|_| RoadStagingError::GeometryTooLarge)?;
        self.vertices
            .push(join_vertex(height, center, center, reference)?);
        for point in points {
            self.vertices
                .push(join_vertex(height, center, *point, reference)?);
        }
        for index in 0..points.len() {
            let current = base_vertex
                .checked_add(
                    u32::try_from(index + 1).map_err(|_| RoadStagingError::GeometryTooLarge)?,
                )
                .ok_or(RoadStagingError::GeometryTooLarge)?;
            let next_offset = (index + 1) % points.len() + 1;
            let next = base_vertex
                .checked_add(
                    u32::try_from(next_offset).map_err(|_| RoadStagingError::GeometryTooLarge)?,
                )
                .ok_or(RoadStagingError::GeometryTooLarge)?;
            self.indices
                .extend_from_slice(&[base_vertex, current, next]);
        }
        self.draws.push(StagedRoadDraw {
            kind,
            material_index: reference.material_index,
            first_index,
            index_count: u32::try_from(added_indices)
                .map_err(|_| RoadStagingError::GeometryTooLarge)?,
            first_placement_id: endpoints[0].placement_id,
            second_placement_id: endpoints[endpoints.len() - 1].placement_id,
            retains_corner_or_join_flags: false,
        });
        Ok(())
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss,
        clippy::too_many_arguments
    )]
    fn push_regular_segment(
        &mut self,
        height: &MapHeightField,
        first: [f32; 2],
        second: [f32; 2],
        road_width: f32,
        width_in_texture: f32,
        material_index: u32,
        first_placement_id: u32,
        second_placement_id: u32,
        retains_corner_or_join_flags: bool,
    ) -> Result<(), RoadStagingError> {
        let delta = [second[0] - first[0], second[1] - first[1]];
        let length = delta[0].hypot(delta[1]);
        let direction = [delta[0] / length, delta[1] / length];
        let normal = [-direction[1], direction[0]];
        let half_width = road_width * width_in_texture * 0.5;
        let column_count = usize::try_from((length / TERRAIN_XY_SCALE).ceil() as u64)
            .map_err(|_| RoadStagingError::GeometryTooLarge)?
            .saturating_add(1)
            .max(2);
        if column_count > MAX_COLUMNS_PER_SEGMENT {
            return Err(RoadStagingError::GeometryTooLarge);
        }
        let added_vertices = column_count
            .checked_mul(2)
            .ok_or(RoadStagingError::GeometryTooLarge)?;
        let added_indices = column_count
            .saturating_sub(1)
            .checked_mul(6)
            .ok_or(RoadStagingError::GeometryTooLarge)?;
        if self.vertices.len().saturating_add(added_vertices) > MAX_ROAD_VERTICES
            || self.indices.len().saturating_add(added_indices) > MAX_ROAD_INDICES
        {
            return Err(RoadStagingError::GeometryTooLarge);
        }
        let base_vertex =
            u32::try_from(self.vertices.len()).map_err(|_| RoadStagingError::GeometryTooLarge)?;
        let first_index =
            u32::try_from(self.indices.len()).map_err(|_| RoadStagingError::GeometryTooLarge)?;
        let across_count = usize::try_from((half_width * 2.0 / TERRAIN_XY_SCALE).ceil() as u64)
            .map_err(|_| RoadStagingError::GeometryTooLarge)?
            .saturating_add(1)
            .clamp(2, MAX_ACROSS_SAMPLES);
        for column in 0..column_count {
            #[allow(clippy::cast_precision_loss)]
            let distance = length * column as f32 / (column_count - 1) as f32;
            let center = [
                first[0] + direction[0] * distance,
                first[1] + direction[1] * distance,
            ];
            let mut maximum_height = f32::NEG_INFINITY;
            for sample in 0..across_count {
                #[allow(clippy::cast_precision_loss)]
                let fraction = sample as f32 / (across_count - 1) as f32;
                let offset = -half_width + fraction * half_width * 2.0;
                maximum_height = maximum_height.max(max_cell_height(
                    height,
                    [
                        center[0] + normal[0] * offset,
                        center[1] + normal[1] * offset,
                    ],
                )?);
            }
            let z = maximum_height + FLOAT_ABOVE_TERRAIN;
            let u = distance / (road_width * 4.0);
            let v_half = width_in_texture / 8.0;
            self.vertices.push(RoadVertex {
                position: [
                    center[0] + normal[0] * half_width,
                    center[1] + normal[1] * half_width,
                    z,
                ],
                uv: [u, SOURCE_REGULAR_V_CENTER - v_half],
            });
            self.vertices.push(RoadVertex {
                position: [
                    center[0] - normal[0] * half_width,
                    center[1] - normal[1] * half_width,
                    z,
                ],
                uv: [u, SOURCE_REGULAR_V_CENTER + v_half],
            });
        }
        for column in 0..column_count - 1 {
            let offset =
                u32::try_from(column * 2).map_err(|_| RoadStagingError::GeometryTooLarge)?;
            let left = base_vertex + offset;
            let right = left + 1;
            let next_left = left + 2;
            let next_right = left + 3;
            self.indices
                .extend_from_slice(&[left, right, next_right, left, next_right, next_left]);
        }
        self.draws.push(StagedRoadDraw {
            kind: RoadDrawKind::Segment,
            material_index,
            first_index,
            index_count: u32::try_from(added_indices)
                .map_err(|_| RoadStagingError::GeometryTooLarge)?,
            first_placement_id,
            second_placement_id,
            retains_corner_or_join_flags,
        });
        Ok(())
    }
}

fn join_outline(endpoints: &[RoadEndpoint]) -> Vec<[f32; 2]> {
    let center = endpoints[0].position;
    let mut points = Vec::with_capacity(endpoints.len().saturating_mul(2));
    for endpoint in endpoints {
        let normal = [-endpoint.direction_out[1], endpoint.direction_out[0]];
        points.push([
            center[0] + normal[0] * endpoint.half_width,
            center[1] + normal[1] * endpoint.half_width,
        ]);
        points.push([
            center[0] - normal[0] * endpoint.half_width,
            center[1] - normal[1] * endpoint.half_width,
        ]);
    }
    points.sort_by(|left, right| {
        let left_angle = (left[1] - center[1]).atan2(left[0] - center[0]);
        let right_angle = (right[1] - center[1]).atan2(right[0] - center[0]);
        left_angle.total_cmp(&right_angle)
    });
    points.dedup_by(|left, right| {
        left[0].to_bits() == right[0].to_bits() && left[1].to_bits() == right[1].to_bits()
    });
    points
}

fn join_vertex(
    height: &MapHeightField,
    center: [f32; 2],
    point: [f32; 2],
    reference: RoadEndpoint,
) -> Result<RoadVertex, RoadStagingError> {
    let relative = [point[0] - center[0], point[1] - center[1]];
    let normal = [-reference.direction_out[1], reference.direction_out[0]];
    let texture_scale = reference.road_width * 4.0;
    let u = (relative[0] * reference.direction_out[0] + relative[1] * reference.direction_out[1])
        / texture_scale;
    let v = SOURCE_REGULAR_V_CENTER
        - (relative[0] * normal[0] + relative[1] * normal[1]) / texture_scale;
    Ok(RoadVertex {
        position: [
            point[0],
            point[1],
            max_cell_height(height, point)? + FLOAT_ABOVE_TERRAIN * 2.0,
        ],
        uv: [u, v],
    })
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn max_cell_height(height: &MapHeightField, world: [f32; 2]) -> Result<f32, RoadStagingError> {
    let width =
        usize::try_from(height.width()).map_err(|_| RoadStagingError::InvalidHeightField)?;
    let rows =
        usize::try_from(height.height()).map_err(|_| RoadStagingError::InvalidHeightField)?;
    if width < 2 || rows < 2 || height.samples().len() != width.saturating_mul(rows) {
        return Err(RoadStagingError::InvalidHeightField);
    }
    #[allow(clippy::cast_precision_loss)]
    let border = height.border_size() as f32;
    let grid_x = (world[0] / TERRAIN_XY_SCALE + border)
        .floor()
        .clamp(0.0, (width - 2) as f32) as usize;
    let grid_y = (world[1] / TERRAIN_XY_SCALE + border)
        .floor()
        .clamp(0.0, (rows - 2) as f32) as usize;
    let p0 = grid_y
        .checked_mul(width)
        .and_then(|row| row.checked_add(grid_x))
        .ok_or(RoadStagingError::InvalidHeightField)?;
    let indices = [p0, p0 + 1, p0 + width, p0 + width + 1];
    let sample = indices
        .into_iter()
        .filter_map(|index| height.samples().get(index).copied())
        .max()
        .ok_or(RoadStagingError::InvalidHeightField)?;
    Ok(f32::from(sample) * TERRAIN_HEIGHT_SCALE)
}

fn ascii_fold(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().map(u8::to_ascii_lowercase).collect()
}

/// Road staging failure that prevents a complete immutable result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoadStagingError {
    GeometryTooLarge,
    InvalidHeightField,
}

impl Display for RoadStagingError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::GeometryTooLarge => formatter.write_str("road geometry exceeds staging limits"),
            Self::InvalidHeightField => {
                formatter.write_str("road staging received an inconsistent height field")
            }
        }
    }
}

impl Error for RoadStagingError {}

#[cfg(test)]
mod tests {
    use cic_formats::{
        MapLimits, MapScenarioLimits, RoadIniLimits, decode_map_height, decode_map_world_objects,
        parse_map, parse_road_ini,
    };

    use crate::TextureResourceManager;

    use super::{RoadDiagnosticKind, RoadDrawKind, StagedRoads};

    #[test]
    fn pairs_regular_roads_and_fits_columns_above_terrain() {
        let map = parse_map(&fixture(), "roads.map", MapLimits::default()).expect("map");
        let height = decode_map_height(&map, MapLimits::default()).expect("height");
        let world = decode_map_world_objects(&map, MapScenarioLimits::default()).expect("objects");
        let ini = parse_road_ini(
            b"Road SyntheticRoad\n Texture = road.tga\n RoadWidth = 20\n RoadWidthInTexture = 1\nEnd\n",
            RoadIniLimits::default(),
        )
        .expect("road INI");
        let mut textures = TextureResourceManager::default();
        textures
            .insert(b"road.tga", 1, 1, vec![100, 90, 80, 255])
            .expect("texture");
        let roads =
            StagedRoads::from_map(&world, &height, ini.definitions(), &textures).expect("roads");
        assert_eq!(roads.draws().len(), 3);
        assert_eq!(roads.materials().len(), 1);
        assert_eq!(roads.vertices().len(), 17);
        assert_eq!(roads.indices().len(), 36);
        assert!(
            roads
                .vertices()
                .iter()
                .all(|vertex| vertex.position()[2] > 2.5)
        );
        assert!(roads.diagnostics().is_empty());
        assert_eq!(roads.draws()[2].kind(), RoadDrawKind::Corner);
        assert_eq!(roads.draws()[0].placement_ids(), [0, 1]);
    }

    #[test]
    fn reports_unpaired_and_unresolved_records_without_guessing() {
        let map = parse_map(&fixture(), "roads.map", MapLimits::default()).expect("map");
        let height = decode_map_height(&map, MapLimits::default()).expect("height");
        let world = decode_map_world_objects(&map, MapScenarioLimits::default()).expect("objects");
        let roads = StagedRoads::from_map(&world, &height, &[], &TextureResourceManager::default())
            .expect("roads");
        assert!(roads.draws().is_empty());
        assert_eq!(
            roads.diagnostics()[0].kind(),
            RoadDiagnosticKind::MissingDefinition
        );
    }

    fn fixture() -> Vec<u8> {
        let symbols = [
            (1_u32, b"HeightMapData".as_slice()),
            (2, b"WorldInfo".as_slice()),
            (3, b"ObjectsList".as_slice()),
            (4, b"Object".as_slice()),
        ];
        let mut bytes = b"CkMp".to_vec();
        bytes.extend_from_slice(
            &i32::try_from(symbols.len())
                .expect("symbol count fits")
                .to_le_bytes(),
        );
        for (id, name) in symbols {
            bytes.push(u8::try_from(name.len()).expect("symbol name fits"));
            bytes.extend_from_slice(name);
            bytes.extend_from_slice(&id.to_le_bytes());
        }
        let mut height = Vec::new();
        height.extend_from_slice(&3_i32.to_le_bytes());
        height.extend_from_slice(&3_i32.to_le_bytes());
        height.extend_from_slice(&0_i32.to_le_bytes());
        height.extend_from_slice(&9_i32.to_le_bytes());
        height.extend_from_slice(&[4_u8; 9]);
        push_chunk(&mut bytes, 1, 3, &height);
        push_chunk(&mut bytes, 2, 1, &0_u16.to_le_bytes());
        let mut objects = Vec::new();
        objects.extend_from_slice(&object([0.0, 0.0, 0.0], 0x2, b"SyntheticRoad"));
        objects.extend_from_slice(&object([20.0, 0.0, 0.0], 0x4 | 0x40, b"SyntheticRoad"));
        objects.extend_from_slice(&object([20.0, 0.0, 0.0], 0x2 | 0x40, b"SyntheticRoad"));
        objects.extend_from_slice(&object([20.0, 20.0, 0.0], 0x4, b"SyntheticRoad"));
        push_chunk(&mut bytes, 3, 3, &objects);
        bytes
    }

    fn object(position: [f32; 3], flags: u32, name: &[u8]) -> Vec<u8> {
        let mut payload = Vec::new();
        for value in [position[0], position[1], position[2], 0.0] {
            payload.extend_from_slice(&value.to_le_bytes());
        }
        payload.extend_from_slice(&flags.to_le_bytes());
        payload.extend_from_slice(
            &u16::try_from(name.len())
                .expect("object name fits")
                .to_le_bytes(),
        );
        payload.extend_from_slice(name);
        payload.extend_from_slice(&0_u16.to_le_bytes());
        let mut chunk = 4_u32.to_le_bytes().to_vec();
        chunk.extend_from_slice(&3_u16.to_le_bytes());
        chunk.extend_from_slice(
            &i32::try_from(payload.len())
                .expect("object payload fits")
                .to_le_bytes(),
        );
        chunk.extend_from_slice(&payload);
        chunk
    }

    fn push_chunk(bytes: &mut Vec<u8>, id: u32, version: u16, payload: &[u8]) {
        bytes.extend_from_slice(&id.to_le_bytes());
        bytes.extend_from_slice(&version.to_le_bytes());
        bytes.extend_from_slice(
            &i32::try_from(payload.len())
                .expect("chunk payload fits")
                .to_le_bytes(),
        );
        bytes.extend_from_slice(payload);
    }
}
