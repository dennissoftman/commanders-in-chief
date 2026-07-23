// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Deterministic terrain-fitted staging for MAP road networks.
//!
//! Consecutive endpoint pairing, physical width, regular-segment texture coordinates, terrain
//! sampling interval, and the small height offset are derived from `W3DRoadBuffer.cpp` in
//! `GeneralsGameCode` revision `9f7abb866f5afd446db14149979e744c7216baaf`, licensed under
//! GPL-3.0-or-later with Electronic Arts Section 7 terms. The topology pass retains immutable
//! staging values, explicit limits, and stable ordering while reproducing the source endpoint
//! trimming, 30-degree curve subdivision, miter policy, and atlas-specific junction quads.

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
const CORNER_RADIUS: f32 = 1.5;
const TIGHT_CORNER_RADIUS: f32 = 0.5;
const TEE_WIDTH_ADJUSTMENT: f32 = 1.03;
const CURVE_STEP_RADIANS: f32 = std::f32::consts::PI / 6.0;
const MAX_CURVE_STEPS: usize = 12;
const MITER_LIMIT: f32 = 4.0;

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
    Curve,
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
struct PendingRoadPoint {
    loc: [f32; 2],
    top: [f32; 2],
    bottom: [f32; 2],
    is_angled: bool,
    is_join: bool,
}

#[derive(Debug, Clone, Copy)]
struct PendingRoadSegment {
    points: [PendingRoadPoint; 2],
    scale: f32,
    width_in_texture: f32,
    curve_radius: f32,
    material_index: u32,
    source_order: u32,
    placement_ids: [u32; 2],
    retains_corner_or_join_flags: bool,
}

#[derive(Debug, Clone, Copy)]
struct EndpointRef {
    segment_index: usize,
    point_index: usize,
    source_order: u32,
}

