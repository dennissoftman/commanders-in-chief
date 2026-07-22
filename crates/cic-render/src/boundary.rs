// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Renderer-only visualization of the first playable MAP boundary.
//!
//! The source extent interpretation follows `HeightMapData.cpp` and `HeightMap.cpp` in
//! `GeneralsGameCode` revision `9f7abb866f5afd446db14149979e744c7216baaf`, licensed under
//! GPL-3.0-or-later with Electronic Arts Section 7 terms. The translucent terrain-following fence
//! is project-authored presentation and never changes simulation reachability.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

use cic_formats::MapHeightField;

use crate::TERRAIN_HEIGHT_SCALE;

const FENCE_HEIGHT: f32 = 42.0;
const FENCE_BOTTOM_OFFSET: f32 = 0.25;
const MAX_FENCE_SEGMENTS: usize = 100_000;
const FENCE_COLOR: [f32; 4] = [0.12, 0.72, 1.0, 0.24];

/// One immutable world-space boundary-fence vertex.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoundaryVertex {
    position: [f32; 3],
    color: [f32; 4],
}

impl BoundaryVertex {
    #[must_use]
    pub const fn position(self) -> [f32; 3] {
        self.position
    }

    #[must_use]
    pub const fn color(self) -> [f32; 4] {
        self.color
    }
}

/// Immutable renderer upload data for the primary playable-area fence.
#[derive(Debug, Clone, PartialEq)]
pub struct StagedBoundaryFence {
    extent_cells: [u32; 2],
    vertices: Vec<BoundaryVertex>,
    indices: Vec<u32>,
}

impl StagedBoundaryFence {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            extent_cells: [0; 2],
            vertices: Vec::new(),
            indices: Vec::new(),
        }
    }

    /// Stages the first valid source boundary as a terrain-following translucent rectangle.
    ///
    /// # Errors
    ///
    /// Returns a structured error when the source extent is non-positive, exceeds the height
    /// field, or would exceed the explicit presentation geometry limit.
    pub fn from_map(height: &MapHeightField) -> Result<Self, BoundaryStagingError> {
        let Some(boundary) = height.boundaries().first().copied() else {
            return Ok(Self::empty());
        };
        let (Ok(x), Ok(y)) = (u32::try_from(boundary.x()), u32::try_from(boundary.y())) else {
            return Ok(Self::empty());
        };
        if x == 0 || y == 0 {
            return Ok(Self::empty());
        }
        if x > height.width() || y > height.height() {
            return Err(BoundaryStagingError::OutsideHeightField {
                extent: [x, y],
                field: [height.width(), height.height()],
            });
        }
        let segment_count = usize::try_from(x)
            .ok()
            .and_then(|x| usize::try_from(y).ok().and_then(|y| x.checked_add(y)))
            .and_then(|half| half.checked_mul(2))
            .ok_or(BoundaryStagingError::GeometryTooLarge)?;
        if segment_count > MAX_FENCE_SEGMENTS {
            return Err(BoundaryStagingError::GeometryTooLarge);
        }
        let mut staged = Self {
            extent_cells: [x, y],
            vertices: Vec::with_capacity(segment_count.saturating_mul(4)),
            indices: Vec::with_capacity(segment_count.saturating_mul(6)),
        };
        let global_top = height
            .samples()
            .iter()
            .copied()
            .max()
            .map_or(FENCE_HEIGHT, |sample| {
                f32::from(sample) * TERRAIN_HEIGHT_SCALE + FENCE_HEIGHT
            });
        for cell_x in 0..x {
            staged.push_segment(height, [cell_x, 0], [cell_x + 1, 0], global_top)?;
            staged.push_segment(height, [x - cell_x, y], [x - cell_x - 1, y], global_top)?;
        }
        for cell_y in 0..y {
            staged.push_segment(height, [x, cell_y], [x, cell_y + 1], global_top)?;
            staged.push_segment(height, [0, y - cell_y], [0, y - cell_y - 1], global_top)?;
        }
        Ok(staged)
    }

    #[must_use]
    pub const fn extent_cells(&self) -> [u32; 2] {
        self.extent_cells
    }

    #[must_use]
    pub fn vertices(&self) -> &[BoundaryVertex] {
        &self.vertices
    }

    #[must_use]
    pub fn indices(&self) -> &[u32] {
        &self.indices
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

    #[allow(clippy::cast_precision_loss)]
    fn push_segment(
        &mut self,
        height: &MapHeightField,
        first: [u32; 2],
        second: [u32; 2],
        global_top: f32,
    ) -> Result<(), BoundaryStagingError> {
        let base = u32::try_from(self.vertices.len())
            .map_err(|_| BoundaryStagingError::GeometryTooLarge)?;
        let spacing = f32::from(height.cell_size_world_units());
        let first_z = sample_height(height, first)? + FENCE_BOTTOM_OFFSET;
        let second_z = sample_height(height, second)? + FENCE_BOTTOM_OFFSET;
        let first_xy = [first[0] as f32 * spacing, first[1] as f32 * spacing];
        let second_xy = [second[0] as f32 * spacing, second[1] as f32 * spacing];
        self.vertices.extend([
            boundary_vertex(first_xy, first_z),
            boundary_vertex(second_xy, second_z),
            boundary_vertex(second_xy, global_top),
            boundary_vertex(first_xy, global_top),
        ]);
        self.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        Ok(())
    }
}

