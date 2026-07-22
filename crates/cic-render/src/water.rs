// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Stable renderer staging for immutable water footprints.
//!
//! Input shape semantics come from the bounded project decoder. Standing-water texture, diffuse,
//! opacity, and polygon-strip policy follow `W3DWater.cpp` from `GeneralsGameCode` revision
//! `9f7abb866f5afd446db14149979e744c7216baaf` under GPL-3.0 with its Section 7 terms; see
//! `docs/provenance/map.md`. Modern shading remains original project work.

use cic_formats::{MapWaterArea, MapWaterData, MapWaterPoint};

use crate::RenderError;

const MAX_WATER_VERTICES: usize = 1_000_000;
const MAX_WATER_INDICES: usize = 6_000_000;
const MAX_CAUSTIC_FRAMES: usize = 64;
const MAX_CAUSTIC_BYTES: usize = 16 * 1_024 * 1_024;
const MAX_WATER_SURFACE_DIMENSION: u32 = 4_096;
const MAX_WATER_SURFACE_BYTES: usize = 64 * 1_024 * 1_024;

/// Explicit presentation policy for source-compatible or project-authored water.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaterPresentationPolicy {
    /// Preserve source standing-water texture, tint, alpha, and depth-feather semantics.
    ZeroHourLegacy,
    /// Use the project-authored refractive water presentation.
    Modern,
}