#[derive(Debug, Clone, Copy)]
enum PendingRoadPrimitive {
    Curve {
        material_index: u32,
        placement_ids: [u32; 2],
        loc1: [f32; 2],
        loc2: [f32; 2],
        scale: f32,
        width_in_texture: f32,
        radius: f32,
    },
    Tee {
        material_index: u32,
        placement_ids: [u32; 2],
        loc: [f32; 2],
        direction: [f32; 2],
        scale: f32,
        width_in_texture: f32,
        four_way: bool,
    },
    Y {
        material_index: u32,
        placement_ids: [u32; 2],
        loc: [f32; 2],
        direction: [f32; 2],
        scale: f32,
    },
    H {
        material_index: u32,
        placement_ids: [u32; 2],
        loc: [f32; 2],
        direction: [f32; 2],
        scale: f32,
        width_in_texture: f32,
        flip: bool,
    },
    AlphaJoin {
        material_index: u32,
        placement_ids: [u32; 2],
        loc: [f32; 2],
        direction: [f32; 2],
        scale: f32,
        width_in_texture: f32,
    },
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
        let mut pending_segments = Vec::new();
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
            let direction = [delta[0] / length, delta[1] / length];
            let half_width = definition.road_width() * definition.road_width_in_texture() * 0.5;
            let normal = [-direction[1] * half_width, direction[0] * half_width];
            pending_segments.push(PendingRoadSegment {
                points: [
                    PendingRoadPoint {
                        loc: first_xy,
                        top: add(first_xy, normal),
                        bottom: sub(first_xy, normal),
                        is_angled: first.flags() & object_flags::ROAD_CORNER_ANGLED != 0,
                        is_join: first.flags() & object_flags::ROAD_JOIN != 0,
                    },
                    PendingRoadPoint {
                        loc: second_xy,
                        top: add(second_xy, normal),
                        bottom: sub(second_xy, normal),
                        is_angled: second.flags() & object_flags::ROAD_CORNER_ANGLED != 0,
                        is_join: second.flags() & object_flags::ROAD_JOIN != 0,
                    },
                ],
                scale: definition.road_width(),
                width_in_texture: definition.road_width_in_texture(),
                curve_radius: if first.flags() & object_flags::ROAD_CORNER_TIGHT != 0 {
                    TIGHT_CORNER_RADIUS
                } else {
                    CORNER_RADIUS
                },
                material_index,
                source_order: u32::try_from(object_index)
                    .map_err(|_| RoadStagingError::GeometryTooLarge)?,
                placement_ids: [first.placement_id(), second.placement_id()],
                retains_corner_or_join_flags,
            });
            object_index += 2;
        }
        let (primitives, material_stacking) =
            preprocess_legacy_topology(&mut pending_segments, staged.materials.len())?;
        for segment in pending_segments {
            staged.push_pending_segment(height, segment)?;
        }
        for primitive in primitives {
            staged.push_pending_primitive(height, primitive)?;
        }
        staged.draws.sort_by_key(|draw| {
            usize::try_from(draw.material_index)
                .ok()
                .and_then(|index| material_stacking.get(index))
                .copied()
                .unwrap_or(u32::MAX)
        });
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

    fn push_pending_segment(
        &mut self,
        height: &MapHeightField,
        segment: PendingRoadSegment,
    ) -> Result<(), RoadStagingError> {
        let road_vector = sub(segment.points[1].loc, segment.points[0].loc);
        let Some(direction) = normalized(road_vector) else {
            return Ok(());
        };
        let half_width = segment.scale * segment.width_in_texture * 0.5;
        self.push_quad_section(
            height,
            [
                segment.points[0].bottom,
                segment.points[1].bottom,
                segment.points[0].top,
                segment.points[1].top,
            ],
            segment.points[0].loc,
            direction,
            left_normal(direction),
            0.0,
            SOURCE_REGULAR_V_CENTER,
            segment.scale,
            segment.scale,
            segment.material_index,
            segment.placement_ids,
            RoadDrawKind::Segment,
            segment.retains_corner_or_join_flags,
            half_width,
        )
    }

    #[allow(clippy::too_many_lines)]
    fn push_pending_primitive(
        &mut self,
        height: &MapHeightField,
        primitive: PendingRoadPrimitive,
    ) -> Result<(), RoadStagingError> {
        match primitive {
            PendingRoadPrimitive::Curve {
                material_index,
                placement_ids,
                loc1,
                loc2,
                scale,
                width_in_texture,
                radius,
            } => {
                let Some(direction) = normalized(sub(loc2, loc1)) else {
                    return Ok(());
                };
                let normal = mul(left_normal(direction), width_in_texture * scale * 0.5);
                let road = mul(direction, scale);
                let mut corners = [
                    sub(loc1, normal),
                    add(sub(loc1, normal), road),
                    add(loc1, normal),
                    add(add(loc1, normal), road),
                ];
                let v_offset = if radius == TIGHT_CORNER_RADIUS {
                    425.0 / 512.0
                } else {
                    255.0 / 512.0
                };
                if radius == TIGHT_CORNER_RADIUS {
                    corners[1] = add(add(corners[0], mul(road, 0.6)), mul(normal, 0.2));
                    corners[0] = sub(sub(corners[0], mul(normal, 0.1)), mul(road, 0.02));
                    corners[2] = sub(corners[2], mul(road, 0.02));
                    corners[3] = add(add(loc1, mul(normal, 1.2)), mul(road, 0.1));
                } else {
                    corners[1] = add(add(corners[1], mul(road, 0.1)), mul(normal, 0.4));
                    corners[0] = sub(sub(corners[0], mul(normal, 0.2)), mul(road, 0.02));
                    corners[2] = sub(corners[2], mul(road, 0.02));
                    corners[3] = add(
                        add(add(sub(loc1, normal), road), mul(normal, 2.4)),
                        mul(road, -0.4),
                    );
                }
                self.push_quad_section(
                    height,
                    corners,
                    loc1,
                    direction,
                    left_normal(direction),
                    4.0 / 512.0,
                    v_offset,
                    scale,
                    scale,
                    material_index,
                    placement_ids,
                    RoadDrawKind::Curve,
                    false,
                    width_in_texture * scale * 0.5,
                )
            }
            PendingRoadPrimitive::Tee {
                material_index,
                placement_ids,
                loc,
                direction,
                scale,
                width_in_texture,
                four_way,
            } => {
                let direction = normalized(direction).unwrap_or([1.0, 0.0]);
                let normal = left_normal(direction);
                let left = width_in_texture * scale * 0.5;
                let right = scale * TEE_WIDTH_ADJUSTMENT * 0.5;
                let left_center = sub(loc, mul(direction, left));
                let along = mul(direction, left + right);
                let across = mul(normal, right);
                let corners = [
                    sub(left_center, across),
                    add(sub(left_center, across), along),
                    add(left_center, across),
                    add(add(left_center, across), along),
                ];
                self.push_quad_section(
                    height,
                    corners,
                    loc,
                    direction,
                    normal,
                    425.0 / 512.0,
                    if four_way {
                        425.0 / 512.0
                    } else {
                        255.0 / 512.0
                    },
                    scale,
                    scale,
                    material_index,
                    placement_ids,
                    RoadDrawKind::Junction,
                    false,
                    right,
                )
            }
            PendingRoadPrimitive::Y {
                material_index,
                placement_ids,
                loc,
                direction,
                scale,
            } => {
                let direction = normalized(direction).unwrap_or([1.0, 0.0]);
                let road = mul(direction, scale * 1.59);
                let normal = mul(left_normal(direction), scale);
                let top_left = sub(add(loc, mul(normal, 0.29)), mul(road, 0.5));
                let bottom_left = sub(top_left, mul(normal, 1.08));
                self.push_quad_section(
                    height,
                    [
                        bottom_left,
                        add(bottom_left, road),
                        top_left,
                        add(top_left, road),
                    ],
                    loc,
                    direction,
                    left_normal(direction),
                    255.0 / 512.0,
                    226.0 / 512.0,
                    scale,
                    scale,
                    material_index,
                    placement_ids,
                    RoadDrawKind::Junction,
                    false,
                    scale * 0.54,
                )
            }
            PendingRoadPrimitive::H {
                material_index,
                placement_ids,
                loc,
                direction,
                scale,
                width_in_texture,
                flip,
            } => {
                let direction = normalized(direction).unwrap_or([1.0, 0.0]);
                let road = mul(direction, scale);
                let normal = mul(left_normal(direction), scale * 1.35);
                let bottom_left = if flip {
                    sub(
                        sub(loc, mul(normal, 0.20)),
                        mul(road, width_in_texture * 0.5),
                    )
                } else {
                    sub(
                        sub(loc, mul(normal, 0.8)),
                        mul(road, width_in_texture * 0.5),
                    )
                };
                let width = mul(road, width_in_texture * 0.5 + 1.2);
                self.push_quad_section(
                    height,
                    [
                        bottom_left,
                        add(bottom_left, width),
                        add(bottom_left, normal),
                        add(add(bottom_left, width), normal),
                    ],
                    loc,
                    direction,
                    if flip {
                        mul(left_normal(direction), -1.0)
                    } else {
                        left_normal(direction)
                    },
                    202.0 / 512.0,
                    364.0 / 512.0,
                    scale,
                    scale,
                    material_index,
                    placement_ids,
                    RoadDrawKind::Junction,
                    false,
                    scale * 0.675,
                )
            }
            PendingRoadPrimitive::AlphaJoin {
                material_index,
                placement_ids,
                loc,
                direction,
                scale,
                width_in_texture,
            } => {
                let direction = normalized(direction).unwrap_or([1.0, 0.0]);
                let along = mul(direction, scale * 48.0 / 128.0);
                let across = mul(
                    left_normal(direction),
                    width_in_texture * (1.0 + 8.0 / 128.0),
                );
                let top_left = sub(add(loc, mul(across, 0.5)), mul(along, 0.65));
                let corners = [
                    sub(top_left, across),
                    add(sub(top_left, across), along),
                    top_left,
                    add(top_left, along),
                ];
                self.push_quad_section(
                    height,
                    corners,
                    loc,
                    direction,
                    left_normal(direction),
                    106.0 / 512.0,
                    425.0 / 512.0,
                    scale,
                    width_in_texture,
                    material_index,
                    placement_ids,
                    RoadDrawKind::Junction,
                    false,
                    width_in_texture * (1.0 + 8.0 / 128.0) * 0.5,
                )
            }
        }
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss,
        clippy::too_many_arguments
    )]
    fn push_quad_section(
        &mut self,
        height: &MapHeightField,
        corners: [[f32; 2]; 4],
        uv_origin: [f32; 2],
        uv_direction: [f32; 2],
        uv_normal: [f32; 2],
        u_offset: f32,
        v_offset: f32,
        u_scale: f32,
        v_scale: f32,
        material_index: u32,
        placement_ids: [u32; 2],
        kind: RoadDrawKind,
        retains_corner_or_join_flags: bool,
        half_width_hint: f32,
    ) -> Result<(), RoadStagingError> {
        let length = distance(corners[0], corners[1]).max(distance(corners[2], corners[3]));
        if !length.is_finite() || length <= f32::EPSILON {
            return Ok(());
        }
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
        let across_count =
            usize::try_from((half_width_hint.abs() * 2.0 / TERRAIN_XY_SCALE).ceil() as u64)
                .map_err(|_| RoadStagingError::GeometryTooLarge)?
                .saturating_add(1)
                .clamp(2, MAX_ACROSS_SAMPLES);
        for column in 0..column_count {
            #[allow(clippy::cast_precision_loss)]
            let fraction = column as f32 / (column_count - 1) as f32;
            let bottom = lerp(corners[0], corners[1], fraction);
            let top = lerp(corners[2], corners[3], fraction);
            let mut maximum_height = f32::NEG_INFINITY;
            for sample in 0..across_count {
                #[allow(clippy::cast_precision_loss)]
                let across_fraction = sample as f32 / (across_count - 1) as f32;
                maximum_height = maximum_height
                    .max(max_cell_height(height, lerp(bottom, top, across_fraction))?);
            }
            let z = maximum_height + FLOAT_ABOVE_TERRAIN;
            for point in [top, bottom] {
                let relative = sub(point, uv_origin);
                self.vertices.push(RoadVertex {
                    position: [point[0], point[1], z],
                    uv: [
                        u_offset + dot(uv_direction, relative) / (u_scale * 4.0),
                        v_offset - dot(uv_normal, relative) / (v_scale * 4.0),
                    ],
                });
            }
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
            kind,
            material_index,
            first_index,
            index_count: u32::try_from(added_indices)
                .map_err(|_| RoadStagingError::GeometryTooLarge)?,
            first_placement_id: placement_ids[0],
            second_placement_id: placement_ids[1],
            retains_corner_or_join_flags,
        });
        Ok(())
    }
}

fn preprocess_legacy_topology(
    segments: &mut [PendingRoadSegment],
    material_count: usize,
) -> Result<(Vec<PendingRoadPrimitive>, Vec<u32>), RoadStagingError> {
    let mut grouped = BTreeMap::<(u32, u32, u32), Vec<EndpointRef>>::new();
    for (segment_index, segment) in segments.iter().enumerate() {
        for point_index in 0..2 {
            let loc = segment.points[point_index].loc;
            grouped
                .entry((loc[0].to_bits(), loc[1].to_bits(), segment.material_index))
                .or_default()
                .push(EndpointRef {
                    segment_index,
                    point_index,
                    source_order: segment
                        .source_order
                        .saturating_add(u32::from(point_index != 0)),
                });
        }
    }
    let cross_join_endpoints = grouped
        .values()
        .filter(|group| group.len() == 1)
        .filter_map(|group| {
            let endpoint = group[0];
            segments[endpoint.segment_index].points[endpoint.point_index]
                .is_join
                .then_some(endpoint)
        })
        .collect::<Vec<_>>();
    let mut groups = grouped.into_values().collect::<Vec<_>>();
    groups.sort_by_key(|group| {
        group
            .iter()
            .map(|endpoint| endpoint.source_order)
            .min()
            .unwrap_or(u32::MAX)
    });
    let mut primitives = Vec::new();
    for mut group in groups {
        group.sort_by_key(|endpoint| endpoint.source_order);
        match group.len() {
            2 if group[0].segment_index != group[1].segment_index => {
                insert_corner(segments, &group, &mut primitives);
            }
            3 => insert_three_way(segments, &group, &mut primitives),
            4 => insert_four_way(segments, &group, &mut primitives),
            _ => {}
        }
        if primitives.len() > MAX_ROAD_VERTICES {
            return Err(RoadStagingError::GeometryTooLarge);
        }
    }
    let mut material_stacking = vec![0; material_count];
    insert_cross_type_joins(
        segments,
        &cross_join_endpoints,
        &mut primitives,
        &mut material_stacking,
    );
    Ok((primitives, material_stacking))
}