fn boundary_vertex(xy: [f32; 2], z: f32) -> BoundaryVertex {
    BoundaryVertex {
        position: [xy[0], xy[1], z],
        color: FENCE_COLOR,
    }
}

fn sample_height(height: &MapHeightField, point: [u32; 2]) -> Result<f32, BoundaryStagingError> {
    let x = point[0].min(height.width().saturating_sub(1));
    let y = point[1].min(height.height().saturating_sub(1));
    let index = usize::try_from(y)
        .ok()
        .and_then(|y| {
            usize::try_from(height.width())
                .ok()
                .and_then(|width| y.checked_mul(width))
        })
        .and_then(|row| usize::try_from(x).ok().and_then(|x| row.checked_add(x)))
        .ok_or(BoundaryStagingError::GeometryTooLarge)?;
    height
        .samples()
        .get(index)
        .copied()
        .map(|sample| f32::from(sample) * TERRAIN_HEIGHT_SCALE)
        .ok_or(BoundaryStagingError::TruncatedHeightField)
}

/// A structured boundary-fence staging failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoundaryStagingError {
    InvalidExtent,
    OutsideHeightField { extent: [u32; 2], field: [u32; 2] },
    TruncatedHeightField,
    GeometryTooLarge,
}

impl Display for BoundaryStagingError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidExtent => formatter.write_str("MAP primary boundary must be positive"),
            Self::OutsideHeightField { extent, field } => write!(
                formatter,
                "MAP primary boundary {}x{} exceeds height field {}x{}",
                extent[0], extent[1], field[0], field[1]
            ),
            Self::TruncatedHeightField => formatter.write_str("MAP height samples are truncated"),
            Self::GeometryTooLarge => formatter.write_str("boundary fence exceeds geometry limits"),
        }
    }
}

impl Error for BoundaryStagingError {}

#[cfg(test)]
mod tests {
    use cic_formats::{MapLimits, decode_map_height, parse_map};

    use super::StagedBoundaryFence;

    #[test]
    fn stages_primary_boundary_as_terrain_following_quads() {
        let digits = include_str!("../../cic-formats/tests/fixtures/minimal.map.hex")
            .bytes()
            .filter(u8::is_ascii_hexdigit)
            .collect::<Vec<_>>();
        let map = digits
            .chunks_exact(2)
            .map(|pair| {
                let pair = std::str::from_utf8(pair).expect("ASCII fixture");
                u8::from_str_radix(pair, 16).expect("valid hex fixture")
            })
            .collect::<Vec<_>>();
        let parsed =
            parse_map(&map, "synthetic-boundary.map", MapLimits::default()).expect("MAP container");
        let height = decode_map_height(&parsed, MapLimits::default()).expect("height field");
        let fence = StagedBoundaryFence::from_map(&height).expect("boundary fence");
        assert_eq!(fence.extent_cells(), [3, 2]);
        assert_eq!(fence.vertices().len(), 40);
        assert_eq!(fence.indices().len(), 60);
        assert!(fence.vertices()[0].position()[2] < fence.vertices()[3].position()[2]);
        assert!(fence.vertices()[3].position()[2] > 200.0);
    }
}