/// One bounded renderer-owned standing-water texture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaterSurfaceTexture {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl WaterSurfaceTexture {
    /// Validates a complete bounded RGBA surface texture.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::InvalidTexture`] for zero or mismatched dimensions and
    /// [`RenderError::TextureTooLarge`] when the explicit surface limits are exceeded.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, RenderError> {
        if width == 0 || height == 0 {
            return Err(RenderError::InvalidTexture);
        }
        if width > MAX_WATER_SURFACE_DIMENSION || height > MAX_WATER_SURFACE_DIMENSION {
            return Err(RenderError::TextureTooLarge);
        }
        let expected = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|texels| texels.checked_mul(4))
            .ok_or(RenderError::TextureTooLarge)?;
        if expected > MAX_WATER_SURFACE_BYTES || rgba.len() != expected {
            return Err(RenderError::InvalidTexture);
        }
        Ok(Self {
            width,
            height,
            rgba,
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
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }
}

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
    surface_texture: Option<WaterSurfaceTexture>,
    sky_texture: Option<WaterSurfaceTexture>,
    environment_texture: Option<WaterSurfaceTexture>,
    minimum_opacity: f32,
    opaque_depth: f32,
    source_surface_rgba: Option<[f32; 4]>,
    source_scroll_per_ms: [f32; 2],
    additive_blending: bool,
    presentation: WaterPresentationPolicy,
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
            surface_texture: None,
            sky_texture: None,
            environment_texture: None,
            minimum_opacity: 1.0,
            opaque_depth: 3.0,
            source_surface_rgba: None,
            source_scroll_per_ms: [0.0; 2],
            additive_blending: false,
            presentation: WaterPresentationPolicy::ZeroHourLegacy,
        }
    }

    #[must_use]
    pub const fn with_caustics(caustics: WaterCausticSequence) -> Self {
        Self {
            caustics: Some(caustics),
            surface_texture: None,
            sky_texture: None,
            environment_texture: None,
            minimum_opacity: 1.0,
            opaque_depth: 3.0,
            source_surface_rgba: None,
            source_scroll_per_ms: [0.0; 2],
            additive_blending: false,
            presentation: WaterPresentationPolicy::ZeroHourLegacy,
        }
    }

    /// Applies source water-transparency values without coupling the renderer to INI ownership.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::InvalidMaterial`] for non-finite/out-of-range values.
    pub fn with_transparency(
        mut self,
        minimum_opacity: f32,
        opaque_depth: f32,
    ) -> Result<Self, RenderError> {
        if !minimum_opacity.is_finite()
            || !(0.0..=1.0).contains(&minimum_opacity)
            || !opaque_depth.is_finite()
            || opaque_depth <= 0.0
            || opaque_depth > 10_000.0
        {
            return Err(RenderError::InvalidMaterial);
        }
        self.minimum_opacity = minimum_opacity;
        self.opaque_depth = opaque_depth;
        Ok(self)
    }

    #[must_use]
    pub const fn caustics(&self) -> Option<&WaterCausticSequence> {
        self.caustics.as_ref()
    }

    #[must_use]
    pub const fn minimum_opacity(&self) -> f32 {
        self.minimum_opacity
    }

    #[must_use]
    pub const fn opaque_depth(&self) -> f32 {
        self.opaque_depth
    }

    /// Applies the selected `WaterSet` diffuse color and explicit presentation motion inputs.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::InvalidMaterial`] for non-finite/out-of-range values.
    pub fn with_source_surface(
        mut self,
        rgba: Option<[f32; 4]>,
        scroll_per_ms: [f32; 2],
    ) -> Result<Self, RenderError> {
        if rgba.is_some_and(|color| {
            color
                .into_iter()
                .any(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
        }) || scroll_per_ms
            .into_iter()
            .any(|value| !value.is_finite() || value.abs() > 1_000_000.0)
        {
            return Err(RenderError::InvalidMaterial);
        }
        self.source_surface_rgba = rgba;
        self.source_scroll_per_ms = scroll_per_ms;
        Ok(self)
    }

    #[must_use]
    pub const fn source_surface_rgba(&self) -> Option<[f32; 4]> {
        self.source_surface_rgba
    }

    #[must_use]
    pub const fn source_scroll_per_ms(&self) -> [f32; 2] {
        self.source_scroll_per_ms
    }

    /// Applies source standing-water texture/blend inputs.
    #[must_use]
    pub fn with_standing_surface(
        mut self,
        texture: Option<WaterSurfaceTexture>,
        additive_blending: bool,
    ) -> Self {
        self.surface_texture = texture;
        self.additive_blending = additive_blending;
        self
    }

    /// Selects source-compatible or project-authored water presentation explicitly.
    #[must_use]
    pub const fn with_presentation(mut self, presentation: WaterPresentationPolicy) -> Self {
        self.presentation = presentation;
        self
    }

    #[must_use]
    pub const fn surface_texture(&self) -> Option<&WaterSurfaceTexture> {
        self.surface_texture.as_ref()
    }

    /// Applies resolved selected-`WaterSet` sky and water/environment textures.
    #[must_use]
    pub fn with_environment_textures(
        mut self,
        sky_texture: Option<WaterSurfaceTexture>,
        environment_texture: Option<WaterSurfaceTexture>,
    ) -> Self {
        self.sky_texture = sky_texture;
        self.environment_texture = environment_texture;
        self
    }

    #[must_use]
    pub const fn sky_texture(&self) -> Option<&WaterSurfaceTexture> {
        self.sky_texture.as_ref()
    }

    #[must_use]
    pub const fn environment_texture(&self) -> Option<&WaterSurfaceTexture> {
        self.environment_texture.as_ref()
    }

    #[must_use]
    pub const fn additive_blending(&self) -> bool {
        self.additive_blending
    }

    #[must_use]
    pub const fn presentation(&self) -> WaterPresentationPolicy {
        self.presentation
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
            if area.is_river() {
                stage_river(area, &mut vertices, &mut indices)?;
            } else {
                stage_lake(area, &mut vertices, &mut indices)?;
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

fn stage_lake(
    area: &MapWaterArea,
    vertices: &mut Vec<[f32; 3]>,
    indices: &mut Vec<u32>,
) -> Result<(), RenderError> {
    if area.points().len() < 3 {
        return Ok(());
    }
    let base = checked_vertex_base(vertices, area.points().len())?;
    vertices.extend(area.points().iter().copied().map(water_position));
    for point in 1..area.points().len() - 1 {
        let point = u32::try_from(point).map_err(|_| RenderError::GeometryTooLarge)?;
        push_indices(indices, [base, base + point, base + point + 1])?;
    }
    Ok(())
}

fn stage_river(
    area: &MapWaterArea,
    vertices: &mut Vec<[f32; 3]>,
    indices: &mut Vec<u32>,
) -> Result<(), RenderError> {
    let points = area.points();
    if points.len() < 4 {
        return Ok(());
    }
    let Ok(mut backward) = usize::try_from(area.river_start()) else {
        return Ok(());
    };
    if backward >= points.len() - 1 {
        return Ok(());
    }
    let pair_count = points.len() / 2;
    let vertex_count = pair_count
        .checked_mul(2)
        .ok_or(RenderError::GeometryTooLarge)?;
    let base = checked_vertex_base(vertices, vertex_count)?;
    let mut forward = backward + 1;
    for _ in 0..pair_count {
        vertices.push(water_position(points[forward]));
        vertices.push(water_position(points[backward]));
        forward += 1;
        if forward == points.len() {
            forward = 0;
        }
        backward = backward.checked_sub(1).unwrap_or(points.len() - 1);
    }
    for pair in 0..pair_count.saturating_sub(1) {
        let offset = u32::try_from(pair * 2).map_err(|_| RenderError::GeometryTooLarge)?;
        push_indices(
            indices,
            [base + offset, base + offset + 1, base + offset + 3],
        )?;
        push_indices(
            indices,
            [base + offset, base + offset + 3, base + offset + 2],
        )?;
    }
    Ok(())
}

fn checked_vertex_base(vertices: &[[f32; 3]], additional: usize) -> Result<u32, RenderError> {
    let following = vertices
        .len()
        .checked_add(additional)
        .ok_or(RenderError::GeometryTooLarge)?;
    if following > MAX_WATER_VERTICES {
        return Err(RenderError::GeometryTooLarge);
    }
    u32::try_from(vertices.len()).map_err(|_| RenderError::GeometryTooLarge)
}

#[allow(clippy::cast_precision_loss)]
fn water_position(point: MapWaterPoint) -> [f32; 3] {
    let [x, y, z] = point.coordinates();
    [x as f32, y as f32, z as f32]
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

    use super::{
        StagedWater, WaterAppearance, WaterCausticSequence, WaterPresentationPolicy,
        WaterSurfaceTexture,
    };

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
        let surface = WaterSurfaceTexture::new(1, 1, vec![10, 20, 30, 40])
            .expect("valid standing-water texture");
        let appearance = WaterAppearance::default()
            .with_transparency(0.8, 2.0)
            .expect("valid source water values")
            .with_source_surface(Some([0.1, 0.2, 0.3, 0.5]), [0.001, -0.002])
            .expect("valid source surface")
            .with_standing_surface(Some(surface), true)
            .with_environment_textures(
                Some(WaterSurfaceTexture::new(1, 1, vec![1, 2, 3, 255]).expect("sky")),
                Some(WaterSurfaceTexture::new(1, 1, vec![4, 5, 6, 255]).expect("environment")),
            )
            .with_presentation(WaterPresentationPolicy::Modern);
        assert_eq!(appearance.minimum_opacity().to_bits(), 0.8_f32.to_bits());
        assert_eq!(appearance.opaque_depth().to_bits(), 2.0_f32.to_bits());
        assert_eq!(appearance.source_surface_rgba(), Some([0.1, 0.2, 0.3, 0.5]));
        assert_eq!(
            appearance.surface_texture().expect("surface").rgba(),
            [10, 20, 30, 40]
        );
        assert!(appearance.additive_blending());
        assert_eq!(
            appearance.sky_texture().expect("sky").rgba(),
            [1, 2, 3, 255]
        );
        assert_eq!(
            appearance
                .environment_texture()
                .expect("environment")
                .rgba(),
            [4, 5, 6, 255]
        );
        assert_eq!(appearance.presentation(), WaterPresentationPolicy::Modern);
        assert!(WaterSurfaceTexture::new(2, 2, vec![0; 15]).is_err());
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
            0,
            &[[0, 0, 3], [10, 0, 3], [10, 10, 3], [0, 10, 3]],
        );
        add_area(
            &mut payload,
            b"river",
            true,
            2,
            &[
                [40, 5, 4],
                [30, 5, 4],
                [20, 5, 4],
                [20, 0, 4],
                [30, 0, 4],
                [40, 0, 4],
            ],
        );
        let map = parse_map(&map_bytes(&payload), "water.map", MapLimits::default()).expect("map");
        let decoded = decode_map_water(&map, MapLimits::default()).expect("water");
        let staged = StagedWater::from_map(&decoded).expect("staged water");
        assert_eq!(staged.area_count(), 2);
        assert_eq!(staged.vertices().len(), 10);
        assert_eq!(
            &staged.vertices()[4..],
            &[
                [20.0, 0.0, 4.0],
                [20.0, 5.0, 4.0],
                [30.0, 0.0, 4.0],
                [30.0, 5.0, 4.0],
                [40.0, 0.0, 4.0],
                [40.0, 5.0, 4.0],
            ]
        );
        assert_eq!(
            staged.indices(),
            [0, 1, 2, 0, 2, 3, 4, 5, 7, 4, 7, 6, 6, 7, 9, 6, 9, 8]
        );
    }

    #[test]
    fn invalid_river_seams_produce_no_geometry() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&2_i32.to_le_bytes());
        let points = [[0, 0, 3], [0, 5, 3], [10, 5, 3], [10, 0, 3]];
        add_area(&mut payload, b"negative", true, -1, &points);
        add_area(&mut payload, b"past-end", true, 3, &points);
        let map = parse_map(&map_bytes(&payload), "seams.map", MapLimits::default()).expect("map");
        let decoded = decode_map_water(&map, MapLimits::default()).expect("water");
        let staged = StagedWater::from_map(&decoded).expect("bounded invalid seams");
        assert_eq!(staged.area_count(), 2);
        assert!(staged.vertices().is_empty());
        assert!(staged.indices().is_empty());
    }

    fn add_area(
        payload: &mut Vec<u8>,
        name: &[u8],
        river: bool,
        river_start: i32,
        points: &[[i32; 3]],
    ) {
        payload.extend_from_slice(
            &u16::try_from(name.len())
                .expect("test name fits u16")
                .to_le_bytes(),
        );
        payload.extend_from_slice(name);
        payload.extend_from_slice(&1_i32.to_le_bytes());
        payload.push(1);
        payload.push(u8::from(river));
        payload.extend_from_slice(&river_start.to_le_bytes());
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