fn insert_cross_type_joins(
    segments: &mut [PendingRoadSegment],
    endpoints: &[EndpointRef],
    primitives: &mut Vec<PendingRoadPrimitive>,
    material_stacking: &mut [u32],
) {
    for endpoint in endpoints {
        let segment = segments[endpoint.segment_index];
        let loc = segment.points[endpoint.point_index].loc;
        let Some(outbound) = endpoint_outbound(&segment, endpoint.point_index) else {
            continue;
        };
        let incoming = mul(outbound, -1.0);
        let mut join_direction = mul(incoming, 100.0);
        let mut other_material = None;
        for candidate in &*segments {
            if candidate.material_index == segment.material_index {
                continue;
            }
            let candidate_vector = sub(candidate.points[1].loc, candidate.points[0].loc);
            let length_squared = dot(candidate_vector, candidate_vector);
            if length_squared <= f32::EPSILON {
                continue;
            }
            let parameter = (dot(sub(loc, candidate.points[0].loc), candidate_vector)
                / length_squared)
                .clamp(0.0, 1.0);
            let closest = add(candidate.points[0].loc, mul(candidate_vector, parameter));
            if distance(closest, loc) >= candidate.scale * 0.55 {
                continue;
            }
            let Some(candidate_direction) = normalized(candidate_vector) else {
                continue;
            };
            let normal = left_normal(candidate_direction);
            join_direction = if cross(candidate_direction, incoming) > 0.0 {
                normal
            } else {
                mul(normal, -1.0)
            };
            other_material = Some(candidate.material_index);
            break;
        }
        if let Some(other_material) = other_material {
            let half_width = segment.scale * segment.width_in_texture * 0.5;
            let normal = left_normal(outbound);
            let join_line = left_normal(join_direction);
            if let (Some(left), Some(right)) = (
                line_intersection(add(loc, mul(normal, half_width)), outbound, loc, join_line),
                line_intersection(sub(loc, mul(normal, half_width)), outbound, loc, join_line),
            ) && distance(loc, left) <= MITER_LIMIT * half_width
                && distance(loc, right) <= MITER_LIMIT * half_width
            {
                set_endpoint_sides(
                    &mut segments[endpoint.segment_index],
                    endpoint.point_index,
                    left,
                    right,
                );
            }
            adjust_material_stacking(material_stacking, segment.material_index, other_material);
        }
        let endpoint_width = distance(
            segments[endpoint.segment_index].points[endpoint.point_index].top,
            segments[endpoint.segment_index].points[endpoint.point_index].bottom,
        );
        let alpha_width_in_texture = endpoint_width / segment.width_in_texture;
        primitives.push(PendingRoadPrimitive::AlphaJoin {
            material_index: segment.material_index,
            placement_ids: segment.placement_ids,
            loc,
            direction: join_direction,
            scale: segment.scale,
            width_in_texture: alpha_width_in_texture,
        });
    }
}

