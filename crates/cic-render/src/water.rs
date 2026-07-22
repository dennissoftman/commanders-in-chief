// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Stable renderer staging for immutable water footprints.
//!
//! Input shape semantics come from the bounded project decoder. Triangulation and all shading are
//! original project implementation; no legacy renderer algorithm is used here.

use cic_formats::MapWaterData;

use crate::RenderError;

const MAX_WATER_VERTICES: usize = 1_000_000;
const MAX_WATER_INDICES: usize = 6_000_000;
const MAX_CAUSTIC_FRAMES: usize = 64;
const MAX_CAUSTIC_BYTES: usize = 16 * 1_024 * 1_024;

/// One bounded renderer-owned caustic animation supplied by the resource layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaterCausticSequence {
    width: u32,
    height: u32,
    frames_per_second: u32,
    frames: Vec<Vec<u8>>,
}

impl WaterCausticSequence {
    /// Validates same-sized linear-luminance frames for GPU array upload.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::InvalidTexture`] for empty or inconsistent inputs and
    /// [`RenderError::TextureTooLarge`] when bounded frame storage is exceeded.
    pub fn new(
        width: u32,
        height: u32,
        frames_per_second: u32,
        frames: Vec<Vec<u8>>,
    ) -> Result<Self, RenderError> {
        if width == 0
            || height == 0
            || frames_per_second == 0
            || frames_per_second > 60
            || frames.is_empty()
            || frames.len() > MAX_CAUSTIC_FRAMES
        {
            return Err(RenderError::InvalidTexture);
        }
        let frame_bytes = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .ok_or(RenderError::TextureTooLarge)?;
        let total_bytes = frame_bytes
            .checked_mul(frames.len())
            .ok_or(RenderError::TextureTooLarge)?;
        if total_bytes > MAX_CAUSTIC_BYTES || frames.iter().any(|frame| frame.len() != frame_bytes)
        {
            return Err(RenderError::InvalidTexture);
        }
        Ok(Self {
            width,
            height,
            frames_per_second,
            frames,
        })
    }

    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    #[must_use]
    pub const fn frames_per_second(&self) -> u32 {
        self.frames_per_second
    }

    #[must_use]
    pub fn frames(&self) -> &[Vec<u8>] {
        &self.frames
    }
}

/// Reusable renderer inputs for water appearance, independent of VFS and window ownership.
#[derive(Debug, Clone, PartialEq)]
pub struct WaterAppearance {
    caustics: Option<WaterCausticSequence>,
    deep_opacity: f32,
    opaque_depth: f32,
}

impl Default for WaterAppearance {
    fn default() -> Self {
        Self::without_caustics()
    }
}

impl WaterAppearance {
    #[must_use]
    pub const fn without_caustics() -> Self {
        Self {
            caustics: None,
            deep_opacity: 1.0,
            opaque_depth: 3.0,
        }
    }

    #[must_use]
    pub const fn with_caustics(caustics: WaterCausticSequence) -> Self {
        Self {
            caustics: Some(caustics),
            deep_opacity: 1.0,
            opaque_depth: 3.0,
        }
    }

    /// Applies source water-transparency values without coupling the renderer to INI ownership.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::InvalidMaterial`] for non-finite/out-of-range values.
    pub fn with_transparency(
        mut self,
        deep_opacity: f32,
        opaque_depth: f32,
    ) -> Result<Self, RenderError> {
        if !deep_opacity.is_finite()
            || !(0.0..=1.0).contains(&deep_opacity)
            || !opaque_depth.is_finite()
            || opaque_depth <= 0.0
            || opaque_depth > 10_000.0
        {
            return Err(RenderError::InvalidMaterial);
        }
        self.deep_opacity = deep_opacity;
        self.opaque_depth = opaque_depth;
        Ok(self)
    }

    #[must_use]
    pub const fn caustics(&self) -> Option<&WaterCausticSequence> {
        self.caustics.as_ref()
    }

    #[must_use]
    pub const fn deep_opacity(&self) -> f32 {
        self.deep_opacity
    }

    #[must_use]
    pub const fn opaque_depth(&self) -> f32 {
        self.opaque_depth
    }
}

/// Stable, renderer-ready water vertices and triangles in MAP source order.
#[derive(Debug, Clone, PartialEq)]
pub struct StagedWater {
    vertices: Vec<[f32; 3]>,
    indices: Vec<u32>,
    area_count: u32,
}