fn adjust_material_stacking(stacking: &mut [u32], top: u32, bottom: u32) {
    let (Ok(top), Ok(bottom)) = (usize::try_from(top), usize::try_from(bottom)) else {
        return;
    };
    let (Some(&top_order), Some(&bottom_order)) = (stacking.get(top), stacking.get(bottom)) else {
        return;
    };
    if top_order > bottom_order {
        return;
    }
    for order in &mut *stacking {
        if *order > bottom_order {
            *order = order.saturating_add(1);
        }
    }
    stacking[top] = bottom_order.saturating_add(1);
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn insert_corner(
    segments: &mut [PendingRoadSegment],
    endpoints: &[EndpointRef],
    primitives: &mut Vec<PendingRoadPrimitive>,
) {
    let first = endpoints[0];
    let second = endpoints[1];
    let shared = segments[first.segment_index].points[first.point_index].loc;
    let Some(first_out) = endpoint_outbound(&segments[first.segment_index], first.point_index)
    else {
        return;
    };
    let Some(second_out) = endpoint_outbound(&segments[second.segment_index], second.point_index)
    else {
        return;
    };
    let incoming = mul(second_out, -1.0);
    let turn_dot = dot(incoming, first_out).clamp(-1.0, 1.0);
    let angle = turn_dot.acos();
    if angle <= 0.01 {
        return;
    }
    let is_angled = segments[first.segment_index].points[first.point_index].is_angled
        || segments[second.segment_index].points[second.point_index].is_angled;
    if is_angled || angle / CURVE_STEP_RADIANS < 0.9 {
        miter_corner(segments, first, second, shared, first_out, second_out);
        return;
    }
    let radius_scale = segments[first.segment_index].curve_radius;
    let scale = segments[first.segment_index].scale;
    let radius = radius_scale * scale;
    let tangent_distance = radius * (angle * 0.5).tan();
    let first_length = endpoint_length(&segments[first.segment_index], first.point_index);
    let second_length = endpoint_length(&segments[second.segment_index], second.point_index);
    if !tangent_distance.is_finite()
        || tangent_distance + 0.5 >= first_length
        || tangent_distance + 0.5 >= second_length
    {
        miter_corner(segments, first, second, shared, first_out, second_out);
        return;
    }
    let tangent_out = add(shared, mul(first_out, tangent_distance));
    let tangent_in = add(shared, mul(second_out, tangent_distance));
    let turn_sign = cross(incoming, first_out).signum();
    if turn_sign == 0.0 {
        return;
    }
    let center = add(tangent_in, mul(left_normal(incoming), radius * turn_sign));
    let steps = ((angle / CURVE_STEP_RADIANS).ceil() as usize).clamp(1, MAX_CURVE_STEPS);
    let source_cross = cross(first_out, incoming);
    let (curve_start, initial_direction, curve_end, final_direction) = if source_cross > 0.0 {
        (tangent_in, second_out, tangent_out, mul(first_out, -1.0))
    } else {
        (tangent_out, first_out, tangent_in, mul(second_out, -1.0))
    };
    let start_radius = sub(curve_start, center);
    let material_index = segments[first.segment_index].material_index;
    let width_in_texture = segments[first.segment_index].width_in_texture;
    let placement_ids = [
        segments[first.segment_index].placement_ids[first.point_index],
        segments[second.segment_index].placement_ids[second.point_index],
    ];
    set_endpoint_center(
        &mut segments[first.segment_index],
        first.point_index,
        tangent_out,
    );
    set_endpoint_center(
        &mut segments[second.segment_index],
        second.point_index,
        tangent_in,
    );
    for step in 0..steps {
        let is_source_final = steps > 1 && step + 1 == steps;
        let (loc1, piece_direction) = if is_source_final {
            (curve_end, final_direction)
        } else {
            let piece_angle = -CURVE_STEP_RADIANS * (step + 1) as f32;
            (
                add(center, rotate(start_radius, piece_angle)),
                rotate(initial_direction, piece_angle),
            )
        };
        let loc2 = add(loc1, piece_direction);
        primitives.push(PendingRoadPrimitive::Curve {
            material_index,
            placement_ids,
            loc1,
            loc2,
            scale,
            width_in_texture,
            radius: radius_scale,
        });
    }
}

fn miter_corner(
    segments: &mut [PendingRoadSegment],
    first: EndpointRef,
    second: EndpointRef,
    shared: [f32; 2],
    first_out: [f32; 2],
    second_out: [f32; 2],
) {
    let first_half =
        segments[first.segment_index].scale * segments[first.segment_index].width_in_texture * 0.5;
    let second_half = segments[second.segment_index].scale
        * segments[second.segment_index].width_in_texture
        * 0.5;
    let first_normal = left_normal(first_out);
    let second_normal = left_normal(second_out);
    let Some(left_join) = line_intersection(
        add(shared, mul(first_normal, first_half)),
        first_out,
        sub(shared, mul(second_normal, second_half)),
        second_out,
    ) else {
        return;
    };
    let Some(right_join) = line_intersection(
        sub(shared, mul(first_normal, first_half)),
        first_out,
        add(shared, mul(second_normal, second_half)),
        second_out,
    ) else {
        return;
    };
    let limit = MITER_LIMIT * first_half.max(second_half);
    if distance(shared, left_join) > limit || distance(shared, right_join) > limit {
        return;
    }
    set_endpoint_sides(
        &mut segments[first.segment_index],
        first.point_index,
        left_join,
        right_join,
    );
    set_endpoint_sides(
        &mut segments[second.segment_index],
        second.point_index,
        right_join,
        left_join,
    );
}

#[allow(clippy::too_many_lines)]
fn insert_three_way(
    segments: &mut [PendingRoadSegment],
    endpoints: &[EndpointRef],
    primitives: &mut Vec<PendingRoadPrimitive>,
) {
    let Some(directions) = endpoint_directions(segments, endpoints) else {
        return;
    };
    let shared = segments[endpoints[0].segment_index].points[endpoints[0].point_index].loc;
    let scale = segments[endpoints[0].segment_index].scale;
    let width_in_texture = segments[endpoints[0].segment_index].width_in_texture;
    let material_index = segments[endpoints[0].segment_index].material_index;
    if let Some((first_leg, second_leg, stem)) = classify_y(&directions) {
        let up = mul(directions[stem], scale * 0.5);
        let (negative_leg, positive_leg) = if cross(directions[stem], directions[first_leg]) < 0.0 {
            (first_leg, second_leg)
        } else {
            (second_leg, first_leg)
        };
        let negative_arm = rotate(up, -3.0 * std::f32::consts::PI / 4.0);
        let positive_arm = rotate(up, 3.0 * std::f32::consts::PI / 4.0);
        set_endpoint_center(
            &mut segments[endpoints[negative_leg].segment_index],
            endpoints[negative_leg].point_index,
            add(shared, mul(negative_arm, 1.1)),
        );
        set_endpoint_center(
            &mut segments[endpoints[positive_leg].segment_index],
            endpoints[positive_leg].point_index,
            add(shared, mul(positive_arm, 1.1)),
        );
        set_endpoint_center(
            &mut segments[endpoints[stem].segment_index],
            endpoints[stem].point_index,
            add(shared, mul(up, 0.55)),
        );
        primitives.push(PendingRoadPrimitive::Y {
            material_index,
            placement_ids: [
                segments[endpoints[first_leg].segment_index].placement_ids
                    [endpoints[first_leg].point_index],
                segments[endpoints[stem].segment_index].placement_ids[endpoints[stem].point_index],
            ],
            loc: shared,
            direction: rotate(up, -std::f32::consts::PI / 2.0),
            scale,
        });
        return;
    }

    let (first_trunk, second_trunk) = most_opposite_pair(&directions);
    let arm = (0..3)
        .find(|index| *index != first_trunk && *index != second_trunk)
        .unwrap_or(2);
    let Some(up_direction) = normalized(sub(directions[second_trunk], directions[first_trunk]))
    else {
        return;
    };
    let up = mul(up_direction, scale * 0.5);
    let mirror = cross(up_direction, directions[arm]) < 0.0;
    let tee = rotate(
        up,
        if mirror {
            -std::f32::consts::PI / 2.0
        } else {
            std::f32::consts::PI / 2.0
        },
    );
    let placement_ids = [
        segments[endpoints[first_trunk].segment_index].placement_ids
            [endpoints[first_trunk].point_index],
        segments[endpoints[arm].segment_index].placement_ids[endpoints[arm].point_index],
    ];
    if dot(up_direction, directions[arm]).abs() > 0.5 {
        let flip = cross(tee, directions[arm]) > 0.0;
        let (first_factor, second_factor) = if flip == mirror {
            (-0.46, 2.05)
        } else {
            (-2.05, 0.46)
        };
        set_endpoint_center(
            &mut segments[endpoints[first_trunk].segment_index],
            endpoints[first_trunk].point_index,
            add(shared, mul(up, first_factor)),
        );
        set_endpoint_center(
            &mut segments[endpoints[second_trunk].segment_index],
            endpoints[second_trunk].point_index,
            add(shared, mul(up, second_factor)),
        );
        let arm_offset = rotate(
            tee,
            if flip {
                std::f32::consts::PI / 4.0
            } else {
                -std::f32::consts::PI / 4.0
            },
        );
        set_endpoint_center(
            &mut segments[endpoints[arm].segment_index],
            endpoints[arm].point_index,
            add(shared, mul(arm_offset, 2.1)),
        );
        primitives.push(PendingRoadPrimitive::H {
            material_index,
            placement_ids,
            loc: shared,
            direction: tee,
            scale,
            width_in_texture,
            flip,
        });
        return;
    }
    set_endpoint_center(
        &mut segments[endpoints[first_trunk].segment_index],
        endpoints[first_trunk].point_index,
        sub(shared, up),
    );
    set_endpoint_center(
        &mut segments[endpoints[second_trunk].segment_index],
        endpoints[second_trunk].point_index,
        add(shared, up),
    );
    set_endpoint_center(
        &mut segments[endpoints[arm].segment_index],
        endpoints[arm].point_index,
        add(shared, tee),
    );
    primitives.push(PendingRoadPrimitive::Tee {
        material_index,
        placement_ids,
        loc: shared,
        direction: tee,
        scale,
        width_in_texture,
        four_way: false,
    });
}

fn classify_y(directions: &[[f32; 2]]) -> Option<(usize, usize, usize)> {
    const COS_30: f32 = 0.866;
    if dot(directions[0], directions[1]) < -COS_30
        || dot(directions[0], directions[2]) < -COS_30
        || dot(directions[1], directions[2]) < -COS_30
    {
        return None;
    }
    let mut best = None;
    for stem in 0..3 {
        let legs = (0..3).filter(|index| *index != stem).collect::<Vec<_>>();
        if cross(directions[stem], directions[legs[0]]).signum()
            == cross(directions[stem], directions[legs[1]]).signum()
        {
            continue;
        }
        let score = (dot(directions[stem], directions[legs[0]]) + std::f32::consts::FRAC_1_SQRT_2)
            .abs()
            + (dot(directions[stem], directions[legs[1]]) + std::f32::consts::FRAC_1_SQRT_2).abs();
        if best.is_none_or(|(_, _, _, best_score)| score < best_score) {
            best = Some((legs[0], legs[1], stem, score));
        }
    }
    best.map(|(first, second, stem, _)| (first, second, stem))
}

fn insert_four_way(
    segments: &mut [PendingRoadSegment],
    endpoints: &[EndpointRef],
    primitives: &mut Vec<PendingRoadPrimitive>,
) {
    let Some(directions) = endpoint_directions(segments, endpoints) else {
        return;
    };
    let (first_axis, second_axis) = most_opposite_pair(&directions);
    let shared = segments[endpoints[0].segment_index].points[endpoints[0].point_index].loc;
    let scale = segments[endpoints[0].segment_index].scale;
    let mut direction = normalized(sub(directions[second_axis], directions[first_axis]))
        .unwrap_or(directions[second_axis]);
    let align = mul(direction, scale * 0.5);
    let remaining = (0..4)
        .filter(|index| *index != first_axis && *index != second_axis)
        .collect::<Vec<_>>();
    let tee = rotate(
        align,
        if cross(align, directions[remaining[0]]) < 0.0 {
            -std::f32::consts::PI / 2.0
        } else {
            std::f32::consts::PI / 2.0
        },
    );
    for (index, center) in [
        (first_axis, sub(shared, align)),
        (second_axis, add(shared, align)),
        (remaining[0], add(shared, tee)),
        (remaining[1], sub(shared, tee)),
    ] {
        set_endpoint_center(
            &mut segments[endpoints[index].segment_index],
            endpoints[index].point_index,
            center,
        );
    }
    if direction[0] < 0.0 {
        direction = mul(direction, -1.0);
    }
    primitives.push(PendingRoadPrimitive::Tee {
        material_index: segments[endpoints[0].segment_index].material_index,
        placement_ids: [
            segments[endpoints[first_axis].segment_index].placement_ids
                [endpoints[first_axis].point_index],
            segments[endpoints[second_axis].segment_index].placement_ids
                [endpoints[second_axis].point_index],
        ],
        loc: shared,
        direction,
        scale,
        width_in_texture: TEE_WIDTH_ADJUSTMENT,
        four_way: true,
    });
}

fn endpoint_directions(
    segments: &[PendingRoadSegment],
    endpoints: &[EndpointRef],
) -> Option<Vec<[f32; 2]>> {
    endpoints
        .iter()
        .map(|endpoint| endpoint_outbound(&segments[endpoint.segment_index], endpoint.point_index))
        .collect()
}

fn most_opposite_pair(directions: &[[f32; 2]]) -> (usize, usize) {
    let mut pair = (0, 1);
    let mut pair_dot = dot(directions[0], directions[1]);
    for first in 0..directions.len() {
        for second in first + 1..directions.len() {
            let candidate = dot(directions[first], directions[second]);
            if candidate < pair_dot {
                pair = (first, second);
                pair_dot = candidate;
            }
        }
    }
    pair
}

fn endpoint_outbound(segment: &PendingRoadSegment, point_index: usize) -> Option<[f32; 2]> {
    normalized(sub(
        segment.points[1 - point_index].loc,
        segment.points[point_index].loc,
    ))
}

fn endpoint_length(segment: &PendingRoadSegment, point_index: usize) -> f32 {
    distance(
        segment.points[point_index].loc,
        segment.points[1 - point_index].loc,
    )
}

fn set_endpoint_center(segment: &mut PendingRoadSegment, point_index: usize, loc: [f32; 2]) {
    segment.points[point_index].loc = loc;
    let Some(direction) = normalized(sub(segment.points[1].loc, segment.points[0].loc)) else {
        return;
    };
    let normal = mul(
        left_normal(direction),
        segment.scale * segment.width_in_texture * 0.5,
    );
    segment.points[point_index].top = add(loc, normal);
    segment.points[point_index].bottom = sub(loc, normal);
}

fn set_endpoint_sides(
    segment: &mut PendingRoadSegment,
    point_index: usize,
    left: [f32; 2],
    right: [f32; 2],
) {
    if point_index == 0 {
        segment.points[point_index].top = left;
        segment.points[point_index].bottom = right;
    } else {
        segment.points[point_index].top = right;
        segment.points[point_index].bottom = left;
    }
}

fn line_intersection(
    first: [f32; 2],
    first_direction: [f32; 2],
    second: [f32; 2],
    second_direction: [f32; 2],
) -> Option<[f32; 2]> {
    let denominator = cross(first_direction, second_direction);
    if denominator.abs() <= 1.0e-5 {
        return None;
    }
    let parameter = cross(sub(second, first), second_direction) / denominator;
    let result = add(first, mul(first_direction, parameter));
    result[0]
        .is_finite()
        .then_some(result)
        .filter(|point| point[1].is_finite())
}

fn add(left: [f32; 2], right: [f32; 2]) -> [f32; 2] {
    [left[0] + right[0], left[1] + right[1]]
}

fn sub(left: [f32; 2], right: [f32; 2]) -> [f32; 2] {
    [left[0] - right[0], left[1] - right[1]]
}

fn mul(vector: [f32; 2], scalar: f32) -> [f32; 2] {
    [vector[0] * scalar, vector[1] * scalar]
}

fn dot(left: [f32; 2], right: [f32; 2]) -> f32 {
    left[0] * right[0] + left[1] * right[1]
}

fn cross(left: [f32; 2], right: [f32; 2]) -> f32 {
    left[0] * right[1] - left[1] * right[0]
}

fn left_normal(vector: [f32; 2]) -> [f32; 2] {
    [-vector[1], vector[0]]
}

fn normalized(vector: [f32; 2]) -> Option<[f32; 2]> {
    let length = vector[0].hypot(vector[1]);
    (length.is_finite() && length > f32::EPSILON).then(|| mul(vector, length.recip()))
}

fn distance(first: [f32; 2], second: [f32; 2]) -> f32 {
    let delta = sub(second, first);
    delta[0].hypot(delta[1])
}

fn rotate(vector: [f32; 2], angle: f32) -> [f32; 2] {
    let (sin, cos) = angle.sin_cos();
    [
        vector[0] * cos - vector[1] * sin,
        vector[0] * sin + vector[1] * cos,
    ]
}

fn lerp(first: [f32; 2], second: [f32; 2], amount: f32) -> [f32; 2] {
    add(first, mul(sub(second, first), amount))
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

    use crate::{TERRAIN_HEIGHT_SCALE, TextureResourceManager};

    use super::{
        CORNER_RADIUS, CURVE_STEP_RADIANS, FLOAT_ABOVE_TERRAIN, MITER_LIMIT, PendingRoadPoint,
        PendingRoadPrimitive, PendingRoadSegment, RoadDiagnosticKind, RoadDrawKind,
        RoadStagingError, SOURCE_REGULAR_V_CENTER, StagedRoads, TEE_WIDTH_ADJUSTMENT,
        TIGHT_CORNER_RADIUS, adjust_material_stacking, classify_y, preprocess_legacy_topology,
    };

    #[test]
    fn source_constants_and_empty_constructor_are_exact() {
        assert_eq!(CORNER_RADIUS.to_bits(), 1.5_f32.to_bits());
        assert_eq!(TIGHT_CORNER_RADIUS.to_bits(), 0.5_f32.to_bits());
        assert_eq!(
            CURVE_STEP_RADIANS.to_bits(),
            (std::f32::consts::PI / 6.0).to_bits()
        );
        assert_eq!(TEE_WIDTH_ADJUSTMENT.to_bits(), 1.03_f32.to_bits());
        assert_eq!(MITER_LIMIT.to_bits(), 4.0_f32.to_bits());
        assert_eq!(
            SOURCE_REGULAR_V_CENTER.to_bits(),
            (85.0_f32 / 512.0).to_bits()
        );
        assert_eq!(
            FLOAT_ABOVE_TERRAIN.to_bits(),
            (TERRAIN_HEIGHT_SCALE / 8.0).to_bits()
        );
        let roads = StagedRoads::empty();
        assert!(roads.vertices().is_empty());
        assert!(roads.indices().is_empty());
        assert!(roads.materials().is_empty());
        assert!(roads.draws().is_empty());
        assert!(roads.diagnostics().is_empty());
    }

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
        assert_eq!(roads.draws().len(), 2);
        assert_eq!(roads.materials().len(), 1);
        assert!(roads.vertices().len() >= 12);
        assert!(roads.indices().len() >= 24);
        assert!(
            roads
                .vertices()
                .iter()
                .all(|vertex| vertex.position()[2] > 2.5)
        );
        assert!(roads.diagnostics().is_empty());
        assert!(
            roads
                .draws()
                .iter()
                .all(|draw| draw.kind() == RoadDrawKind::Segment)
        );
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

    #[test]
    #[allow(clippy::too_many_lines)]
    fn every_endpoint_input_failure_has_a_stable_diagnostic() {
        let valid_ini = parse_road_ini(
            b"Road SyntheticRoad\n Texture = road.tga\n RoadWidth = 10\n RoadWidthInTexture = 1\nEnd\n",
            RoadIniLimits::default(),
        )
        .expect("road INI");
        let invalid_ini = parse_road_ini(
            b"Road SyntheticRoad\n Texture = road.tga\n RoadWidth = 0\n RoadWidthInTexture = 1\nEnd\n",
            RoadIniLimits::default(),
        )
        .expect("invalid road remains immutable input");
        let no_texture_ini = parse_road_ini(
            b"Road SyntheticRoad\n RoadWidth = 10\n RoadWidthInTexture = 1\nEnd\n",
            RoadIniLimits::default(),
        )
        .expect("missing texture definition");
        let mut textures = TextureResourceManager::default();
        textures
            .insert(b"road.tga", 1, 1, vec![100, 90, 80, 255])
            .expect("texture");
        let empty_textures = TextureResourceManager::default();

        let cases = [
            (
                vec![object([0.0, 0.0, 0.0], 0x4, b"SyntheticRoad")],
                valid_ini.definitions(),
                &textures,
                RoadDiagnosticKind::UnexpectedSecondEndpoint,
            ),
            (
                vec![object([0.0, 0.0, 0.0], 0x2, b"SyntheticRoad")],
                valid_ini.definitions(),
                &textures,
                RoadDiagnosticKind::MissingSecondEndpoint,
            ),
            (
                vec![
                    object([0.0, 0.0, 0.0], 0x2, b"SyntheticRoad"),
                    object([10.0, 0.0, 0.0], 0, b"NotAnEndpoint"),
                ],
                valid_ini.definitions(),
                &textures,
                RoadDiagnosticKind::MissingSecondEndpoint,
            ),
            (
                vec![
                    object([0.0, 0.0, 0.0], 0x2, b"UnknownRoad"),
                    object([10.0, 0.0, 0.0], 0x4, b"UnknownRoad"),
                ],
                valid_ini.definitions(),
                &textures,
                RoadDiagnosticKind::MissingDefinition,
            ),
            (
                vec![
                    object([0.0, 0.0, 0.0], 0x2, b"SyntheticRoad"),
                    object([10.0, 0.0, 0.0], 0x4, b"SyntheticRoad"),
                ],
                invalid_ini.definitions(),
                &textures,
                RoadDiagnosticKind::InvalidDefinition,
            ),
            (
                vec![
                    object([0.0, 0.0, 0.0], 0x2, b"SyntheticRoad"),
                    object([10.0, 0.0, 0.0], 0x4, b"SyntheticRoad"),
                ],
                no_texture_ini.definitions(),
                &textures,
                RoadDiagnosticKind::MissingTexture,
            ),
            (
                vec![
                    object([0.0, 0.0, 0.0], 0x2, b"SyntheticRoad"),
                    object([10.0, 0.0, 0.0], 0x4, b"SyntheticRoad"),
                ],
                valid_ini.definitions(),
                &empty_textures,
                RoadDiagnosticKind::MissingTexture,
            ),
            (
                vec![
                    object([0.0, 0.0, 0.0], 0x2, b"SyntheticRoad"),
                    object([0.0, 0.0, 0.0], 0x4, b"SyntheticRoad"),
                ],
                valid_ini.definitions(),
                &textures,
                RoadDiagnosticKind::ZeroLength,
            ),
        ];
        for (records, definitions, resources, expected) in cases {
            let records = records.concat();
            let bytes = fixture_with_objects(&records);
            let map = parse_map(&bytes, "diagnostic.map", MapLimits::default()).expect("map");
            let height = decode_map_height(&map, MapLimits::default()).expect("height");
            let world =
                decode_map_world_objects(&map, MapScenarioLimits::default()).expect("objects");
            let roads =
                StagedRoads::from_map(&world, &height, definitions, resources).expect("roads");
            assert_eq!(roads.diagnostics().len(), 1);
            assert_eq!(roads.diagnostics()[0].kind(), expected);
            assert!(roads.draws().is_empty());
        }

        let object = object([0.0, 0.0, 0.0], 0, b"Scenery");
        let bytes = fixture_with_objects(&object);
        let map = parse_map(&bytes, "non-road.map", MapLimits::default()).expect("map");
        let height = decode_map_height(&map, MapLimits::default()).expect("height");
        let world = decode_map_world_objects(&map, MapScenarioLimits::default()).expect("objects");
        let roads = StagedRoads::from_map(&world, &height, valid_ini.definitions(), &textures)
            .expect("roads");
        assert!(roads.draws().is_empty());
        assert!(roads.diagnostics().is_empty());
    }

    #[test]
    fn source_radius_corner_uses_three_bounded_atlas_pieces() {
        let ini = parse_road_ini(
            b"Road SyntheticRoad\n Texture = road.tga\n RoadWidth = 10\n RoadWidthInTexture = 1\nEnd\n",
            RoadIniLimits::default(),
        )
        .expect("road INI");
        let mut textures = TextureResourceManager::default();
        textures
            .insert(b"road.tga", 1, 1, vec![100, 90, 80, 255])
            .expect("texture");
        for (name, end_y) in [
            ("left-curved-roads.map", 20.0),
            ("right-curved-roads.map", -20.0),
        ] {
            let map = parse_map(
                &fixture_with_corner_direction(0, end_y),
                name,
                MapLimits::default(),
            )
            .expect("map");
            let height = decode_map_height(&map, MapLimits::default()).expect("height");
            let world =
                decode_map_world_objects(&map, MapScenarioLimits::default()).expect("objects");
            let roads = StagedRoads::from_map(&world, &height, ini.definitions(), &textures)
                .expect("roads");
            assert_eq!(
                roads
                    .draws()
                    .iter()
                    .filter(|draw| draw.kind() == RoadDrawKind::Curve)
                    .count(),
                3
            );
            assert!(roads.vertices().iter().all(|vertex| {
                let [x, y, _] = vertex.position();
                x.abs() < 100.0 && y.abs() < 100.0
            }));
        }
    }

    #[test]
    fn cross_material_join_preserves_source_width_and_stacking() {
        let mut segments = [
            test_segment([-20.0, 0.0], [0.0, 0.0], 0, true),
            test_segment([-10.0, -10.0], [10.0, 10.0], 1, false),
        ];
        let (primitives, stacking) =
            preprocess_legacy_topology(&mut segments, 2).expect("topology");
        assert_eq!(stacking, [1, 0]);
        let alpha_width = primitives
            .iter()
            .find_map(|primitive| match primitive {
                PendingRoadPrimitive::AlphaJoin {
                    width_in_texture, ..
                } => Some(*width_in_texture),
                _ => None,
            })
            .expect("alpha join");
        assert!((alpha_width - 10.0_f32 * 2.0_f32.sqrt()).abs() < 0.001);
    }

    #[test]
    fn material_stacking_matches_every_source_branch() {
        let mut stacking = [0, 0, 0, 0];
        adjust_material_stacking(&mut stacking, 0, 1);
        assert_eq!(stacking, [1, 0, 0, 0]);
        adjust_material_stacking(&mut stacking, 2, 0);
        assert_eq!(stacking, [1, 0, 2, 0]);
        adjust_material_stacking(&mut stacking, 3, 1);
        assert_eq!(stacking, [2, 0, 3, 1]);
        let established = stacking;
        adjust_material_stacking(&mut stacking, 2, 1);
        assert_eq!(stacking, established, "already-higher material moved");
        adjust_material_stacking(&mut stacking, 99, 0);
        adjust_material_stacking(&mut stacking, 0, 99);
        assert_eq!(stacking, established, "out-of-range material changed order");
    }

    #[test]
    fn topology_noop_and_open_join_inputs_are_stable() {
        let mut isolated = radial_segments(&[[1.0, 0.0]]);
        let (primitives, stacking) =
            preprocess_legacy_topology(&mut isolated, 1).expect("isolated road");
        assert!(primitives.is_empty());
        assert_eq!(stacking, [0]);

        let mut continuation = radial_segments(&[[1.0, 0.0], [-1.0, 0.0]]);
        let original = continuation
            .iter()
            .map(|segment| {
                segment
                    .points
                    .map(|point| (point.loc, point.top, point.bottom))
            })
            .collect::<Vec<_>>();
        let (primitives, _) =
            preprocess_legacy_topology(&mut continuation, 1).expect("straight continuation");
        assert!(primitives.is_empty());
        assert_eq!(
            continuation
                .iter()
                .map(|segment| {
                    segment
                        .points
                        .map(|point| (point.loc, point.top, point.bottom))
                })
                .collect::<Vec<_>>(),
            original
        );

        let mut overfull =
            radial_segments(&[[1.0, 0.0], [0.0, 1.0], [-1.0, 0.0], [0.0, -1.0], [0.7, 0.7]]);
        let (primitives, _) =
            preprocess_legacy_topology(&mut overfull, 1).expect("unsupported junction degree");
        assert!(primitives.is_empty());

        let mut open_join = [test_segment([-20.0, 0.0], [0.0, 0.0], 0, true)];
        let (primitives, stacking) =
            preprocess_legacy_topology(&mut open_join, 1).expect("open alpha cap");
        assert_eq!(stacking, [0]);
        let [
            PendingRoadPrimitive::AlphaJoin {
                direction,
                width_in_texture,
                ..
            },
        ] = primitives.as_slice()
        else {
            panic!("expected one open alpha cap");
        };
        assert_eq!(
            direction.map(f32::to_bits),
            [100.0_f32.to_bits(), (-0.0_f32).to_bits()]
        );
        assert_eq!(width_in_texture.to_bits(), 10.0_f32.to_bits());
    }

    #[test]
    fn geometry_inputs_skip_degenerate_and_reject_unbounded_sections() {
        let map = parse_map(&fixture(), "geometry.map", MapLimits::default()).expect("map");
        let height = decode_map_height(&map, MapLimits::default()).expect("height");
        let mut roads = StagedRoads::empty();
        let arguments = (
            [0.0, 0.0],
            [1.0, 0.0],
            [0.0, 1.0],
            0.0,
            0.0,
            1.0,
            1.0,
            0,
            [0, 1],
            RoadDrawKind::Segment,
            false,
            5.0,
        );
        roads
            .push_quad_section(
                &height,
                [[0.0, 0.0]; 4],
                arguments.0,
                arguments.1,
                arguments.2,
                arguments.3,
                arguments.4,
                arguments.5,
                arguments.6,
                arguments.7,
                arguments.8,
                arguments.9,
                arguments.10,
                arguments.11,
            )
            .expect("degenerate section is ignored");
        assert!(roads.draws().is_empty());

        assert_eq!(
            roads.push_quad_section(
                &height,
                [
                    [0.0, 0.0],
                    [1_000_010.0, 0.0],
                    [0.0, 10.0],
                    [1_000_010.0, 10.0],
                ],
                arguments.0,
                arguments.1,
                arguments.2,
                arguments.3,
                arguments.4,
                arguments.5,
                arguments.6,
                arguments.7,
                arguments.8,
                arguments.9,
                arguments.10,
                arguments.11,
            ),
            Err(RoadStagingError::GeometryTooLarge)
        );
    }

    #[test]
    fn topology_dispatches_curve_miter_tight_tee_y_h_and_four_way() {
        let mut regular_curve = radial_segments(&[[1.0, 0.0], [0.0, 1.0]]);
        let (primitives, _) =
            preprocess_legacy_topology(&mut regular_curve, 1).expect("regular curve");
        assert_eq!(
            primitives
                .iter()
                .filter(|primitive| matches!(primitive, PendingRoadPrimitive::Curve { radius, .. } if *radius == CORNER_RADIUS))
                .count(),
            3
        );

        let mut tight_curve = radial_segments(&[[1.0, 0.0], [0.0, 1.0]]);
        tight_curve[0].curve_radius = TIGHT_CORNER_RADIUS;
        let (primitives, _) = preprocess_legacy_topology(&mut tight_curve, 1).expect("tight curve");
        assert_eq!(
            primitives
                .iter()
                .filter(|primitive| matches!(primitive, PendingRoadPrimitive::Curve { radius, .. } if *radius == TIGHT_CORNER_RADIUS))
                .count(),
            3
        );

        let mut angled = radial_segments(&[[1.0, 0.0], [0.0, 1.0]]);
        angled[0].points[0].is_angled = true;
        let (primitives, _) = preprocess_legacy_topology(&mut angled, 1).expect("angled miter");
        assert!(primitives.is_empty());
        assert_point(angled[0].points[0].top, [5.0, 5.0]);
        assert_point(angled[0].points[0].bottom, [-5.0, -5.0]);

        let mut too_short = radial_segments(&[[1.0, 0.0], [0.0, 1.0]]);
        for segment in &mut too_short {
            segment.points[1].loc = [
                segment.points[1].loc[0] * 0.1,
                segment.points[1].loc[1] * 0.1,
            ];
        }
        let (primitives, _) = preprocess_legacy_topology(&mut too_short, 1).expect("short miter");
        assert!(primitives.is_empty());

        let mut tee = radial_segments(&[[1.0, 0.0], [-1.0, 0.0], [0.0, 1.0]]);
        let (primitives, _) = preprocess_legacy_topology(&mut tee, 1).expect("tee");
        assert!(matches!(
            primitives.as_slice(),
            [PendingRoadPrimitive::Tee {
                four_way: false,
                ..
            }]
        ));

        let diagonal = std::f32::consts::FRAC_1_SQRT_2;
        let y_directions = [[1.0, 0.0], [-diagonal, diagonal], [-diagonal, -diagonal]];
        assert!(classify_y(&y_directions).is_some());
        let mut y = radial_segments(&y_directions);
        let (primitives, _) = preprocess_legacy_topology(&mut y, 1).expect("Y");
        assert!(matches!(
            primitives.as_slice(),
            [PendingRoadPrimitive::Y { .. }]
        ));

        for (arm, expected_flip) in [([0.8, 0.6], false), ([0.8, -0.6], true)] {
            let mut h = radial_segments(&[[1.0, 0.0], [-1.0, 0.0], arm]);
            let (primitives, _) = preprocess_legacy_topology(&mut h, 1).expect("slanted tee");
            assert!(matches!(
                primitives.as_slice(),
                [PendingRoadPrimitive::H { flip, .. }] if *flip == expected_flip
            ));
        }

        let mut four_way = radial_segments(&[[1.0, 0.0], [-1.0, 0.0], [0.0, 1.0], [0.0, -1.0]]);
        let (primitives, _) = preprocess_legacy_topology(&mut four_way, 1).expect("four way");
        assert!(matches!(
            primitives.as_slice(),
            [PendingRoadPrimitive::Tee {
                four_way: true,
                width_in_texture,
                ..
            }] if *width_in_texture == TEE_WIDTH_ADJUSTMENT
        ));

        assert!(classify_y(&[[1.0, 0.0], [-1.0, 0.0], [0.0, 1.0]]).is_none());
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn source_atlas_functions_emit_exact_geometry_and_uv_inputs() {
        assert_primitive(
            PendingRoadPrimitive::Curve {
                material_index: 0,
                placement_ids: [1, 2],
                loc1: [0.0, 0.0],
                loc2: [1.0, 0.0],
                scale: 10.0,
                width_in_texture: 1.0,
                radius: CORNER_RADIUS,
            },
            RoadDrawKind::Curve,
            [-0.2, 5.0],
            [-0.2, -6.0],
            [0.002_812_5, 0.373_046_88],
            [0.002_812_5, 0.648_046_85],
        );
        assert_primitive(
            PendingRoadPrimitive::Curve {
                material_index: 0,
                placement_ids: [1, 2],
                loc1: [0.0, 0.0],
                loc2: [1.0, 0.0],
                scale: 10.0,
                width_in_texture: 1.0,
                radius: TIGHT_CORNER_RADIUS,
            },
            RoadDrawKind::Curve,
            [-0.2, 5.0],
            [-0.2, -5.5],
            [0.002_812_5, 0.705_078_1],
            [0.002_812_5, 0.967_578_1],
        );
        assert_primitive(
            PendingRoadPrimitive::Tee {
                material_index: 0,
                placement_ids: [1, 2],
                loc: [0.0, 0.0],
                direction: [1.0, 0.0],
                scale: 10.0,
                width_in_texture: 1.0,
                four_way: false,
            },
            RoadDrawKind::Junction,
            [-5.0, 5.15],
            [-5.0, -5.15],
            [0.705_078_1, 0.369_296_88],
            [0.705_078_1, 0.626_796_9],
        );
        assert_primitive(
            PendingRoadPrimitive::Tee {
                material_index: 0,
                placement_ids: [1, 2],
                loc: [0.0, 0.0],
                direction: [1.0, 0.0],
                scale: 10.0,
                width_in_texture: TEE_WIDTH_ADJUSTMENT,
                four_way: true,
            },
            RoadDrawKind::Junction,
            [-5.15, 5.15],
            [-5.15, -5.15],
            [0.701_328_1, 0.701_328_1],
            [0.701_328_1, 0.958_828_1],
        );
        assert_primitive(
            PendingRoadPrimitive::Y {
                material_index: 0,
                placement_ids: [1, 2],
                loc: [0.0, 0.0],
                direction: [1.0, 0.0],
                scale: 10.0,
            },
            RoadDrawKind::Junction,
            [-7.95, 2.9],
            [-7.95, -7.9],
            [0.299_296_86, 0.368_906_26],
            [0.299_296_86, 0.638_906_24],
        );
        for (flip, top, bottom, top_uv, bottom_uv) in [
            (
                false,
                [-5.0, 2.7],
                [-5.0, -10.8],
                [0.269_531_25, 0.643_437_5],
                [0.269_531_25, 0.980_937_5],
            ),
            (
                true,
                [-5.0, 10.8],
                [-5.0, -2.7],
                [0.269_531_25, 0.980_937_5],
                [0.269_531_25, 0.643_437_5],
            ),
        ] {
            assert_primitive(
                PendingRoadPrimitive::H {
                    material_index: 0,
                    placement_ids: [1, 2],
                    loc: [0.0, 0.0],
                    direction: [1.0, 0.0],
                    scale: 10.0,
                    width_in_texture: 1.0,
                    flip,
                },
                RoadDrawKind::Junction,
                top,
                bottom,
                top_uv,
                bottom_uv,
            );
        }
        assert_primitive(
            PendingRoadPrimitive::AlphaJoin {
                material_index: 0,
                placement_ids: [1, 2],
                loc: [0.0, 0.0],
                direction: [1.0, 0.0],
                scale: 10.0,
                width_in_texture: 10.0,
            },
            RoadDrawKind::Junction,
            [-2.4375, 5.3125],
            [-2.4375, -5.3125],
            [0.146_093_76, 0.697_265_6],
            [0.146_093_76, 0.962_890_6],
        );
    }

    fn assert_primitive(
        primitive: PendingRoadPrimitive,
        kind: RoadDrawKind,
        expected_top: [f32; 2],
        expected_bottom: [f32; 2],
        expected_top_uv: [f32; 2],
        expected_bottom_uv: [f32; 2],
    ) {
        let map = parse_map(&fixture(), "primitive.map", MapLimits::default()).expect("map");
        let height = decode_map_height(&map, MapLimits::default()).expect("height");
        let mut roads = StagedRoads::empty();
        roads
            .push_pending_primitive(&height, primitive)
            .expect("primitive");
        assert_eq!(roads.draws().len(), 1);
        assert_eq!(roads.draws()[0].kind(), kind);
        assert_point_3d(
            roads.vertices()[0].position(),
            [expected_top[0], expected_top[1], 2.5 + FLOAT_ABOVE_TERRAIN],
        );
        assert_point_3d(
            roads.vertices()[1].position(),
            [
                expected_bottom[0],
                expected_bottom[1],
                2.5 + FLOAT_ABOVE_TERRAIN,
            ],
        );
        assert_point(roads.vertices()[0].uv(), expected_top_uv);
        assert_point(roads.vertices()[1].uv(), expected_bottom_uv);
    }

    fn assert_point(actual: [f32; 2], expected: [f32; 2]) {
        for (actual, expected) in actual.into_iter().zip(expected) {
            assert!(
                (actual - expected).abs() <= 1.0e-5,
                "{actual} differs from {expected}"
            );
        }
    }

    fn assert_point_3d(actual: [f32; 3], expected: [f32; 3]) {
        for (actual, expected) in actual.into_iter().zip(expected) {
            assert!(
                (actual - expected).abs() <= 1.0e-5,
                "{actual} differs from {expected}"
            );
        }
    }

    fn radial_segments(directions: &[[f32; 2]]) -> Vec<PendingRoadSegment> {
        directions
            .iter()
            .enumerate()
            .map(|(index, direction)| {
                let mut segment = test_segment(
                    [0.0, 0.0],
                    [direction[0] * 100.0, direction[1] * 100.0],
                    0,
                    false,
                );
                segment.source_order = u32::try_from(index * 2).expect("source order");
                segment.placement_ids = [
                    u32::try_from(index * 2).expect("placement"),
                    u32::try_from(index * 2 + 1).expect("placement"),
                ];
                segment
            })
            .collect()
    }

    fn test_segment(
        start: [f32; 2],
        end: [f32; 2],
        material_index: u32,
        join_at_end: bool,
    ) -> PendingRoadSegment {
        let delta = [end[0] - start[0], end[1] - start[1]];
        let length = delta[0].hypot(delta[1]);
        let direction = [delta[0] / length, delta[1] / length];
        let normal = [-direction[1] * 5.0, direction[0] * 5.0];
        PendingRoadSegment {
            points: [
                PendingRoadPoint {
                    loc: start,
                    top: [start[0] + normal[0], start[1] + normal[1]],
                    bottom: [start[0] - normal[0], start[1] - normal[1]],
                    is_angled: false,
                    is_join: false,
                },
                PendingRoadPoint {
                    loc: end,
                    top: [end[0] + normal[0], end[1] + normal[1]],
                    bottom: [end[0] - normal[0], end[1] - normal[1]],
                    is_angled: false,
                    is_join: join_at_end,
                },
            ],
            scale: 10.0,
            width_in_texture: 1.0,
            curve_radius: 1.5,
            material_index,
            source_order: material_index * 2,
            placement_ids: [material_index * 2, material_index * 2 + 1],
            retains_corner_or_join_flags: join_at_end,
        }
    }

    fn fixture() -> Vec<u8> {
        fixture_with_corner_flags(0x40)
    }

    fn fixture_with_corner_flags(corner_flags: u32) -> Vec<u8> {
        fixture_with_corner_direction(corner_flags, 20.0)
    }

    fn fixture_with_corner_direction(corner_flags: u32, end_y: f32) -> Vec<u8> {
        let mut objects = Vec::new();
        objects.extend_from_slice(&object([0.0, 0.0, 0.0], 0x2, b"SyntheticRoad"));
        objects.extend_from_slice(&object(
            [20.0, 0.0, 0.0],
            0x4 | corner_flags,
            b"SyntheticRoad",
        ));
        objects.extend_from_slice(&object(
            [20.0, 0.0, 0.0],
            0x2 | corner_flags,
            b"SyntheticRoad",
        ));
        objects.extend_from_slice(&object([20.0, end_y, 0.0], 0x4, b"SyntheticRoad"));
        fixture_with_objects(&objects)
    }

    fn fixture_with_objects(objects: &[u8]) -> Vec<u8> {
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
        push_chunk(&mut bytes, 3, 3, objects);
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