impl StagedWater {
    /// Creates an empty water stage for maps without an established water payload.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
            area_count: 0,
        }
    }

    /// Triangulates decoded lake polygons as fans and paired river points as strips.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::GeometryTooLarge`] if checked staging limits are exceeded.
    pub fn from_map(water: &MapWaterData) -> Result<Self, RenderError> {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for area in water.areas() {
            if area.points().len() < 3 || (area.is_river() && area.points().len() < 4) {
                continue;
            }
            let base = u32::try_from(vertices.len()).map_err(|_| RenderError::GeometryTooLarge)?;
            let following = vertices
                .len()
                .checked_add(area.points().len())
                .ok_or(RenderError::GeometryTooLarge)?;
            if following > MAX_WATER_VERTICES {
                return Err(RenderError::GeometryTooLarge);
            }
            vertices.extend(area.points().iter().map(|point| {
                let [x, y, z] = point.coordinates();
                #[allow(clippy::cast_precision_loss)]
                [x as f32, y as f32, z as f32]
            }));
            if area.is_river() {
                let pair_count = area.points().len() / 2;
                for pair in 0..pair_count.saturating_sub(1) {
                    let offset =
                        u32::try_from(pair * 2).map_err(|_| RenderError::GeometryTooLarge)?;
                    push_indices(
                        &mut indices,
                        [base + offset, base + offset + 1, base + offset + 3],
                    )?;
                    push_indices(
                        &mut indices,
                        [base + offset, base + offset + 3, base + offset + 2],
                    )?;
                }
            } else {
                for point in 1..area.points().len() - 1 {
                    let point = u32::try_from(point).map_err(|_| RenderError::GeometryTooLarge)?;
                    push_indices(&mut indices, [base, base + point, base + point + 1])?;
                }
            }
        }
        Ok(Self {
            vertices,
            indices,
            area_count: u32::try_from(water.areas().len())
                .map_err(|_| RenderError::GeometryTooLarge)?,
        })
    }

    /// Returns the number of source water areas.
    #[must_use]
    pub const fn area_count(&self) -> u32 {
        self.area_count
    }
    /// Returns staged vertices.
    #[must_use]
    pub fn vertices(&self) -> &[[f32; 3]] {
        &self.vertices
    }
    /// Returns staged triangle indices.
    #[must_use]
    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    pub(crate) fn vertex_bytes(&self) -> Vec<u8> {
        self.vertices
            .iter()
            .flatten()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }

    pub(crate) fn index_bytes(&self) -> Vec<u8> {
        self.indices
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }
}

fn push_indices(indices: &mut Vec<u32>, triangle: [u32; 3]) -> Result<(), RenderError> {
    if indices
        .len()
        .checked_add(3)
        .is_none_or(|count| count > MAX_WATER_INDICES)
    {
        return Err(RenderError::GeometryTooLarge);
    }
    indices.extend_from_slice(&triangle);
    Ok(())
}

#[cfg(test)]
mod tests {
    use cic_formats::{MapLimits, decode_map_water, parse_map};

    use super::{StagedWater, WaterAppearance, WaterCausticSequence};

    #[test]
    fn caustic_sequences_require_bounded_consistent_frames() {
        let sequence =
            WaterCausticSequence::new(2, 2, 16, vec![vec![1, 2, 3, 4]; 3]).expect("valid caustics");
        assert_eq!(sequence.frames().len(), 3);
        assert!(WaterCausticSequence::new(2, 2, 16, vec![vec![0; 3]]).is_err());
        assert!(WaterCausticSequence::new(2, 2, 0, vec![vec![0; 4]]).is_err());
    }

    #[test]
    fn water_appearance_validates_source_transparency_inputs() {
        let appearance = WaterAppearance::default()
            .with_transparency(0.8, 2.0)
            .expect("valid source water values");
        assert_eq!(appearance.deep_opacity().to_bits(), 0.8_f32.to_bits());
        assert_eq!(appearance.opaque_depth().to_bits(), 2.0_f32.to_bits());
        assert!(
            WaterAppearance::default()
                .with_transparency(1.1, 2.0)
                .is_err()
        );
    }

    #[test]
    fn triangulates_lakes_and_rivers_in_source_order() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&2_i32.to_le_bytes());
        add_area(
            &mut payload,
            b"lake",
            false,
            &[[0, 0, 3], [10, 0, 3], [10, 10, 3], [0, 10, 3]],
        );
        add_area(
            &mut payload,
            b"river",
            true,
            &[[20, 0, 4], [20, 5, 4], [30, 0, 4], [30, 5, 4]],
        );
        let map = parse_map(&map_bytes(&payload), "water.map", MapLimits::default()).expect("map");
        let decoded = decode_map_water(&map, MapLimits::default()).expect("water");
        let staged = StagedWater::from_map(&decoded).expect("staged water");
        assert_eq!(staged.area_count(), 2);
        assert_eq!(staged.vertices().len(), 8);
        assert_eq!(staged.indices(), [0, 1, 2, 0, 2, 3, 4, 5, 7, 4, 7, 6]);
    }

    fn add_area(payload: &mut Vec<u8>, name: &[u8], river: bool, points: &[[i32; 3]]) {
        payload.extend_from_slice(
            &u16::try_from(name.len())
                .expect("test name fits u16")
                .to_le_bytes(),
        );
        payload.extend_from_slice(name);
        payload.extend_from_slice(&1_i32.to_le_bytes());
        payload.push(1);
        payload.push(u8::from(river));
        payload.extend_from_slice(&0_i32.to_le_bytes());
        payload.extend_from_slice(
            &i32::try_from(points.len())
                .expect("test point count fits i32")
                .to_le_bytes(),
        );
        for point in points {
            for value in point {
                payload.extend_from_slice(&value.to_le_bytes());
            }
        }
    }

    fn map_bytes(payload: &[u8]) -> Vec<u8> {
        let mut bytes = b"CkMp".to_vec();
        bytes.extend_from_slice(&1_i32.to_le_bytes());
        bytes.push(15);
        bytes.extend_from_slice(b"PolygonTriggers");
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&3_u16.to_le_bytes());
        bytes.extend_from_slice(
            &i32::try_from(payload.len())
                .expect("test payload fits i32")
                .to_le_bytes(),
        );
        bytes.extend_from_slice(payload);
        bytes
    }
}
