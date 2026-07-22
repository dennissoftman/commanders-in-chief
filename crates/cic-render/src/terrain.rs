// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Immutable terrain staging and source-compatible base/primary/extra texture compositing.
//!
//! Geometry scale, packed tile quadrants, alpha masks, layer order, and triangle flipping are
//! derived from `WorldHeightMap.cpp`, `W3DCustomEdging.cpp`, `TerrainTex.cpp`, `TileData.cpp`,
//! and `HeightMap.cpp` in `GeneralsGameCode` revision
//! `9f7abb866f5afd446db14149979e744c7216baaf`. That source is GPL-3.0-or-later with Electronic
//! Arts Section 7 terms; full notices are recorded in `docs/provenance/map.md`.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::sync::OnceLock;

use cic_formats::{MapBlendData, MapBlendTile, MapHeightField, MapTextureClass};

use crate::TextureResourceManager;

const SOURCE_TILE_PIXELS: usize = 64;
const SOURCE_TILE_PIXELS_U32: u32 = 64;
const SOURCE_TERRAIN_ATLAS_WIDTH: f32 = 2_048.0;
const MAX_BAKED_TEXTURE_DIMENSION: usize = 4_096;
const MAX_BAKED_TEXTURE_BYTES: usize = 64 * 1_024 * 1_024;
const MAX_VIRTUAL_SOURCE_TILES: usize = 4_096;
const VIRTUAL_CELL_BYTES: usize = 160;
const MAX_VIRTUAL_CELL_BUFFER_BYTES: usize = 64 * 1_024 * 1_024;
const MAX_TERRAIN_VERTICES: usize = 4_000_000;
const DETAIL_REGION_QUANTUM_CELLS: u32 = 8;
const DETAIL_MIN_MARGIN_CELLS: u32 = 12;
const INVERTED_MASK: u8 = 0x01;
const FORCED_FLIP_MASK: u8 = 0x02;

/// Source world-space distance between adjacent terrain samples.
pub const TERRAIN_XY_SCALE: f32 = 10.0;
/// Source world-space height represented by one height byte.
pub const TERRAIN_HEIGHT_SCALE: f32 = TERRAIN_XY_SCALE / 16.0;

/// Explicit policy for source cliff texture-coordinate compatibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerrainCompatibilityPolicy {
    /// Apply stored cliff UVs and the legacy steep-slope adjustment.
    ZeroHourLegacy,
    /// Apply stored cliff UVs without implicit steep-slope retile adjustment.
    Modern,
}

/// Explicit deterministic terrain texture-bake inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerrainStagingOptions {
    pixels_per_cell: u32,
    compatibility: TerrainCompatibilityPolicy,
}

impl TerrainStagingOptions {
    /// Source background terrain uses eight pixels per cell.
    pub const SOURCE_BACKGROUND: Self = Self {
        pixels_per_cell: 8,
        compatibility: TerrainCompatibilityPolicy::ZeroHourLegacy,
    };

    /// Selects a source mip-compatible power-of-two cell resolution from 1 through 32.
    ///
    /// # Errors
    ///
    /// Returns [`TerrainError::InvalidPixelsPerCell`] for other values.
    pub const fn new(pixels_per_cell: u32) -> Result<Self, TerrainError> {
        if pixels_per_cell == 0 || pixels_per_cell > 32 || !pixels_per_cell.is_power_of_two() {
            return Err(TerrainError::InvalidPixelsPerCell(pixels_per_cell));
        }
        Ok(Self {
            pixels_per_cell,
            compatibility: TerrainCompatibilityPolicy::ZeroHourLegacy,
        })
    }

    #[must_use]
    pub const fn pixels_per_cell(self) -> u32 {
        self.pixels_per_cell
    }

    #[must_use]
    pub const fn compatibility(self) -> TerrainCompatibilityPolicy {
        self.compatibility
    }

    #[must_use]
    pub const fn with_compatibility(mut self, compatibility: TerrainCompatibilityPolicy) -> Self {
        self.compatibility = compatibility;
        self
    }
}

/// One optional alpha-blended terrain layer in source order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerrainLayer {
    table_index: u32,
    tile_index: i32,
    alpha_corners: [u8; 4],
    custom_edge_class: i32,
}

impl TerrainLayer {
    #[must_use]
    pub const fn table_index(self) -> u32 {
        self.table_index
    }

    #[must_use]
    pub const fn tile_index(self) -> i32 {
        self.tile_index
    }

    /// Returns source corner order: low/low, high/low, high/high, low/high.
    #[must_use]
    pub const fn alpha_corners(self) -> [u8; 4] {
        self.alpha_corners
    }

    /// Returns the custom edge class, or a negative source sentinel for procedural alpha.
    #[must_use]
    pub const fn custom_edge_class(self) -> i32 {
        self.custom_edge_class
    }
}

/// Renderer-side layer selection for one terrain cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerrainCell {
    x: u32,
    y: u32,
    base_tile: i16,
    primary: Option<TerrainLayer>,
    extra: Option<TerrainLayer>,
    cliff_info: i16,
    cliff: bool,
    flipped: bool,
}

impl TerrainCell {
    #[must_use]
    pub const fn x(self) -> u32 {
        self.x
    }

    #[must_use]
    pub const fn y(self) -> u32 {
        self.y
    }

    #[must_use]
    pub const fn base_tile(self) -> i16 {
        self.base_tile
    }

    #[must_use]
    pub const fn primary(self) -> Option<TerrainLayer> {
        self.primary
    }

    #[must_use]
    pub const fn extra(self) -> Option<TerrainLayer> {
        self.extra
    }

    #[must_use]
    pub const fn cliff_info(self) -> i16 {
        self.cliff_info
    }

    #[must_use]
    pub const fn is_cliff(self) -> bool {
        self.cliff
    }

    #[must_use]
    pub const fn is_flipped(self) -> bool {
        self.flipped
    }
}

/// One source-scaled terrain vertex.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerrainVertex {
    position: [f32; 3],
    uv: [f32; 2],
}

impl TerrainVertex {
    #[must_use]
    pub const fn position(self) -> [f32; 3] {
        self.position
    }

    #[must_use]
    pub const fn uv(self) -> [f32; 2] {
        self.uv
    }
}

/// Immutable terrain geometry, layers, and deterministic baked base texture.
#[derive(Debug, Clone, PartialEq)]
pub struct StagedTerrain {
    width: u32,
    height: u32,
    cells: Vec<TerrainCell>,
    vertices: Vec<TerrainVertex>,
    indices: Vec<u32>,
    edge_indices: Vec<u32>,
    texture_width: u32,
    texture_height: u32,
    texture_rgba: Vec<u8>,
    edge_texture_rgba: Vec<u8>,
    custom_edge_cell_count: usize,
    detail_source: TerrainDetailSource,
}

#[derive(Debug, Clone, PartialEq)]
struct TerrainDetailSource {
    height: MapHeightField,
    blend: MapBlendData,
    textures: TextureResourceManager,
    compatibility: TerrainCompatibilityPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TerrainRegion {
    min_x: usize,
    min_y: usize,
    max_x: usize,
    max_y: usize,
}

type BakedTerrainTextures = (u32, u32, Vec<u8>, Vec<u8>);

/// One bounded, viewport-derived terrain texture request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TerrainDetailRequest {
    min: [u32; 2],
    max: [u32; 2],
    visible_min: [u32; 2],
    visible_max: [u32; 2],
    pixels_per_cell: u32,
}

impl TerrainDetailRequest {
    #[cfg(test)]
    pub(crate) const fn for_test(min: [u32; 2], max: [u32; 2], density: u32) -> Self {
        Self {
            min,
            max,
            visible_min: min,
            visible_max: max,
            pixels_per_cell: density,
        }
    }

    #[cfg(test)]
    const fn region(self) -> TerrainRegion {
        TerrainRegion {
            min_x: self.min[0] as usize,
            min_y: self.min[1] as usize,
            max_x: self.max[0] as usize,
            max_y: self.max[1] as usize,
        }
    }

    #[cfg(test)]
    const fn width(self) -> u32 {
        self.max[0] - self.min[0]
    }

    #[cfg(test)]
    const fn height(self) -> u32 {
        self.max[1] - self.min[1]
    }

    #[cfg(test)]
    const fn pixels_per_cell(self) -> u32 {
        self.pixels_per_cell
    }

    pub(crate) const fn minimum(self) -> [u32; 2] {
        self.min
    }

    pub(crate) const fn maximum(self) -> [u32; 2] {
        self.max
    }

    pub(crate) const fn visible_minimum(self) -> [u32; 2] {
        self.visible_min
    }

    pub(crate) const fn visible_maximum(self) -> [u32; 2] {
        self.visible_max
    }

    pub(crate) const fn density(self) -> u32 {
        self.pixels_per_cell
    }

    #[cfg(test)]
    pub(crate) const fn covers(self, required: Self) -> bool {
        self.pixels_per_cell >= required.pixels_per_cell
            && self.min[0] <= required.visible_min[0]
            && self.min[1] <= required.visible_min[1]
            && self.max[0] >= required.visible_max[0]
            && self.max[1] >= required.visible_max[1]
    }
}

/// Compact, immutable semantic inputs for GPU terrain-page composition.
pub(crate) struct TerrainVirtualSource {
    cell_size: [u32; 2],
    source_tile_grid_width: u32,
    source_tile_atlas_rgba: Vec<u8>,
    edge_tile_grid_width: u32,
    edge_tile_atlas_rgba: Vec<u8>,
    macro_lattice_size: [u32; 2],
    macro_lattice: Vec<u8>,
    cell_bytes: Vec<u8>,
    modern: bool,
}

impl TerrainVirtualSource {
    pub(crate) const fn cell_size(&self) -> [u32; 2] {
        self.cell_size
    }

    pub(crate) const fn source_tile_grid_width(&self) -> u32 {
        self.source_tile_grid_width
    }

    pub(crate) fn source_tile_atlas_rgba(&self) -> &[u8] {
        &self.source_tile_atlas_rgba
    }

    pub(crate) const fn edge_tile_grid_width(&self) -> u32 {
        self.edge_tile_grid_width
    }

    pub(crate) fn edge_tile_atlas_rgba(&self) -> &[u8] {
        &self.edge_tile_atlas_rgba
    }

    pub(crate) const fn macro_lattice_size(&self) -> [u32; 2] {
        self.macro_lattice_size
    }

    pub(crate) fn macro_lattice(&self) -> &[u8] {
        &self.macro_lattice
    }

    pub(crate) fn cell_bytes(&self) -> &[u8] {
        &self.cell_bytes
    }

    pub(crate) const fn modern(&self) -> bool {
        self.modern
    }
}

impl TerrainRegion {
    const fn width(self) -> usize {
        self.max_x - self.min_x
    }

    const fn height(self) -> usize {
        self.max_y - self.min_y
    }
}

const fn quantize_up(value: u32, quantum: u32) -> u32 {
    value.saturating_add(quantum - 1) / quantum * quantum
}

#[cfg(test)]
fn detail_fits(size: [u32; 2], pixels_per_cell: u32) -> bool {
    let Some(width) = size[0].checked_mul(pixels_per_cell) else {
        return false;
    };
    let Some(height) = size[1].checked_mul(pixels_per_cell) else {
        return false;
    };
    let Some(bytes) = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|texels| texels.checked_mul(4))
    else {
        return false;
    };
    let (Ok(width), Ok(height), Ok(bytes)) = (
        usize::try_from(width),
        usize::try_from(height),
        usize::try_from(bytes),
    ) else {
        return false;
    };
    width <= MAX_BAKED_TEXTURE_DIMENSION
        && height <= MAX_BAKED_TEXTURE_DIMENSION
        && bytes <= MAX_BAKED_TEXTURE_BYTES
}

#[cfg(test)]
fn select_detail_pixels(size: [u32; 2], visible_size: [u32; 2], viewport: [u32; 2]) -> Option<u32> {
    let desired_x = u64::from(viewport[0])
        .checked_mul(3)?
        .div_ceil(u64::from(visible_size[0]).checked_mul(2)?);
    let desired_y = u64::from(viewport[1])
        .checked_mul(3)?
        .div_ceil(u64::from(visible_size[1]).checked_mul(2)?);
    let desired = u32::try_from(desired_x.max(desired_y).min(32))
        .expect("detail density is clamped to u32")
        .next_power_of_two()
        .clamp(8, 32);
    [32, 16, 8]
        .into_iter()
        .find(|pixels| *pixels <= desired && detail_fits(size, *pixels))
        .or_else(|| {
            [8, 4, 2, 1]
                .into_iter()
                .find(|pixels| detail_fits(size, *pixels))
        })
}

#[cfg(test)]
pub(crate) struct TerrainDetailPatch {
    indices: Vec<u32>,
    edge_indices: Vec<u32>,
    texture_width: u32,
    texture_height: u32,
    texture_rgba: Vec<u8>,
    edge_texture_rgba: Vec<u8>,
}

pub(crate) struct TerrainMipLevel {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) rgba: Vec<u8>,
}

#[cfg(test)]
impl TerrainDetailPatch {
    pub(crate) const fn texture_width(&self) -> u32 {
        self.texture_width
    }

    pub(crate) const fn texture_height(&self) -> u32 {
        self.texture_height
    }

    pub(crate) fn texture_rgba(&self) -> &[u8] {
        &self.texture_rgba
    }

    pub(crate) fn edge_texture_rgba(&self) -> &[u8] {
        &self.edge_texture_rgba
    }

    pub(crate) fn index_count(&self) -> Result<u32, TerrainError> {
        u32::try_from(self.indices.len()).map_err(|_| TerrainError::TerrainTooLarge)
    }

    pub(crate) fn edge_index_count(&self) -> Result<u32, TerrainError> {
        u32::try_from(self.edge_indices.len()).map_err(|_| TerrainError::TerrainTooLarge)
    }
}

impl StagedTerrain {
    /// Stages source-scaled geometry and the base, primary, then extra texture layers.
    ///
    /// Custom edge classes remain visible in [`TerrainCell`] but are not composited because the
    /// source renders them as separate geometry. Their count is exposed for explicit diagnostics.
    ///
    /// # Errors
    ///
    /// Returns [`TerrainError`] for inconsistent dimensions or indices, missing/malformed texture
    /// sheets, unsupported bake resolution, or bounded geometry/texture allocation excess.
    pub fn from_map(
        height: &MapHeightField,
        blend: &MapBlendData,
        textures: &TextureResourceManager,
        options: TerrainStagingOptions,
    ) -> Result<Self, TerrainError> {
        if (height.width(), height.height()) != (blend.width(), blend.height()) {
            return Err(TerrainError::DimensionMismatch);
        }
        let width = usize::try_from(height.width()).map_err(|_| TerrainError::TerrainTooLarge)?;
        let height_count =
            usize::try_from(height.height()).map_err(|_| TerrainError::TerrainTooLarge)?;
        if width < 2 || height_count < 2 {
            return Err(TerrainError::EmptyTerrain);
        }
        let vertex_count = width
            .checked_mul(height_count)
            .ok_or(TerrainError::TerrainTooLarge)?;
        if vertex_count > MAX_TERRAIN_VERTICES || vertex_count > u32::MAX as usize {
            return Err(TerrainError::TerrainTooLarge);
        }

        let cells = stage_cells(height, blend, options, width, height_count)?;
        let (vertices, indices) = stage_geometry(height, &cells, width, height_count)?;
        let edge_indices = stage_edge_indices(&cells, width)?;
        let custom_edge_cell_count = edge_indices.len() / 6;
        let (texture_width, texture_height, texture_rgba, edge_texture_rgba) = bake_texture(
            height,
            blend,
            textures,
            options,
            &cells,
            width,
            height_count,
        )?;
        Ok(Self {
            width: height.width(),
            height: height.height(),
            cells,
            vertices,
            indices,
            edge_indices,
            texture_width,
            texture_height,
            texture_rgba,
            edge_texture_rgba,
            custom_edge_cell_count,
            detail_source: TerrainDetailSource {
                height: height.clone(),
                blend: blend.clone(),
                textures: textures.clone(),
                compatibility: options.compatibility,
            },
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
    pub fn cells(&self) -> &[TerrainCell] {
        &self.cells
    }

    #[must_use]
    pub fn vertices(&self) -> &[TerrainVertex] {
        &self.vertices
    }

    #[must_use]
    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    #[must_use]
    pub fn edge_indices(&self) -> &[u32] {
        &self.edge_indices
    }

    #[must_use]
    pub const fn texture_width(&self) -> u32 {
        self.texture_width
    }

    #[must_use]
    pub const fn texture_height(&self) -> u32 {
        self.texture_height
    }

    #[must_use]
    pub fn texture_rgba(&self) -> &[u8] {
        &self.texture_rgba
    }

    #[must_use]
    pub fn edge_texture_rgba(&self) -> &[u8] {
        &self.edge_texture_rgba
    }

    #[must_use]
    pub const fn custom_edge_cell_count(&self) -> usize {
        self.custom_edge_cell_count
    }

    pub(crate) fn virtual_source(&self) -> Result<TerrainVirtualSource, TerrainError> {
        build_virtual_source(self)
    }

    #[cfg(test)]
    pub(crate) fn detail_request(
        &self,
        minimum_world: [f32; 2],
        maximum_world: [f32; 2],
        viewport: [u32; 2],
    ) -> Result<TerrainDetailRequest, TerrainError> {
        let cell_width = self
            .width
            .checked_sub(1)
            .ok_or(TerrainError::EmptyTerrain)?;
        let cell_height = self
            .height
            .checked_sub(1)
            .ok_or(TerrainError::EmptyTerrain)?;
        if !minimum_world
            .into_iter()
            .chain(maximum_world)
            .all(f32::is_finite)
            || viewport[0] == 0
            || viewport[1] == 0
        {
            return Err(TerrainError::InvalidDetailViewport);
        }
        let (visible_min, visible_max) =
            self.detail_visible_cells(minimum_world, maximum_world, cell_width, cell_height);
        let visible_size = [
            visible_max[0].saturating_sub(visible_min[0]).max(1),
            visible_max[1].saturating_sub(visible_min[1]).max(1),
        ];
        let pixels_per_cell = select_detail_pixels(visible_size, visible_size, viewport)
            .ok_or(TerrainError::TerrainTooLarge)?;
        self.detail_request_for_visible(visible_min, visible_max, pixels_per_cell)
    }

    pub(crate) fn detail_request_at_density(
        &self,
        minimum_world: [f32; 2],
        maximum_world: [f32; 2],
        pixels_per_cell: u32,
    ) -> Result<TerrainDetailRequest, TerrainError> {
        let cell_width = self
            .width
            .checked_sub(1)
            .ok_or(TerrainError::EmptyTerrain)?;
        let cell_height = self
            .height
            .checked_sub(1)
            .ok_or(TerrainError::EmptyTerrain)?;
        if !minimum_world
            .into_iter()
            .chain(maximum_world)
            .all(f32::is_finite)
            || !matches!(pixels_per_cell, 8 | 16 | 32)
        {
            return Err(TerrainError::InvalidDetailViewport);
        }
        let (visible_min, visible_max) =
            self.detail_visible_cells(minimum_world, maximum_world, cell_width, cell_height);
        self.virtual_request_for_visible(visible_min, visible_max, pixels_per_cell)
    }

    fn virtual_request_for_visible(
        &self,
        visible_min: [u32; 2],
        visible_max: [u32; 2],
        pixels_per_cell: u32,
    ) -> Result<TerrainDetailRequest, TerrainError> {
        let cell_width = self
            .width
            .checked_sub(1)
            .ok_or(TerrainError::EmptyTerrain)?;
        let cell_height = self
            .height
            .checked_sub(1)
            .ok_or(TerrainError::EmptyTerrain)?;
        let visible_size = [
            visible_max[0].saturating_sub(visible_min[0]).max(1),
            visible_max[1].saturating_sub(visible_min[1]).max(1),
        ];
        let margin = [
            (visible_size[0] / 2).max(DETAIL_MIN_MARGIN_CELLS),
            (visible_size[1] / 2).max(DETAIL_MIN_MARGIN_CELLS),
        ];
        let min = [
            visible_min[0].saturating_sub(margin[0]) / DETAIL_REGION_QUANTUM_CELLS
                * DETAIL_REGION_QUANTUM_CELLS,
            visible_min[1].saturating_sub(margin[1]) / DETAIL_REGION_QUANTUM_CELLS
                * DETAIL_REGION_QUANTUM_CELLS,
        ];
        let max = [
            quantize_up(
                visible_max[0].saturating_add(margin[0]),
                DETAIL_REGION_QUANTUM_CELLS,
            )
            .min(cell_width),
            quantize_up(
                visible_max[1].saturating_add(margin[1]),
                DETAIL_REGION_QUANTUM_CELLS,
            )
            .min(cell_height),
        ];
        Ok(TerrainDetailRequest {
            min,
            max,
            visible_min,
            visible_max,
            pixels_per_cell,
        })
    }

    fn detail_visible_cells(
        &self,
        minimum_world: [f32; 2],
        maximum_world: [f32; 2],
        cell_width: u32,
        cell_height: u32,
    ) -> ([u32; 2], [u32; 2]) {
        let minimum = self.cell_for_world(minimum_world);
        let maximum = self.cell_for_world(maximum_world);
        (
            [minimum[0].min(maximum[0]), minimum[1].min(maximum[1])],
            [
                minimum[0].max(maximum[0]).saturating_add(1).min(cell_width),
                minimum[1]
                    .max(maximum[1])
                    .saturating_add(1)
                    .min(cell_height),
            ],
        )
    }

    #[cfg(test)]
    fn detail_request_for_visible(
        &self,
        visible_min: [u32; 2],
        visible_max: [u32; 2],
        pixels_per_cell: u32,
    ) -> Result<TerrainDetailRequest, TerrainError> {
        let cell_width = self
            .width
            .checked_sub(1)
            .ok_or(TerrainError::EmptyTerrain)?;
        let cell_height = self
            .height
            .checked_sub(1)
            .ok_or(TerrainError::EmptyTerrain)?;
        let visible_size = [
            visible_max[0].saturating_sub(visible_min[0]).max(1),
            visible_max[1].saturating_sub(visible_min[1]).max(1),
        ];
        let margin = [
            (visible_size[0] / 2).max(DETAIL_MIN_MARGIN_CELLS),
            (visible_size[1] / 2).max(DETAIL_MIN_MARGIN_CELLS),
        ];
        let mut min = [
            visible_min[0].saturating_sub(margin[0]) / DETAIL_REGION_QUANTUM_CELLS
                * DETAIL_REGION_QUANTUM_CELLS,
            visible_min[1].saturating_sub(margin[1]) / DETAIL_REGION_QUANTUM_CELLS
                * DETAIL_REGION_QUANTUM_CELLS,
        ];
        let mut max = [
            quantize_up(
                visible_max[0].saturating_add(margin[0]),
                DETAIL_REGION_QUANTUM_CELLS,
            )
            .min(cell_width),
            quantize_up(
                visible_max[1].saturating_add(margin[1]),
                DETAIL_REGION_QUANTUM_CELLS,
            )
            .min(cell_height),
        ];
        while !detail_fits([max[0] - min[0], max[1] - min[1]], pixels_per_cell) {
            let margins = [
                visible_min[0] - min[0],
                max[0] - visible_max[0],
                visible_min[1] - min[1],
                max[1] - visible_max[1],
            ];
            let Some((largest, _)) = margins
                .into_iter()
                .enumerate()
                .filter(|(_, margin)| *margin > 0)
                .max_by_key(|(index, margin)| (*margin, std::cmp::Reverse(*index)))
            else {
                return Err(TerrainError::TerrainTooLarge);
            };
            match largest {
                0 => min[0] += DETAIL_REGION_QUANTUM_CELLS.min(margins[0]),
                1 => max[0] -= DETAIL_REGION_QUANTUM_CELLS.min(margins[1]),
                2 => min[1] += DETAIL_REGION_QUANTUM_CELLS.min(margins[2]),
                3 => max[1] -= DETAIL_REGION_QUANTUM_CELLS.min(margins[3]),
                _ => unreachable!("detail margin axis is bounded"),
            }
        }
        Ok(TerrainDetailRequest {
            min,
            max,
            visible_min,
            visible_max,
            pixels_per_cell,
        })
    }

    #[allow(clippy::cast_precision_loss)]
    #[cfg(test)]
    pub(crate) fn detail_patch(
        &self,
        request: TerrainDetailRequest,
    ) -> Result<TerrainDetailPatch, TerrainError> {
        self.detail_patch_controlled(request, || false)
            .map(|patch| patch.expect("an uncancelled terrain bake completes"))
    }

    #[allow(clippy::cast_precision_loss)]
    #[cfg(test)]
    pub(crate) fn detail_patch_controlled<F>(
        &self,
        request: TerrainDetailRequest,
        cancelled: F,
    ) -> Result<Option<TerrainDetailPatch>, TerrainError>
    where
        F: Fn() -> bool,
    {
        let cell_width = self
            .width
            .checked_sub(1)
            .ok_or(TerrainError::EmptyTerrain)?;
        let cell_height = self
            .height
            .checked_sub(1)
            .ok_or(TerrainError::EmptyTerrain)?;
        if request.min[0] >= request.max[0]
            || request.min[1] >= request.max[1]
            || request.max[0] > cell_width
            || request.max[1] > cell_height
            || !detail_fits(
                [request.width(), request.height()],
                request.pixels_per_cell(),
            )
        {
            return Err(TerrainError::InvalidDetailViewport);
        }
        let region = request.region();
        let options = TerrainStagingOptions::new(request.pixels_per_cell())?
            .with_compatibility(self.detail_source.compatibility);
        let width = usize::try_from(self.width).map_err(|_| TerrainError::TerrainTooLarge)?;
        let height = usize::try_from(self.height).map_err(|_| TerrainError::TerrainTooLarge)?;
        let Some((texture_width, texture_height, texture_rgba, edge_texture_rgba)) =
            bake_texture_region_controlled(
                &self.detail_source.height,
                &self.detail_source.blend,
                &self.detail_source.textures,
                options,
                &self.cells,
                width,
                height,
                region,
                &cancelled,
            )?
        else {
            return Ok(None);
        };
        if cancelled() {
            return Ok(None);
        }
        let (_, indices, edge_indices) = self.detail_geometry(region)?;
        if cancelled() {
            return Ok(None);
        }
        Ok(Some(TerrainDetailPatch {
            indices,
            edge_indices,
            texture_width,
            texture_height,
            texture_rgba,
            edge_texture_rgba,
        }))
    }

    #[allow(clippy::cast_precision_loss)]
    pub(crate) fn cell_for_world(&self, world: [f32; 2]) -> [u32; 2] {
        let border = self.detail_source.height.border_size() as f32;
        let maximum_x = self.width.saturating_sub(2);
        let maximum_y = self.height.saturating_sub(2);
        let x = (world[0] / TERRAIN_XY_SCALE + border)
            .floor()
            .clamp(0.0, maximum_x as f32);
        let y = (world[1] / TERRAIN_XY_SCALE + border)
            .floor()
            .clamp(0.0, maximum_y as f32);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        [x as u32, y as u32]
    }

    #[allow(clippy::type_complexity)]
    #[cfg(test)]
    fn detail_geometry(
        &self,
        region: TerrainRegion,
    ) -> Result<(Vec<TerrainVertex>, Vec<u32>, Vec<u32>), TerrainError> {
        let source_width =
            usize::try_from(self.width).map_err(|_| TerrainError::TerrainTooLarge)?;
        let cell_stride = source_width
            .checked_sub(1)
            .ok_or(TerrainError::EmptyTerrain)?;
        let patch_width = region.width();
        let patch_height = region.height();
        let vertex_width = patch_width
            .checked_add(1)
            .ok_or(TerrainError::TerrainTooLarge)?;
        let mut vertices = Vec::with_capacity(
            vertex_width
                .checked_mul(patch_height + 1)
                .ok_or(TerrainError::TerrainTooLarge)?,
        );
        #[allow(clippy::cast_precision_loss)]
        for y in region.min_y..=region.max_y {
            for x in region.min_x..=region.max_x {
                let source = self
                    .vertices
                    .get(y * source_width + x)
                    .ok_or(TerrainError::DimensionMismatch)?;
                vertices.push(TerrainVertex {
                    position: source.position,
                    uv: [
                        (x - region.min_x) as f32 / patch_width as f32,
                        1.0 - (y - region.min_y) as f32 / patch_height as f32,
                    ],
                });
            }
        }
        let mut indices = Vec::with_capacity(patch_width * patch_height * 6);
        let mut edge_indices = Vec::new();
        for y in region.min_y..region.max_y {
            for x in region.min_x..region.max_x {
                let cell = *self
                    .cells
                    .get(y * cell_stride + x)
                    .ok_or(TerrainError::DimensionMismatch)?;
                let local_x = x - region.min_x;
                let local_y = y - region.min_y;
                let p0 = local_y * vertex_width + local_x;
                let p1 = p0 + 1;
                let p3 = p0 + vertex_width;
                let p2 = p3 + 1;
                let corners = [p0, p1, p2, p3]
                    .map(|index| u32::try_from(index).map_err(|_| TerrainError::TerrainTooLarge))
                    .into_iter()
                    .collect::<Result<Vec<_>, _>>()?;
                let triangles = if cell.flipped {
                    [
                        corners[1], corners[3], corners[0], corners[1], corners[2], corners[3],
                    ]
                } else {
                    [
                        corners[0], corners[2], corners[3], corners[0], corners[1], corners[2],
                    ]
                };
                indices.extend_from_slice(&triangles);
                if cell
                    .primary
                    .is_some_and(|layer| layer.custom_edge_class >= 0)
                {
                    edge_indices.extend_from_slice(&triangles);
                }
            }
        }
        Ok((vertices, indices, edge_indices))
    }

    pub(crate) fn projected_vertex_bytes(&self, aspect: f32) -> Vec<u8> {
        let mut projected = Vec::with_capacity(self.vertices.len());
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for vertex in &self.vertices {
            let [x, y, z] = vertex.position;
            let value = [
                (x - y) * std::f32::consts::FRAC_1_SQRT_2,
                z * 0.816_496_6 - (x + y) * 0.408_248_3,
                (x + y - z) * 0.577_350_26,
            ];
            for axis in 0..3 {
                min[axis] = min[axis].min(value[axis]);
                max[axis] = max[axis].max(value[axis]);
            }
            projected.push(value);
        }
        let center_x = (min[0] + max[0]) * 0.5;
        let center_y = (min[1] + max[1]) * 0.5;
        let range_x = (max[0] - min[0]).max(f32::EPSILON);
        let range_y = (max[1] - min[1]).max(f32::EPSILON);
        let scale = 1.8 / range_y.max(range_x / aspect);
        let depth_range = (max[2] - min[2]).max(f32::EPSILON);
        let mut bytes = Vec::with_capacity(self.vertices.len() * 20);
        for (vertex, projected) in self.vertices.iter().zip(projected) {
            let clip = [
                (projected[0] - center_x) * scale / aspect,
                (projected[1] - center_y) * scale,
                0.05 + 0.9 * (projected[2] - min[2]) / depth_range,
            ];
            for value in clip.into_iter().chain(vertex.uv) {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        bytes
    }

    pub(crate) fn index_bytes(&self) -> Vec<u8> {
        index_bytes(&self.indices)
    }

    pub(crate) fn edge_index_bytes(&self) -> Vec<u8> {
        index_bytes(&self.edge_indices)
    }

    pub(crate) fn viewer_vertex_bytes(&self) -> Result<Vec<u8>, TerrainError> {
        terrain_viewer_vertex_bytes(
            &self.vertices,
            usize::try_from(self.width).map_err(|_| TerrainError::TerrainTooLarge)?,
            usize::try_from(self.height).map_err(|_| TerrainError::TerrainTooLarge)?,
        )
    }

    pub(crate) fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        let mut minimum = [f32::INFINITY; 3];
        let mut maximum = [f32::NEG_INFINITY; 3];
        for vertex in &self.vertices {
            for axis in 0..3 {
                minimum[axis] = minimum[axis].min(vertex.position[axis]);
                maximum[axis] = maximum[axis].max(vertex.position[axis]);
            }
        }
        (minimum, maximum)
    }
}

#[derive(Clone, Copy)]
struct VirtualMaterial {
    parameters: [u32; 4],
    u: [f32; 4],
    v: [f32; 4],
}

impl VirtualMaterial {
    const INVALID: Self = Self {
        parameters: [0; 4],
        u: [0.0; 4],
        v: [0.0; 4],
    };

    fn write_bytes(self, bytes: &mut Vec<u8>) {
        for value in self.parameters {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for value in self.u.into_iter().chain(self.v) {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
}

#[allow(clippy::too_many_lines)]
fn build_virtual_source(terrain: &StagedTerrain) -> Result<TerrainVirtualSource, TerrainError> {
    let blend = &terrain.detail_source.blend;
    let textures = &terrain.detail_source.textures;
    let mut class_starts = Vec::with_capacity(blend.texture_classes().len());
    let mut source_tiles = Vec::new();
    for class in blend.texture_classes() {
        class_starts
            .push(u32::try_from(source_tiles.len()).map_err(|_| TerrainError::TerrainTooLarge)?);
        let width = usize::try_from(class.width()).map_err(|_| TerrainError::TerrainTooLarge)?;
        let tile_count = width
            .checked_mul(width)
            .ok_or(TerrainError::TerrainTooLarge)?;
        if source_tiles.len().saturating_add(tile_count) > MAX_VIRTUAL_SOURCE_TILES {
            return Err(TerrainError::TerrainTooLarge);
        }
        let image = textures
            .image(class.name_bytes())
            .ok_or_else(|| TerrainError::MissingTexture(class.name_bytes().to_vec()))?;
        let required = width
            .checked_mul(SOURCE_TILE_PIXELS)
            .ok_or(TerrainError::TerrainTooLarge)?;
        let image_width =
            usize::try_from(image.width()).map_err(|_| TerrainError::TerrainTooLarge)?;
        let image_height =
            usize::try_from(image.height()).map_err(|_| TerrainError::TerrainTooLarge)?;
        if image_width < required || image_height < required {
            return Err(TerrainError::InvalidTextureSheet(
                class.name_bytes().to_vec(),
            ));
        }
        for row_from_bottom in 0..width {
            for x in 0..width {
                source_tiles.push(copy_region(
                    image.rgba(),
                    image_width,
                    x * SOURCE_TILE_PIXELS,
                    image_height - (row_from_bottom + 1) * SOURCE_TILE_PIXELS,
                    SOURCE_TILE_PIXELS,
                )?);
            }
        }
    }
    if source_tiles.is_empty() {
        return Err(TerrainError::TerrainTooLarge);
    }
    let (source_tile_grid_width, source_tile_atlas_rgba) =
        pack_square_tiles(&source_tiles, SOURCE_TILE_PIXELS)?;

    let pixels = 32_usize;
    let mut needed_tiles = BTreeSet::new();
    for cell in &terrain.cells {
        needed_tiles.insert(i32::from(cell.base_tile));
        for layer in [cell.primary, cell.extra].into_iter().flatten() {
            needed_tiles.insert(layer.tile_index);
        }
    }
    let mut tile_cache = BTreeMap::new();
    for tile in needed_tiles {
        tile_cache.insert(tile, extract_cell_tile(tile, pixels, blend, textures)?);
    }
    let transparent_edge = vec![0_u8; pixels * pixels * 4];
    let mut edge_tiles = vec![transparent_edge.clone()];
    let mut edge_lookup = BTreeMap::from([(transparent_edge, 0_u32)]);
    let width = usize::try_from(terrain.width).map_err(|_| TerrainError::TerrainTooLarge)?;
    let mut edge_indices = Vec::with_capacity(terrain.cells.len());
    for cell in &terrain.cells {
        let Some(layer) = cell.primary.filter(|layer| layer.custom_edge_class >= 0) else {
            edge_indices.push(0);
            continue;
        };
        let blend_tile = cell_tile(
            layer.tile_index,
            *cell,
            &terrain.detail_source.height,
            width,
            pixels,
            blend,
            textures,
            terrain.detail_source.compatibility,
            &tile_cache,
        )?;
        let edge = extract_custom_edge(layer, *cell, pixels, blend, textures, &blend_tile)?;
        let index = if let Some(index) = edge_lookup.get(&edge) {
            *index
        } else {
            if edge_tiles.len() >= MAX_VIRTUAL_SOURCE_TILES {
                return Err(TerrainError::TerrainTooLarge);
            }
            let index =
                u32::try_from(edge_tiles.len()).map_err(|_| TerrainError::TerrainTooLarge)?;
            edge_lookup.insert(edge.clone(), index);
            edge_tiles.push(edge);
            index
        };
        edge_indices.push(index);
    }
    let (edge_tile_grid_width, edge_tile_atlas_rgba) = pack_square_tiles(&edge_tiles, pixels)?;

    let cell_byte_count = terrain
        .cells
        .len()
        .checked_mul(VIRTUAL_CELL_BYTES)
        .filter(|size| *size <= MAX_VIRTUAL_CELL_BUFFER_BYTES)
        .ok_or(TerrainError::TerrainTooLarge)?;
    let mut cell_bytes = Vec::with_capacity(cell_byte_count);
    for (cell, edge_index) in terrain.cells.iter().zip(edge_indices) {
        virtual_material(terrain, *cell, i32::from(cell.base_tile), &class_starts)?
            .write_bytes(&mut cell_bytes);
        let primary = cell
            .primary
            .filter(|layer| layer.custom_edge_class < 0)
            .map_or(Ok(VirtualMaterial::INVALID), |layer| {
                virtual_material(terrain, *cell, layer.tile_index, &class_starts)
            })?;
        primary.write_bytes(&mut cell_bytes);
        let extra = cell.extra.map_or(Ok(VirtualMaterial::INVALID), |layer| {
            virtual_material(terrain, *cell, layer.tile_index, &class_starts)
        })?;
        extra.write_bytes(&mut cell_bytes);
        let primary_mask = cell
            .primary
            .filter(|layer| layer.custom_edge_class < 0)
            .map_or(0, |layer| virtual_mask_code(layer, blend));
        let extra_mask = cell
            .extra
            .map_or(0, |layer| virtual_mask_code(layer, blend));
        for value in [primary_mask, extra_mask, edge_index, 0] {
            cell_bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    debug_assert_eq!(cell_bytes.len(), terrain.cells.len() * VIRTUAL_CELL_BYTES);
    let cell_size = [terrain.width - 1, terrain.height - 1];
    let macro_lattice_size = [cell_size[0].div_ceil(8) + 1, cell_size[1].div_ceil(8) + 1];
    let mut macro_lattice = Vec::with_capacity(
        usize::try_from(u64::from(macro_lattice_size[0]) * u64::from(macro_lattice_size[1]))
            .map_err(|_| TerrainError::TerrainTooLarge)?,
    );
    for y in 0..macro_lattice_size[1] {
        for x in 0..macro_lattice_size[0] {
            macro_lattice.push(
                u8::try_from(macro_hash(x as usize, y as usize))
                    .map_err(|_| TerrainError::TerrainTooLarge)?,
            );
        }
    }
    Ok(TerrainVirtualSource {
        cell_size,
        source_tile_grid_width,
        source_tile_atlas_rgba,
        edge_tile_grid_width,
        edge_tile_atlas_rgba,
        macro_lattice_size,
        macro_lattice,
        cell_bytes,
        modern: terrain.detail_source.compatibility == TerrainCompatibilityPolicy::Modern,
    })
}

fn virtual_material(
    terrain: &StagedTerrain,
    cell: TerrainCell,
    packed_index: i32,
    class_starts: &[u32],
) -> Result<VirtualMaterial, TerrainError> {
    let (_, class) = terrain_class(packed_index, &terrain.detail_source.blend)?;
    let class_index = terrain
        .detail_source
        .blend
        .texture_classes()
        .iter()
        .position(|candidate| std::ptr::eq(candidate, class))
        .ok_or(TerrainError::UnclassifiedTile(
            u32::try_from(packed_index).unwrap_or_default() >> 2,
        ))?;
    let corners = adjusted_uv_corners(
        packed_index,
        cell,
        &terrain.detail_source.height,
        usize::try_from(terrain.width).map_err(|_| TerrainError::TerrainTooLarge)?,
        &terrain.detail_source.blend,
        terrain.detail_source.compatibility,
    )?
    .map_or_else(
        || default_class_uv_corners(packed_index, &terrain.detail_source.blend),
        Ok,
    )?;
    Ok(VirtualMaterial {
        parameters: [class_starts[class_index], class.width(), 1, 0],
        u: corners.map(|corner| corner[0]),
        v: corners.map(|corner| corner[1]),
    })
}

fn virtual_mask_code(layer: TerrainLayer, blend: &MapBlendData) -> u32 {
    let Ok(offset) = usize::try_from(layer.table_index.saturating_sub(1)) else {
        return 0;
    };
    let Some(record) = blend.blend_tiles().get(offset) else {
        return 0;
    };
    let orientation = if record.horizontal() != 0 {
        0
    } else if record.vertical() != 0 {
        1
    } else if record.right_diagonal() != 0 {
        2
    } else if record.left_diagonal() != 0 {
        3
    } else {
        return 0;
    };
    1 | (orientation << 1)
        | (u32::from(record.inverted() & INVERTED_MASK != 0) << 3)
        | (u32::from(record.long_diagonal() != 0) << 4)
}

fn pack_square_tiles(tiles: &[Vec<u8>], tile_size: usize) -> Result<(u32, Vec<u8>), TerrainError> {
    let mut grid = 1_usize;
    while grid
        .checked_mul(grid)
        .ok_or(TerrainError::TerrainTooLarge)?
        < tiles.len()
    {
        grid += 1;
    }
    let extent = grid
        .checked_mul(tile_size)
        .ok_or(TerrainError::TerrainTooLarge)?;
    if extent > MAX_BAKED_TEXTURE_DIMENSION {
        return Err(TerrainError::TerrainTooLarge);
    }
    let byte_count = extent
        .checked_mul(extent)
        .and_then(|count| count.checked_mul(4))
        .ok_or(TerrainError::TerrainTooLarge)?;
    let mut atlas = vec![0_u8; byte_count];
    let expected_tile_bytes = tile_size
        .checked_mul(tile_size)
        .and_then(|count| count.checked_mul(4))
        .ok_or(TerrainError::TerrainTooLarge)?;
    for (index, tile) in tiles.iter().enumerate() {
        if tile.len() != expected_tile_bytes {
            return Err(TerrainError::DimensionMismatch);
        }
        let origin_x = (index % grid) * tile_size;
        let origin_y = (index / grid) * tile_size;
        for row in 0..tile_size {
            let source = row * tile_size * 4;
            let target = ((origin_y + row) * extent + origin_x) * 4;
            atlas[target..target + tile_size * 4]
                .copy_from_slice(&tile[source..source + tile_size * 4]);
        }
    }
    Ok((
        u32::try_from(grid).map_err(|_| TerrainError::TerrainTooLarge)?,
        atlas,
    ))
}

fn terrain_viewer_vertex_bytes(
    vertices: &[TerrainVertex],
    width: usize,
    height: usize,
) -> Result<Vec<u8>, TerrainError> {
    let expected = width
        .checked_mul(height)
        .ok_or(TerrainError::TerrainTooLarge)?;
    if width < 2 || height < 2 || vertices.len() != expected {
        return Err(TerrainError::DimensionMismatch);
    }
    let byte_capacity = expected
        .checked_mul(32)
        .ok_or(TerrainError::TerrainTooLarge)?;
    let mut bytes = Vec::with_capacity(byte_capacity);
    for y in 0..height {
        for x in 0..width {
            let vertex_index = y
                .checked_mul(width)
                .and_then(|row| row.checked_add(x))
                .ok_or(TerrainError::TerrainTooLarge)?;
            let vertex = vertices
                .get(vertex_index)
                .ok_or(TerrainError::DimensionMismatch)?;
            let left_x = x.saturating_sub(1);
            let right_x = x.saturating_add(1).min(width - 1);
            let down_y = y.saturating_sub(1);
            let up_y = y.saturating_add(1).min(height - 1);
            let position = |sample_x: usize, sample_y: usize| {
                sample_y
                    .checked_mul(width)
                    .and_then(|row| row.checked_add(sample_x))
                    .and_then(|index| vertices.get(index))
                    .map(|sample| sample.position)
                    .ok_or(TerrainError::DimensionMismatch)
            };
            let left = position(left_x, y)?;
            let right = position(right_x, y)?;
            let down = position(x, down_y)?;
            let up = position(x, up_y)?;
            let tangent_x = [right[0] - left[0], right[1] - left[1], right[2] - left[2]];
            let tangent_y = [up[0] - down[0], up[1] - down[1], up[2] - down[2]];
            let mut normal = [
                tangent_x[1] * tangent_y[2] - tangent_x[2] * tangent_y[1],
                tangent_x[2] * tangent_y[0] - tangent_x[0] * tangent_y[2],
                tangent_x[0] * tangent_y[1] - tangent_x[1] * tangent_y[0],
            ];
            let length_squared = normal.iter().map(|value| value * value).sum::<f32>();
            if length_squared.is_finite() && length_squared > f32::EPSILON {
                let inverse_length = length_squared.sqrt().recip();
                normal = normal.map(|value| value * inverse_length);
            } else {
                normal = [0.0, 0.0, 1.0];
            }
            for value in vertex.position.into_iter().chain(vertex.uv).chain(normal) {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
    }
    Ok(bytes)
}

fn index_bytes(indices: &[u32]) -> Vec<u8> {
    indices
        .iter()
        .flat_map(|index| index.to_le_bytes())
        .collect()
}

fn stage_cells(
    height_field: &MapHeightField,
    blend: &MapBlendData,
    options: TerrainStagingOptions,
    width: usize,
    height: usize,
) -> Result<Vec<TerrainCell>, TerrainError> {
    let capacity = width
        .checked_sub(1)
        .and_then(|width| width.checked_mul(height - 1))
        .ok_or(TerrainError::TerrainTooLarge)?;
    let mut cells = Vec::with_capacity(capacity);
    for y in 0..height - 1 {
        for x in 0..width - 1 {
            let index = y
                .checked_mul(width)
                .and_then(|row| row.checked_add(x))
                .ok_or(TerrainError::TerrainTooLarge)?;
            let base_tile = *blend
                .tile_indices()
                .get(index)
                .ok_or(TerrainError::DimensionMismatch)?;
            if base_tile < 0 {
                return Err(TerrainError::InvalidTileIndex(i32::from(base_tile)));
            }
            let primary = layer_for_index(
                *blend
                    .blend_indices()
                    .get(index)
                    .ok_or(TerrainError::DimensionMismatch)?,
                blend,
            )?;
            let extra = layer_for_index(
                *blend
                    .extra_blend_indices()
                    .get(index)
                    .ok_or(TerrainError::DimensionMismatch)?,
                blend,
            )?;
            let cliff_info = *blend
                .cliff_info_indices()
                .get(index)
                .ok_or(TerrainError::DimensionMismatch)?;
            let cliff_flip = cliff_flip(cliff_info, blend)?;
            let x_u32 = u32::try_from(x).map_err(|_| TerrainError::TerrainTooLarge)?;
            let y_u32 = u32::try_from(y).map_err(|_| TerrainError::TerrainTooLarge)?;
            let cliff = blend
                .is_cliff(x_u32, y_u32)
                .ok_or(TerrainError::DimensionMismatch)?;
            let blend_flip = primary.is_some_and(|layer| layer_needs_flip(layer, blend));
            let legacy_adjusts = options.compatibility
                == TerrainCompatibilityPolicy::ZeroHourLegacy
                && cliff_info == 0
                && legacy_uv_adjusts(height_field, x, y, width)?;
            let flipped = if cliff_flip || legacy_adjusts {
                height_diagonal_flip(height_field, x, y, width)?
            } else {
                blend_flip
            };
            cells.push(TerrainCell {
                x: x_u32,
                y: y_u32,
                base_tile,
                primary,
                extra,
                cliff_info,
                cliff,
                flipped,
            });
        }
    }
    Ok(cells)
}

fn cell_heights(
    height: &MapHeightField,
    x: usize,
    y: usize,
    width: usize,
) -> Result<[i32; 4], TerrainError> {
    let p0 = y
        .checked_mul(width)
        .and_then(|row| row.checked_add(x))
        .ok_or(TerrainError::TerrainTooLarge)?;
    let p1 = p0.checked_add(1).ok_or(TerrainError::TerrainTooLarge)?;
    let p3 = p0.checked_add(width).ok_or(TerrainError::TerrainTooLarge)?;
    let p2 = p3.checked_add(1).ok_or(TerrainError::TerrainTooLarge)?;
    Ok([
        i32::from(
            *height
                .samples()
                .get(p0)
                .ok_or(TerrainError::DimensionMismatch)?,
        ),
        i32::from(
            *height
                .samples()
                .get(p1)
                .ok_or(TerrainError::DimensionMismatch)?,
        ),
        i32::from(
            *height
                .samples()
                .get(p2)
                .ok_or(TerrainError::DimensionMismatch)?,
        ),
        i32::from(
            *height
                .samples()
                .get(p3)
                .ok_or(TerrainError::DimensionMismatch)?,
        ),
    ])
}

fn height_diagonal_flip(
    height: &MapHeightField,
    x: usize,
    y: usize,
    width: usize,
) -> Result<bool, TerrainError> {
    let [p0, p1, p2, p3] = cell_heights(height, x, y, width)?;
    Ok((p0 - p2).abs() > (p1 - p3).abs())
}

fn legacy_uv_adjusts(
    height: &MapHeightField,
    x: usize,
    y: usize,
    width: usize,
) -> Result<bool, TerrainError> {
    let corners = cell_heights(height, x, y, width)?;
    let minimum = *corners
        .iter()
        .min()
        .ok_or(TerrainError::DimensionMismatch)?;
    let maximum = *corners
        .iter()
        .max()
        .ok_or(TerrainError::DimensionMismatch)?;
    let delta = maximum - minimum;
    let scaled = f32::from(u8::try_from(delta).map_err(|_| TerrainError::TerrainTooLarge)?)
        * (TERRAIN_HEIGHT_SCALE / TERRAIN_XY_SCALE);
    if scaled < 1.5 {
        return Ok(false);
    }
    let below_limit = minimum + (2 * delta + 1) / 3;
    let above_limit = minimum + (delta + 1) / 3;
    let below = corners
        .iter()
        .filter(|height| **height < below_limit)
        .count();
    let above = corners
        .iter()
        .filter(|height| **height > above_limit)
        .count();
    if above != 1 && below != 1 && (above != 2 || below != 2) && scaled < 2.4 {
        return Ok(false);
    }
    if below == 1 || above > below || above == 1 || below > above {
        return Ok(true);
    }
    Ok(scaled >= 2.0)
}

fn layer_for_index(index: i16, blend: &MapBlendData) -> Result<Option<TerrainLayer>, TerrainError> {
    if index == 0 {
        return Ok(None);
    }
    let index =
        u32::try_from(index).map_err(|_| TerrainError::InvalidBlendIndex(i32::from(index)))?;
    let offset = usize::try_from(index - 1).map_err(|_| TerrainError::TerrainTooLarge)?;
    let record = blend
        .blend_tiles()
        .get(offset)
        .filter(|record| record.table_index() == index)
        .ok_or(TerrainError::InvalidBlendIndex(
            i32::try_from(index).unwrap_or(i32::MAX),
        ))?;
    if record.blend_index() < 0 {
        return Err(TerrainError::InvalidTileIndex(record.blend_index()));
    }
    Ok(Some(TerrainLayer {
        table_index: index,
        tile_index: record.blend_index(),
        alpha_corners: alpha_corners(record),
        custom_edge_class: record.custom_edge_class(),
    }))
}

fn cliff_flip(index: i16, blend: &MapBlendData) -> Result<bool, TerrainError> {
    if index == 0 {
        return Ok(false);
    }
    let index =
        u32::try_from(index).map_err(|_| TerrainError::InvalidCliffIndex(i32::from(index)))?;
    let offset = usize::try_from(index - 1).map_err(|_| TerrainError::TerrainTooLarge)?;
    blend
        .cliff_info()
        .get(offset)
        .filter(|record| record.table_index() == index)
        .map(|record| record.flip() != 0)
        .ok_or(TerrainError::InvalidCliffIndex(
            i32::try_from(index).unwrap_or(i32::MAX),
        ))
}

fn alpha_corners(record: &MapBlendTile) -> [u8; 4] {
    if record.custom_edge_class() >= 0 {
        return [0; 4];
    }
    let inverted = record.inverted() & INVERTED_MASK != 0;
    if record.horizontal() != 0 {
        return if inverted {
            [255, 0, 0, 255]
        } else {
            [0, 255, 255, 0]
        };
    }
    if record.vertical() != 0 {
        return if inverted {
            [255, 255, 0, 0]
        } else {
            [0, 0, 255, 255]
        };
    }
    if record.right_diagonal() != 0 {
        return if inverted {
            if record.long_diagonal() != 0 {
                [255, 255, 255, 0]
            } else {
                [0, 255, 0, 0]
            }
        } else if record.long_diagonal() != 0 {
            [0, 255, 255, 255]
        } else {
            [0, 0, 255, 0]
        };
    }
    if record.left_diagonal() != 0 {
        return if inverted {
            if record.long_diagonal() != 0 {
                [255, 255, 0, 255]
            } else {
                [255, 0, 0, 0]
            }
        } else if record.long_diagonal() != 0 {
            [255, 0, 255, 255]
        } else {
            [0, 0, 0, 255]
        };
    }
    [0; 4]
}

fn layer_needs_flip(layer: TerrainLayer, blend: &MapBlendData) -> bool {
    let Ok(offset) = usize::try_from(layer.table_index - 1) else {
        return false;
    };
    let Some(record) = blend.blend_tiles().get(offset) else {
        return false;
    };
    if record.custom_edge_class() >= 0 {
        return false;
    }
    let inverted = record.inverted() & INVERTED_MASK != 0;
    if record.horizontal() != 0 || record.vertical() != 0 {
        return record.inverted() & FORCED_FLIP_MASK != 0;
    }
    (record.right_diagonal() != 0 && !inverted) || (record.left_diagonal() != 0 && inverted)
}

#[allow(clippy::cast_precision_loss)]
fn stage_geometry(
    height: &MapHeightField,
    cells: &[TerrainCell],
    width: usize,
    height_count: usize,
) -> Result<(Vec<TerrainVertex>, Vec<u32>), TerrainError> {
    let mut vertices = Vec::with_capacity(height.samples().len());
    let width_divisor = (width - 1) as f32;
    let height_divisor = (height_count - 1) as f32;
    for y in 0..height_count {
        for x in 0..width {
            let index = y
                .checked_mul(width)
                .and_then(|row| row.checked_add(x))
                .ok_or(TerrainError::TerrainTooLarge)?;
            let sample = *height
                .samples()
                .get(index)
                .ok_or(TerrainError::DimensionMismatch)?;
            vertices.push(TerrainVertex {
                position: [
                    (x as f32 - height.border_size() as f32) * TERRAIN_XY_SCALE,
                    (y as f32 - height.border_size() as f32) * TERRAIN_XY_SCALE,
                    f32::from(sample) * TERRAIN_HEIGHT_SCALE,
                ],
                uv: [x as f32 / width_divisor, 1.0 - y as f32 / height_divisor],
            });
        }
    }

    let index_capacity = cells
        .len()
        .checked_mul(6)
        .ok_or(TerrainError::TerrainTooLarge)?;
    let mut indices = Vec::with_capacity(index_capacity);
    for cell in cells {
        let x = usize::try_from(cell.x).map_err(|_| TerrainError::TerrainTooLarge)?;
        let y = usize::try_from(cell.y).map_err(|_| TerrainError::TerrainTooLarge)?;
        let p0 = y
            .checked_mul(width)
            .and_then(|row| row.checked_add(x))
            .ok_or(TerrainError::TerrainTooLarge)?;
        let p1 = p0.checked_add(1).ok_or(TerrainError::TerrainTooLarge)?;
        let p3 = p0.checked_add(width).ok_or(TerrainError::TerrainTooLarge)?;
        let p2 = p3.checked_add(1).ok_or(TerrainError::TerrainTooLarge)?;
        let p0 = u32::try_from(p0).map_err(|_| TerrainError::TerrainTooLarge)?;
        let p1 = u32::try_from(p1).map_err(|_| TerrainError::TerrainTooLarge)?;
        let p2 = u32::try_from(p2).map_err(|_| TerrainError::TerrainTooLarge)?;
        let p3 = u32::try_from(p3).map_err(|_| TerrainError::TerrainTooLarge)?;
        if cell.flipped {
            indices.extend_from_slice(&[p1, p3, p0, p1, p2, p3]);
        } else {
            indices.extend_from_slice(&[p0, p2, p3, p0, p1, p2]);
        }
    }
    Ok((vertices, indices))
}

fn stage_edge_indices(cells: &[TerrainCell], width: usize) -> Result<Vec<u32>, TerrainError> {
    let edge_count = cells
        .iter()
        .filter(|cell| {
            cell.primary
                .is_some_and(|layer| layer.custom_edge_class >= 0)
        })
        .count();
    let mut indices = Vec::with_capacity(
        edge_count
            .checked_mul(6)
            .ok_or(TerrainError::TerrainTooLarge)?,
    );
    for cell in cells {
        if cell.primary.is_none_or(|layer| layer.custom_edge_class < 0) {
            continue;
        }
        let x = usize::try_from(cell.x).map_err(|_| TerrainError::TerrainTooLarge)?;
        let y = usize::try_from(cell.y).map_err(|_| TerrainError::TerrainTooLarge)?;
        let p0 = y
            .checked_mul(width)
            .and_then(|row| row.checked_add(x))
            .ok_or(TerrainError::TerrainTooLarge)?;
        let p1 = p0.checked_add(1).ok_or(TerrainError::TerrainTooLarge)?;
        let p3 = p0.checked_add(width).ok_or(TerrainError::TerrainTooLarge)?;
        let p2 = p3.checked_add(1).ok_or(TerrainError::TerrainTooLarge)?;
        let [p0, p1, p2, p3] = [p0, p1, p2, p3]
            .map(|index| u32::try_from(index).map_err(|_| TerrainError::TerrainTooLarge))
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .map_err(|_| TerrainError::TerrainTooLarge)?;
        if cell.flipped {
            indices.extend_from_slice(&[p1, p3, p0, p1, p2, p3]);
        } else {
            indices.extend_from_slice(&[p0, p2, p3, p0, p1, p2]);
        }
    }
    Ok(indices)
}

#[allow(clippy::too_many_lines)]
fn bake_texture(
    height_field: &MapHeightField,
    blend: &MapBlendData,
    textures: &TextureResourceManager,
    options: TerrainStagingOptions,
    cells: &[TerrainCell],
    width: usize,
    height: usize,
) -> Result<BakedTerrainTextures, TerrainError> {
    bake_texture_region(
        height_field,
        blend,
        textures,
        options,
        cells,
        width,
        height,
        TerrainRegion {
            min_x: 0,
            min_y: 0,
            max_x: width.checked_sub(1).ok_or(TerrainError::EmptyTerrain)?,
            max_y: height.checked_sub(1).ok_or(TerrainError::EmptyTerrain)?,
        },
    )
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn bake_texture_region(
    height_field: &MapHeightField,
    blend: &MapBlendData,
    textures: &TextureResourceManager,
    options: TerrainStagingOptions,
    cells: &[TerrainCell],
    width: usize,
    height: usize,
    region: TerrainRegion,
) -> Result<BakedTerrainTextures, TerrainError> {
    bake_texture_region_controlled(
        height_field,
        blend,
        textures,
        options,
        cells,
        width,
        height,
        region,
        &|| false,
    )
    .map(|result| result.expect("an uncancelled terrain bake completes"))
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn bake_texture_region_controlled<F>(
    height_field: &MapHeightField,
    blend: &MapBlendData,
    textures: &TextureResourceManager,
    options: TerrainStagingOptions,
    cells: &[TerrainCell],
    width: usize,
    height: usize,
    region: TerrainRegion,
    cancelled: &F,
) -> Result<Option<BakedTerrainTextures>, TerrainError>
where
    F: Fn() -> bool,
{
    if region.min_x >= region.max_x
        || region.min_y >= region.max_y
        || region.max_x >= width
        || region.max_y >= height
    {
        return Err(TerrainError::DimensionMismatch);
    }
    let pixels = usize::try_from(options.pixels_per_cell)
        .map_err(|_| TerrainError::InvalidPixelsPerCell(options.pixels_per_cell))?;
    let texture_width = region
        .width()
        .checked_mul(pixels)
        .ok_or(TerrainError::TerrainTooLarge)?;
    let texture_height = region
        .height()
        .checked_mul(pixels)
        .ok_or(TerrainError::TerrainTooLarge)?;
    let byte_count = texture_width
        .checked_mul(texture_height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(TerrainError::TerrainTooLarge)?;
    if texture_width > MAX_BAKED_TEXTURE_DIMENSION
        || texture_height > MAX_BAKED_TEXTURE_DIMENSION
        || byte_count > MAX_BAKED_TEXTURE_BYTES
    {
        return Err(TerrainError::TerrainTooLarge);
    }

    let cell_stride = width.checked_sub(1).ok_or(TerrainError::EmptyTerrain)?;
    let mut region_cells = Vec::with_capacity(
        region
            .width()
            .checked_mul(region.height())
            .ok_or(TerrainError::TerrainTooLarge)?,
    );
    for y in region.min_y..region.max_y {
        if cancelled() {
            return Ok(None);
        }
        for x in region.min_x..region.max_x {
            region_cells.push(
                *cells
                    .get(y * cell_stride + x)
                    .ok_or(TerrainError::DimensionMismatch)?,
            );
        }
    }

    let mut needed_tiles = BTreeSet::new();
    for cell in &region_cells {
        needed_tiles.insert(i32::from(cell.base_tile));
        for layer in [cell.primary, cell.extra].into_iter().flatten() {
            needed_tiles.insert(layer.tile_index);
        }
    }
    let mut tile_cache = BTreeMap::new();
    for tile in needed_tiles {
        if cancelled() {
            return Ok(None);
        }
        tile_cache.insert(tile, extract_cell_tile(tile, pixels, blend, textures)?);
    }

    let mut alpha_cache = BTreeMap::new();
    for cell in &region_cells {
        if cancelled() {
            return Ok(None);
        }
        for layer in [cell.primary, cell.extra].into_iter().flatten() {
            if layer.custom_edge_class < 0 && !alpha_cache.contains_key(&layer.table_index) {
                let offset = usize::try_from(layer.table_index - 1)
                    .map_err(|_| TerrainError::TerrainTooLarge)?;
                let record =
                    blend
                        .blend_tiles()
                        .get(offset)
                        .ok_or(TerrainError::InvalidBlendIndex(
                            i32::try_from(layer.table_index).unwrap_or(i32::MAX),
                        ))?;
                alpha_cache.insert(layer.table_index, blend_alpha_mask(record, pixels));
            }
        }
    }

    let mut rgba = vec![0_u8; byte_count];
    let mut edge_rgba = vec![0_u8; byte_count];
    for cell in &region_cells {
        if cancelled() {
            return Ok(None);
        }
        let mut cell_rgba = cell_tile(
            i32::from(cell.base_tile),
            *cell,
            height_field,
            width,
            pixels,
            blend,
            textures,
            options.compatibility,
            &tile_cache,
        )?;
        for layer in [cell.primary, cell.extra].into_iter().flatten() {
            if layer.custom_edge_class >= 0 {
                continue;
            }
            let source = cell_tile(
                layer.tile_index,
                *cell,
                height_field,
                width,
                pixels,
                blend,
                textures,
                options.compatibility,
                &tile_cache,
            )?;
            let alpha =
                alpha_cache
                    .get(&layer.table_index)
                    .ok_or(TerrainError::InvalidBlendIndex(
                        i32::try_from(layer.table_index).unwrap_or(i32::MAX),
                    ))?;
            composite_layer(&mut cell_rgba, &source, alpha)?;
        }
        let cell_x = usize::try_from(cell.x).map_err(|_| TerrainError::TerrainTooLarge)?;
        let cell_y = usize::try_from(cell.y).map_err(|_| TerrainError::TerrainTooLarge)?;
        if options.compatibility == TerrainCompatibilityPolicy::Modern {
            apply_modern_macro_variation(&mut cell_rgba, cell_x, cell_y, pixels)?;
        }
        let destination_x = cell_x
            .checked_sub(region.min_x)
            .ok_or(TerrainError::DimensionMismatch)?;
        let destination_y = region
            .max_y
            .checked_sub(cell_y)
            .and_then(|value| value.checked_sub(1))
            .ok_or(TerrainError::DimensionMismatch)?
            .checked_mul(pixels)
            .ok_or(TerrainError::TerrainTooLarge)?;
        copy_cell(
            &mut rgba,
            texture_width,
            destination_x * pixels,
            destination_y,
            pixels,
            &cell_rgba,
        )?;
        if let Some(layer) = cell.primary.filter(|layer| layer.custom_edge_class >= 0) {
            let blend_tile = cell_tile(
                layer.tile_index,
                *cell,
                height_field,
                width,
                pixels,
                blend,
                textures,
                options.compatibility,
                &tile_cache,
            )?;
            let edge = extract_custom_edge(layer, *cell, pixels, blend, textures, &blend_tile)?;
            copy_cell(
                &mut edge_rgba,
                texture_width,
                destination_x * pixels,
                destination_y,
                pixels,
                &edge,
            )?;
        }
    }
    Ok(Some((
        u32::try_from(texture_width).map_err(|_| TerrainError::TerrainTooLarge)?,
        u32::try_from(texture_height).map_err(|_| TerrainError::TerrainTooLarge)?,
        rgba,
        edge_rgba,
    )))
}

fn extract_custom_edge(
    layer: TerrainLayer,
    cell: TerrainCell,
    pixels: usize,
    blend: &MapBlendData,
    textures: &TextureResourceManager,
    blend_tile: &[u8],
) -> Result<Vec<u8>, TerrainError> {
    let class_index = usize::try_from(layer.custom_edge_class)
        .map_err(|_| TerrainError::InvalidEdgeClass(layer.custom_edge_class))?;
    let class = blend
        .edge_texture_classes()
        .get(class_index)
        .ok_or(TerrainError::InvalidEdgeClass(layer.custom_edge_class))?;
    let image = textures
        .image(class.name_bytes())
        .ok_or_else(|| TerrainError::MissingTexture(class.name_bytes().to_vec()))?;
    let class_width = usize::try_from(class.width()).map_err(|_| TerrainError::TerrainTooLarge)?;
    let required = class_width
        .checked_mul(SOURCE_TILE_PIXELS)
        .ok_or(TerrainError::TerrainTooLarge)?;
    let region_size = required / 4;
    if required % 4 != 0 || region_size == 0 {
        return Err(TerrainError::InvalidTextureSheet(
            class.name_bytes().to_vec(),
        ));
    }
    let image_width = usize::try_from(image.width()).map_err(|_| TerrainError::TerrainTooLarge)?;
    let image_height =
        usize::try_from(image.height()).map_err(|_| TerrainError::TerrainTooLarge)?;
    if image_width < required || image_height < required {
        return Err(TerrainError::InvalidTextureSheet(
            class.name_bytes().to_vec(),
        ));
    }
    let offset =
        usize::try_from(layer.table_index - 1).map_err(|_| TerrainError::TerrainTooLarge)?;
    let record = blend
        .blend_tiles()
        .get(offset)
        .ok_or(TerrainError::InvalidBlendIndex(
            i32::try_from(layer.table_index).unwrap_or(i32::MAX),
        ))?;
    let (quarter_x, quarter_y) = custom_edge_quarter(*record, cell)?;
    let origin_x = quarter_x
        .checked_mul(region_size)
        .ok_or(TerrainError::TerrainTooLarge)?;
    let origin_y = image_height
        .checked_sub(required)
        .and_then(|origin| origin.checked_add(quarter_y * region_size))
        .ok_or_else(|| TerrainError::InvalidTextureSheet(class.name_bytes().to_vec()))?;
    let mut edge = copy_region(image.rgba(), image_width, origin_x, origin_y, region_size)?;
    let mut size = region_size;
    while size > pixels && size.is_multiple_of(2) {
        edge = mip_rgba(&edge, size)?;
        size /= 2;
    }
    if size != pixels {
        edge = resize_nearest_rgba(&edge, size, pixels)?;
    }
    if blend_tile.len() != edge.len() {
        return Err(TerrainError::DimensionMismatch);
    }
    for (texel, material) in edge.chunks_exact_mut(4).zip(blend_tile.chunks_exact(4)) {
        let black = texel[..3] == [0, 0, 0];
        let white = texel[..3] == [255, 255, 255];
        if white {
            texel[..3].copy_from_slice(&material[..3]);
            texel[3] = 128;
        } else {
            texel[3] = if black { 0 } else { 255 };
        }
    }
    Ok(edge)
}

fn custom_edge_quarter(
    record: MapBlendTile,
    cell: TerrainCell,
) -> Result<(usize, usize), TerrainError> {
    let inverted = record.inverted() != 0;
    if record.horizontal() != 0 {
        return Ok((
            usize::from(inverted) * 3,
            1 + usize::try_from(cell.y & 1).unwrap_or(0),
        ));
    }
    if record.vertical() != 0 {
        return Ok((
            1 + usize::try_from(cell.x & 1).unwrap_or(0),
            if inverted { 0 } else { 3 },
        ));
    }
    if record.right_diagonal() != 0 {
        return Ok(match (record.long_diagonal() != 0, inverted) {
            (true, false) => (2, 1),
            (true, true) => (2, 2),
            (false, false) => (0, 3),
            (false, true) => (0, 0),
        });
    }
    if record.left_diagonal() != 0 {
        return Ok(match (record.long_diagonal() != 0, inverted) {
            (true, false) => (1, 1),
            (true, true) => (1, 2),
            (false, false) => (3, 3),
            (false, true) => (3, 0),
        });
    }
    Err(TerrainError::InvalidBlendIndex(
        i32::try_from(record.table_index()).unwrap_or(i32::MAX),
    ))
}

fn resize_nearest_rgba(
    source: &[u8],
    source_size: usize,
    target_size: usize,
) -> Result<Vec<u8>, TerrainError> {
    let target_len = target_size
        .checked_mul(target_size)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(TerrainError::TerrainTooLarge)?;
    let mut target = vec![0; target_len];
    for y in 0..target_size {
        for x in 0..target_size {
            let source_x = x * source_size / target_size;
            let source_y = y * source_size / target_size;
            let source_offset = (source_y * source_size + source_x) * 4;
            let target_offset = (y * target_size + x) * 4;
            target[target_offset..target_offset + 4]
                .copy_from_slice(&source[source_offset..source_offset + 4]);
        }
    }
    Ok(target)
}

#[allow(clippy::too_many_arguments)]
fn cell_tile(
    packed_index: i32,
    cell: TerrainCell,
    height: &MapHeightField,
    grid_width: usize,
    pixels: usize,
    blend: &MapBlendData,
    textures: &TextureResourceManager,
    compatibility: TerrainCompatibilityPolicy,
    cache: &BTreeMap<i32, Vec<u8>>,
) -> Result<Vec<u8>, TerrainError> {
    let corners =
        adjusted_uv_corners(packed_index, cell, height, grid_width, blend, compatibility)?;
    let Some(corners) = corners else {
        return cache
            .get(&packed_index)
            .cloned()
            .ok_or(TerrainError::InvalidTileIndex(packed_index));
    };
    let (_, class) = terrain_class(packed_index, blend)?;
    let image = textures
        .image(class.name_bytes())
        .ok_or_else(|| TerrainError::MissingTexture(class.name_bytes().to_vec()))?;
    let class_width = usize::try_from(class.width()).map_err(|_| TerrainError::TerrainTooLarge)?;
    let required = class_width
        .checked_mul(SOURCE_TILE_PIXELS)
        .ok_or(TerrainError::TerrainTooLarge)?;
    let image_width = usize::try_from(image.width()).map_err(|_| TerrainError::TerrainTooLarge)?;
    let image_height =
        usize::try_from(image.height()).map_err(|_| TerrainError::TerrainTooLarge)?;
    if image_width < required || image_height < required {
        return Err(TerrainError::InvalidTextureSheet(
            class.name_bytes().to_vec(),
        ));
    }
    sample_uv_quad(
        image.rgba(),
        image_width,
        image_height - required,
        required,
        corners,
        pixels,
    )
}

fn terrain_class(
    packed_index: i32,
    blend: &MapBlendData,
) -> Result<(u32, &MapTextureClass), TerrainError> {
    let packed =
        u32::try_from(packed_index).map_err(|_| TerrainError::InvalidTileIndex(packed_index))?;
    let base = packed >> 2;
    let class = blend
        .texture_classes()
        .iter()
        .find(|class| {
            class
                .first_tile()
                .checked_add(class.tile_count())
                .is_some_and(|end| base >= class.first_tile() && base < end)
        })
        .ok_or(TerrainError::UnclassifiedTile(base))?;
    Ok((base, class))
}

fn adjusted_uv_corners(
    packed_index: i32,
    cell: TerrainCell,
    height: &MapHeightField,
    grid_width: usize,
    blend: &MapBlendData,
    compatibility: TerrainCompatibilityPolicy,
) -> Result<Option<[[f32; 2]; 4]>, TerrainError> {
    let (base, class) = terrain_class(packed_index, blend)?;
    if cell.cliff_info > 0 {
        let info_index = u32::try_from(cell.cliff_info)
            .map_err(|_| TerrainError::InvalidCliffIndex(i32::from(cell.cliff_info)))?;
        let offset = usize::try_from(info_index - 1).map_err(|_| TerrainError::TerrainTooLarge)?;
        let info = blend
            .cliff_info()
            .get(offset)
            .filter(|info| info.table_index() == info_index)
            .ok_or(TerrainError::InvalidCliffIndex(i32::from(cell.cliff_info)))?;
        let info_tile = u32::try_from(info.tile_index())
            .map_err(|_| TerrainError::InvalidTileIndex(info.tile_index()))?
            >> 2;
        let class_end = class
            .first_tile()
            .checked_add(class.tile_count())
            .ok_or(TerrainError::TerrainTooLarge)?;
        if base >= class.first_tile()
            && base < class_end
            && info_tile >= class.first_tile()
            && info_tile < class_end
        {
            let extent = class
                .width()
                .checked_mul(SOURCE_TILE_PIXELS_U32)
                .ok_or(TerrainError::TerrainTooLarge)?;
            #[allow(clippy::cast_precision_loss)]
            let scale = SOURCE_TERRAIN_ATLAS_WIDTH / extent as f32;
            let uv = info.uv();
            return Ok(Some([
                [uv[0] * scale, 1.0 + uv[1] * scale],
                [uv[2] * scale, 1.0 + uv[3] * scale],
                [uv[4] * scale, 1.0 + uv[5] * scale],
                [uv[6] * scale, 1.0 + uv[7] * scale],
            ]));
        }
        return Ok(None);
    }
    if compatibility != TerrainCompatibilityPolicy::ZeroHourLegacy {
        return Ok(None);
    }
    let x = usize::try_from(cell.x).map_err(|_| TerrainError::TerrainTooLarge)?;
    let y = usize::try_from(cell.y).map_err(|_| TerrainError::TerrainTooLarge)?;
    if !legacy_uv_adjusts(height, x, y, grid_width)? {
        return Ok(None);
    }
    let corners = default_class_uv_corners(packed_index, blend)?;
    Ok(Some(legacy_adjust_uv(
        corners,
        cell_heights(height, x, y, grid_width)?,
    )))
}

#[allow(clippy::cast_precision_loss)]
fn default_class_uv_corners(
    packed_index: i32,
    blend: &MapBlendData,
) -> Result<[[f32; 2]; 4], TerrainError> {
    let packed =
        u32::try_from(packed_index).map_err(|_| TerrainError::InvalidTileIndex(packed_index))?;
    let (base, class) = terrain_class(packed_index, blend)?;
    let class_width = usize::try_from(class.width()).map_err(|_| TerrainError::TerrainTooLarge)?;
    let local =
        usize::try_from(base - class.first_tile()).map_err(|_| TerrainError::TerrainTooLarge)?;
    let tile_x = local % class_width;
    let tile_row_from_bottom = local / class_width;
    if tile_row_from_bottom >= class_width {
        return Err(TerrainError::UnclassifiedTile(base));
    }
    let extent = (class_width * SOURCE_TILE_PIXELS) as f32;
    let quadrant_x = usize::from(packed & 1 != 0);
    let quadrant_y = usize::from(packed & 2 == 0);
    let left =
        (tile_x * SOURCE_TILE_PIXELS + quadrant_x * (SOURCE_TILE_PIXELS / 2)) as f32 / extent;
    let top = ((class_width - 1 - tile_row_from_bottom) * SOURCE_TILE_PIXELS
        + quadrant_y * (SOURCE_TILE_PIXELS / 2)) as f32
        / extent;
    let right = left + (SOURCE_TILE_PIXELS / 2) as f32 / extent;
    let bottom = top + (SOURCE_TILE_PIXELS / 2) as f32 / extent;
    Ok([[left, bottom], [right, bottom], [right, top], [left, top]])
}

fn legacy_adjust_uv(mut corners: [[f32; 2]; 4], heights: [i32; 4]) -> [[f32; 2]; 4] {
    let minimum = *heights.iter().min().expect("four terrain heights");
    let maximum = *heights.iter().max().expect("four terrain heights");
    let delta = maximum - minimum;
    let scaled = f32::from(u8::try_from(delta).expect("height-byte delta"))
        * (TERRAIN_HEIGHT_SCALE / TERRAIN_XY_SCALE);
    let below_limit = minimum + (2 * delta + 1) / 3;
    let above_limit = minimum + (delta + 1) / 3;
    let below = heights
        .iter()
        .filter(|height| **height < below_limit)
        .count();
    let above = heights
        .iter()
        .filter(|height| **height > above_limit)
        .count();
    let divisor = (4.0 / scaled).clamp(1.0, 4.0);
    let top = corners[3][1];
    let bottom = corners[0][1];
    if below == 1 || above > below {
        if let Some(index) = heights.iter().position(|height| *height == minimum) {
            corners[index][1] = if index < 2 {
                top + 1.0 / divisor
            } else {
                bottom - 1.0 / divisor
            };
        }
    } else if above == 1 || below > above {
        if let Some(index) = heights.iter().position(|height| *height == maximum) {
            corners[index][1] = if index < 2 {
                top + 1.0 / divisor
            } else {
                bottom - 1.0 / divisor
            };
        }
    } else {
        let horizontal_scale = corners[1][0] - corners[0][0];
        let vertical_scale = corners[0][1] - corners[3][1];
        let dx = stretched_extent(heights[3] - heights[2]) * horizontal_scale;
        let dy = stretched_extent(heights[3] - heights[0]) * vertical_scale;
        let minimum_u = corners[0][0];
        let minimum_v = corners[3][1];
        corners[0] = [minimum_u, minimum_v + dy];
        corners[1] = [minimum_u + dx, minimum_v + dy];
        corners[2] = [minimum_u + dx, minimum_v];
        corners[3] = [minimum_u, minimum_v];
        let dx = stretched_extent(heights[1] - heights[0]) * horizontal_scale;
        let dy = stretched_extent(heights[2] - heights[1]) * vertical_scale;
        corners[1][0] = corners[0][0] + dx;
        corners[1][1] = corners[3][1] + dy;
    }
    shift_uvs_into_class(&mut corners);
    corners
}

fn stretched_extent(delta: i32) -> f32 {
    let scaled = f32::from(i16::try_from(delta).expect("height-byte delta"))
        * (TERRAIN_HEIGHT_SCALE / TERRAIN_XY_SCALE);
    let length = (1.0 + scaled * scaled).sqrt();
    if length < 1.5 { 1.0 } else { length.min(4.0) }
}

fn shift_uvs_into_class(corners: &mut [[f32; 2]; 4]) {
    let upward = corners
        .iter()
        .map(|corner| -corner[1])
        .fold(0.0_f32, f32::max);
    for corner in &mut *corners {
        corner[1] += upward;
    }
    let leftward = corners
        .iter()
        .map(|corner| corner[0] - 1.0)
        .fold(0.0_f32, f32::max);
    let upward = corners
        .iter()
        .map(|corner| corner[1] - 1.0)
        .fold(0.0_f32, f32::max);
    for corner in corners {
        corner[0] -= leftward;
        corner[1] -= upward;
    }
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn sample_uv_quad(
    source: &[u8],
    source_width: usize,
    class_origin_y: usize,
    class_size: usize,
    corners: [[f32; 2]; 4],
    target_size: usize,
) -> Result<Vec<u8>, TerrainError> {
    let output_len = target_size
        .checked_mul(target_size)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(TerrainError::TerrainTooLarge)?;
    let mut output = vec![0; output_len];
    for y in 0..target_size {
        let vertical = (y as f32 + 0.5) / target_size as f32;
        for x in 0..target_size {
            let horizontal = (x as f32 + 0.5) / target_size as f32;
            let top = lerp_uv(corners[3], corners[2], horizontal);
            let bottom = lerp_uv(corners[0], corners[1], horizontal);
            let uv = lerp_uv(top, bottom, vertical);
            let source_x = (uv[0].clamp(0.0, 1.0) * (class_size - 1) as f32).round() as usize;
            let source_y =
                class_origin_y + (uv[1].clamp(0.0, 1.0) * (class_size - 1) as f32).round() as usize;
            let source_offset = (source_y * source_width + source_x) * 4;
            let target_offset = (y * target_size + x) * 4;
            output[target_offset..target_offset + 4]
                .copy_from_slice(&source[source_offset..source_offset + 4]);
        }
    }
    Ok(output)
}

fn lerp_uv(left: [f32; 2], right: [f32; 2], amount: f32) -> [f32; 2] {
    [
        left[0] + (right[0] - left[0]) * amount,
        left[1] + (right[1] - left[1]) * amount,
    ]
}

fn extract_cell_tile(
    packed_index: i32,
    pixels: usize,
    blend: &MapBlendData,
    textures: &TextureResourceManager,
) -> Result<Vec<u8>, TerrainError> {
    let packed =
        u32::try_from(packed_index).map_err(|_| TerrainError::InvalidTileIndex(packed_index))?;
    let base = packed >> 2;
    let class = blend
        .texture_classes()
        .iter()
        .find(|class| {
            class
                .first_tile()
                .checked_add(class.tile_count())
                .is_some_and(|end| base >= class.first_tile() && base < end)
        })
        .ok_or(TerrainError::UnclassifiedTile(base))?;
    let image = textures
        .image(class.name_bytes())
        .ok_or_else(|| TerrainError::MissingTexture(class.name_bytes().to_vec()))?;
    let class_width = usize::try_from(class.width()).map_err(|_| TerrainError::TerrainTooLarge)?;
    let required = class_width
        .checked_mul(SOURCE_TILE_PIXELS)
        .ok_or(TerrainError::TerrainTooLarge)?;
    let image_width = usize::try_from(image.width()).map_err(|_| TerrainError::TerrainTooLarge)?;
    let image_height =
        usize::try_from(image.height()).map_err(|_| TerrainError::TerrainTooLarge)?;
    if image_width < required || image_height < required {
        return Err(TerrainError::InvalidTextureSheet(
            class.name_bytes().to_vec(),
        ));
    }
    let local =
        usize::try_from(base - class.first_tile()).map_err(|_| TerrainError::TerrainTooLarge)?;
    let tile_x = local % class_width;
    let tile_row_from_bottom = local / class_width;
    if tile_row_from_bottom >= class_width {
        return Err(TerrainError::UnclassifiedTile(base));
    }
    let origin_x = tile_x
        .checked_mul(SOURCE_TILE_PIXELS)
        .ok_or(TerrainError::TerrainTooLarge)?;
    let origin_y = image_height
        .checked_sub((tile_row_from_bottom + 1) * SOURCE_TILE_PIXELS)
        .ok_or_else(|| TerrainError::InvalidTextureSheet(class.name_bytes().to_vec()))?;
    let mut tile = copy_region(
        image.rgba(),
        image_width,
        origin_x,
        origin_y,
        SOURCE_TILE_PIXELS,
    )?;
    let target = pixels.checked_mul(2).ok_or(TerrainError::TerrainTooLarge)?;
    let mut size = SOURCE_TILE_PIXELS;
    while size > target {
        tile = mip_rgba(&tile, size)?;
        size /= 2;
    }
    if size != target {
        return Err(TerrainError::InvalidPixelsPerCell(
            u32::try_from(pixels).unwrap_or(u32::MAX),
        ));
    }
    let quadrant_x = usize::from(packed & 1 != 0);
    let quadrant_y = usize::from(packed & 2 == 0);
    copy_region(
        &tile,
        size,
        quadrant_x * pixels,
        quadrant_y * pixels,
        pixels,
    )
}

fn blend_alpha_mask(record: &MapBlendTile, pixels: usize) -> Vec<u8> {
    let inverted = record.inverted() & INVERTED_MASK != 0;
    let mut raw = vec![0_u8; SOURCE_TILE_PIXELS * SOURCE_TILE_PIXELS];
    for source_y in 0..SOURCE_TILE_PIXELS {
        for x in 0..SOURCE_TILE_PIXELS {
            let mut h = i32::try_from(x).expect("source alpha X fits i32");
            let mut v = i32::try_from(source_y).expect("source alpha Y fits i32");
            let extent = i32::try_from(SOURCE_TILE_PIXELS).expect("tile extent fits i32");
            let mut alpha = 255_i32;
            if record.horizontal() != 0 {
                if !inverted {
                    h = extent - h - 1;
                }
                alpha = alpha * h / (extent - 1);
            } else if record.vertical() != 0 {
                if !inverted {
                    v = extent - v - 1;
                }
                alpha = alpha * v / (extent - 1);
            } else if record.right_diagonal() != 0 {
                h = extent - h - 1;
                if !inverted {
                    v = extent - v - 1;
                }
                v += h;
                if record.long_diagonal() != 0 {
                    v -= extent;
                }
                alpha = alpha * v / (extent - 1);
            } else if record.left_diagonal() != 0 {
                if !inverted {
                    v = extent - v - 1;
                }
                v += h;
                if record.long_diagonal() != 0 {
                    v -= extent;
                }
                alpha = alpha * v / (extent - 1);
            }
            let alpha =
                u8::try_from((255 - alpha).clamp(0, 255)).expect("clamped terrain alpha fits u8");
            raw[source_y * SOURCE_TILE_PIXELS + x] = alpha;
        }
    }
    let mut size = SOURCE_TILE_PIXELS;
    while size > pixels {
        raw = mip_alpha(&raw, size);
        size /= 2;
    }
    let mut top_down = vec![0_u8; pixels * pixels];
    for y in 0..pixels {
        let source = (pixels - 1 - y) * pixels;
        let target = y * pixels;
        top_down[target..target + pixels].copy_from_slice(&raw[source..source + pixels]);
    }
    top_down
}

fn mip_alpha(source: &[u8], size: usize) -> Vec<u8> {
    let target_size = size / 2;
    let mut target = vec![0_u8; target_size * target_size];
    for y in 0..target_size {
        for x in 0..target_size {
            let source_x = x * 2;
            let source_y = y * 2;
            let offsets = [
                source_y * size + source_x,
                source_y * size + source_x + 1,
                (source_y + 1) * size + source_x,
                (source_y + 1) * size + source_x + 1,
            ];
            let sum = offsets
                .iter()
                .map(|offset| u16::from(source[*offset]))
                .sum::<u16>();
            target[y * target_size + x] =
                u8::try_from((sum + 2) / 4).expect("averaged alpha fits u8");
        }
    }
    target
}

fn mip_rgba(source: &[u8], size: usize) -> Result<Vec<u8>, TerrainError> {
    let target_size = size / 2;
    let target_len = target_size
        .checked_mul(target_size)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(TerrainError::TerrainTooLarge)?;
    let mut target = vec![0_u8; target_len];
    for y in 0..target_size {
        for x in 0..target_size {
            for channel in 0..4 {
                let source_x = x * 2;
                let source_y = y * 2;
                let offsets = [
                    ((source_y * size + source_x) * 4) + channel,
                    ((source_y * size + source_x + 1) * 4) + channel,
                    (((source_y + 1) * size + source_x) * 4) + channel,
                    (((source_y + 1) * size + source_x + 1) * 4) + channel,
                ];
                let sum = offsets
                    .iter()
                    .map(|offset| u16::from(source[*offset]))
                    .sum::<u16>();
                target[(y * target_size + x) * 4 + channel] =
                    u8::try_from((sum + 2) / 4).expect("averaged color fits u8");
            }
        }
    }
    Ok(target)
}

fn composite_layer(
    destination: &mut [u8],
    source: &[u8],
    alpha: &[u8],
) -> Result<(), TerrainError> {
    if destination.len() != source.len() || destination.len() / 4 != alpha.len() {
        return Err(TerrainError::TerrainTooLarge);
    }
    for ((destination, source), alpha) in destination
        .chunks_exact_mut(4)
        .zip(source.chunks_exact(4))
        .zip(alpha)
    {
        let alpha = u16::from(*alpha);
        for channel in 0..3 {
            let blended = (u16::from(source[channel]) * alpha) / 255
                + (u16::from(destination[channel]) * (255 - alpha)) / 255;
            destination[channel] = u8::try_from(blended).unwrap_or(u8::MAX);
        }
        destination[3] = u8::MAX;
    }
    Ok(())
}

// Project-authored, world-anchored macro color variation. It does not rotate or mirror authored
// tiles, so directional material detail, blend masks, roads, and cliff UVs keep their semantics.
fn apply_modern_macro_variation(
    rgba: &mut [u8],
    cell_x: usize,
    cell_y: usize,
    pixels: usize,
) -> Result<(), TerrainError> {
    let expected = pixels
        .checked_mul(pixels)
        .and_then(|count| count.checked_mul(4))
        .ok_or(TerrainError::TerrainTooLarge)?;
    if rgba.len() != expected || pixels == 0 {
        return Err(TerrainError::DimensionMismatch);
    }
    let lattice_span = pixels.checked_mul(8).ok_or(TerrainError::TerrainTooLarge)?;
    for y in 0..pixels {
        for x in 0..pixels {
            let global_x = cell_x
                .checked_mul(pixels)
                .and_then(|value| value.checked_add(x))
                .ok_or(TerrainError::TerrainTooLarge)?;
            let global_y = cell_y
                .checked_mul(pixels)
                .and_then(|value| value.checked_add(y))
                .ok_or(TerrainError::TerrainTooLarge)?;
            let lattice_x = global_x / lattice_span;
            let lattice_y = global_y / lattice_span;
            let fraction_x = ((global_x % lattice_span) as u64) * 65_535 / lattice_span as u64;
            let fraction_y = ((global_y % lattice_span) as u64) * 65_535 / lattice_span as u64;
            let smooth_x = smooth_fixed(fraction_x);
            let smooth_y = smooth_fixed(fraction_y);
            let top = lerp_fixed(
                macro_hash(lattice_x, lattice_y),
                macro_hash(lattice_x + 1, lattice_y),
                smooth_x,
            );
            let bottom = lerp_fixed(
                macro_hash(lattice_x, lattice_y + 1),
                macro_hash(lattice_x + 1, lattice_y + 1),
                smooth_x,
            );
            let noise = lerp_fixed(top, bottom, smooth_y);
            let factor = 242_u64 + noise * 28 / 255;
            let offset = (y * pixels + x) * 4;
            for channel in &mut rgba[offset..offset + 3] {
                *channel =
                    u8::try_from((u64::from(*channel) * factor + 128) / 256).unwrap_or(u8::MAX);
            }
        }
    }
    Ok(())
}

fn smooth_fixed(value: u64) -> u64 {
    let squared = value * value / 65_535;
    squared * (196_605 - 2 * value) / 65_535
}

fn lerp_fixed(left: u64, right: u64, fraction: u64) -> u64 {
    (left * (65_535 - fraction) + right * fraction + 32_767) / 65_535
}

fn macro_hash(x: usize, y: usize) -> u64 {
    let mut value = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9)
        ^ 0xD1B5_4A32_D192_ED03;
    value ^= value >> 30;
    value = value.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value ^= value >> 27;
    (value ^ (value >> 31)) & 0xFF
}

fn copy_region(
    source: &[u8],
    source_width: usize,
    origin_x: usize,
    origin_y: usize,
    size: usize,
) -> Result<Vec<u8>, TerrainError> {
    let output_len = size
        .checked_mul(size)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(TerrainError::TerrainTooLarge)?;
    let mut output = vec![0_u8; output_len];
    for row in 0..size {
        let source_start = origin_y
            .checked_add(row)
            .and_then(|y| y.checked_mul(source_width))
            .and_then(|offset| offset.checked_add(origin_x))
            .and_then(|offset| offset.checked_mul(4))
            .ok_or(TerrainError::TerrainTooLarge)?;
        let length = size.checked_mul(4).ok_or(TerrainError::TerrainTooLarge)?;
        let source_row = source
            .get(source_start..source_start + length)
            .ok_or(TerrainError::TerrainTooLarge)?;
        let target_start = row
            .checked_mul(length)
            .ok_or(TerrainError::TerrainTooLarge)?;
        output[target_start..target_start + length].copy_from_slice(source_row);
    }
    Ok(output)
}

fn copy_cell(
    destination: &mut [u8],
    destination_width: usize,
    origin_x: usize,
    origin_y: usize,
    size: usize,
    source: &[u8],
) -> Result<(), TerrainError> {
    let row_bytes = size.checked_mul(4).ok_or(TerrainError::TerrainTooLarge)?;
    for row in 0..size {
        let destination_start = origin_y
            .checked_add(row)
            .and_then(|y| y.checked_mul(destination_width))
            .and_then(|offset| offset.checked_add(origin_x))
            .and_then(|offset| offset.checked_mul(4))
            .ok_or(TerrainError::TerrainTooLarge)?;
        let source_start = row
            .checked_mul(row_bytes)
            .ok_or(TerrainError::TerrainTooLarge)?;
        let destination_row = destination
            .get_mut(destination_start..destination_start + row_bytes)
            .ok_or(TerrainError::TerrainTooLarge)?;
        let source_row = source
            .get(source_start..source_start + row_bytes)
            .ok_or(TerrainError::TerrainTooLarge)?;
        destination_row.copy_from_slice(source_row);
    }
    Ok(())
}

// Project-authored linear-light, alpha-aware mip generation for the modern viewer. The detail
// worker performs this alongside compositing so camera motion never downsamples textures on the
// presentation thread.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
pub(crate) fn generate_srgb_mips(
    width: u32,
    height: u32,
    base_rgba: &[u8],
) -> Result<Vec<TerrainMipLevel>, TerrainError> {
    let expected = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|texels| texels.checked_mul(4))
        .and_then(|bytes| usize::try_from(bytes).ok())
        .ok_or(TerrainError::TerrainTooLarge)?;
    if width == 0 || height == 0 || base_rgba.len() != expected {
        return Err(TerrainError::DimensionMismatch);
    }
    let mut source_width = width;
    let mut source_height = height;
    let mut levels = Vec::new();
    while source_width > 1 || source_height > 1 {
        let source = levels
            .last()
            .map_or(base_rgba, |level: &TerrainMipLevel| level.rgba.as_slice());
        let target_width = (source_width / 2).max(1);
        let target_height = (source_height / 2).max(1);
        let byte_count = u64::from(target_width)
            .checked_mul(u64::from(target_height))
            .and_then(|texels| texels.checked_mul(4))
            .and_then(|bytes| usize::try_from(bytes).ok())
            .ok_or(TerrainError::TerrainTooLarge)?;
        let mut target = vec![0_u8; byte_count];
        for target_y in 0..target_height {
            let row_start = target_y * source_height / target_height;
            let row_end = (target_y + 1) * source_height / target_height;
            for target_x in 0..target_width {
                let column_start = target_x * source_width / target_width;
                let column_end = (target_x + 1) * source_width / target_width;
                let mut alpha_sum = 0.0;
                let mut premultiplied = [0.0; 3];
                let mut sample_count = 0_u32;
                for source_y in row_start..row_end {
                    for source_x in column_start..column_end {
                        let offset = usize::try_from(
                            (u64::from(source_y) * u64::from(source_width) + u64::from(source_x))
                                * 4,
                        )
                        .map_err(|_| TerrainError::TerrainTooLarge)?;
                        let alpha = f32::from(source[offset + 3]) / 255.0;
                        alpha_sum += alpha;
                        for channel in 0..3 {
                            premultiplied[channel] +=
                                srgb_table()[usize::from(source[offset + channel])] * alpha;
                        }
                        sample_count += 1;
                    }
                }
                let target_offset = usize::try_from(
                    (u64::from(target_y) * u64::from(target_width) + u64::from(target_x)) * 4,
                )
                .map_err(|_| TerrainError::TerrainTooLarge)?;
                if alpha_sum > f32::EPSILON {
                    for channel in 0..3 {
                        target[target_offset + channel] =
                            linear_to_srgb(premultiplied[channel] / alpha_sum);
                    }
                }
                let alpha = alpha_sum / sample_count as f32;
                target[target_offset + 3] =
                    (alpha.mul_add(255.0, 0.5).floor() as u32).min(u32::from(u8::MAX)) as u8;
            }
        }
        levels.push(TerrainMipLevel {
            width: target_width,
            height: target_height,
            rgba: target,
        });
        source_width = target_width;
        source_height = target_height;
    }
    Ok(levels)
}

#[allow(clippy::cast_precision_loss)]
fn srgb_table() -> &'static [f32; 256] {
    static TABLE: OnceLock<[f32; 256]> = OnceLock::new();
    TABLE.get_or_init(|| {
        std::array::from_fn(|index| {
            let encoded = index as f32 / 255.0;
            if encoded <= 0.040_45 {
                encoded / 12.92
            } else {
                ((encoded + 0.055) / 1.055).powf(2.4)
            }
        })
    })
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn linear_to_srgb(linear: f32) -> u8 {
    let linear = linear.clamp(0.0, 1.0);
    let encoded = if linear <= 0.003_130_8 {
        linear * 12.92
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    };
    ((encoded.mul_add(255.0, 0.5).floor() as u32).min(u32::from(u8::MAX))) as u8
}

/// A bounded terrain staging or texture-layer failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerrainError {
    DimensionMismatch,
    EmptyTerrain,
    TerrainTooLarge,
    InvalidPixelsPerCell(u32),
    InvalidDetailViewport,
    InvalidTileIndex(i32),
    InvalidBlendIndex(i32),
    InvalidEdgeClass(i32),
    InvalidCliffIndex(i32),
    UnclassifiedTile(u32),
    MissingTexture(Vec<u8>),
    InvalidTextureSheet(Vec<u8>),
}

impl Display for TerrainError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::DimensionMismatch => {
                formatter.write_str("terrain height and blend dimensions do not match")
            }
            Self::EmptyTerrain => {
                formatter.write_str("terrain requires at least a two-by-two grid")
            }
            Self::TerrainTooLarge => {
                formatter.write_str("terrain geometry or texture exceeds limits")
            }
            Self::InvalidPixelsPerCell(value) => write!(
                formatter,
                "terrain pixels per cell must be a power of two from 1 through 32, got {value}"
            ),
            Self::InvalidDetailViewport => formatter
                .write_str("terrain detail viewport is empty, non-finite, or out of bounds"),
            Self::InvalidTileIndex(value) => {
                write!(formatter, "invalid terrain tile index {value}")
            }
            Self::InvalidBlendIndex(value) => {
                write!(formatter, "invalid terrain blend index {value}")
            }
            Self::InvalidEdgeClass(value) => {
                write!(formatter, "invalid terrain edge texture class {value}")
            }
            Self::InvalidCliffIndex(value) => {
                write!(formatter, "invalid terrain cliff index {value}")
            }
            Self::UnclassifiedTile(value) => write!(
                formatter,
                "terrain tile {value} does not belong to a declared texture class"
            ),
            Self::MissingTexture(name) => write!(
                formatter,
                "terrain texture is not loaded: {}",
                String::from_utf8_lossy(name)
            ),
            Self::InvalidTextureSheet(name) => write!(
                formatter,
                "terrain texture sheet is too small for its class: {}",
                String::from_utf8_lossy(name)
            ),
        }
    }
}

impl Error for TerrainError {}

#[cfg(test)]
mod tests {
    use super::{
        StagedTerrain, TerrainCompatibilityPolicy, TerrainDetailRequest, TerrainStagingOptions,
        TerrainVertex, VIRTUAL_CELL_BYTES, generate_srgb_mips, legacy_adjust_uv, mip_rgba,
        select_detail_pixels, terrain_viewer_vertex_bytes,
    };
    use crate::TextureResourceManager;
    use cic_formats::{MapLimits, decode_map_blend, decode_map_height, parse_map};

    #[test]
    fn source_mip_rounding_is_stable() {
        let source = vec![0, 0, 0, 255, 1, 1, 1, 255, 2, 2, 2, 255, 3, 3, 3, 255];
        assert_eq!(mip_rgba(&source, 2).expect("mip"), [2, 2, 2, 255]);
    }

    #[test]
    fn viewer_vertices_carry_smooth_height_field_normals() {
        let mut vertices = Vec::new();
        for y in [0.0_f32, 1.0, 2.0] {
            for x in [0.0_f32, 1.0, 2.0] {
                vertices.push(TerrainVertex {
                    position: [x, y, 2.0 * x + 3.0 * y],
                    uv: [x * 0.5, y * 0.5],
                });
            }
        }

        let bytes = terrain_viewer_vertex_bytes(&vertices, 3, 3).expect("viewer vertices");
        assert_eq!(bytes.len(), 9 * 32);
        let center = &bytes[4 * 32 + 20..4 * 32 + 32];
        let component = |offset| {
            f32::from_le_bytes(
                center[offset..offset + 4]
                    .try_into()
                    .expect("normal component"),
            )
        };
        let expected = [-2.0_f32, -3.0, 1.0].map(|value| value / 14.0_f32.sqrt());
        for (actual, expected) in [component(0), component(4), component(8)]
            .into_iter()
            .zip(expected)
        {
            assert!((actual - expected).abs() < 1.0e-6);
        }
    }

    #[test]
    fn viewport_density_scales_and_respects_texture_limits() {
        assert_eq!(
            select_detail_pixels([400, 400], [400, 400], [1_280, 800]),
            Some(8)
        );
        assert_eq!(
            select_detail_pixels([32, 24], [20, 16], [1_280, 800]),
            Some(32)
        );
        assert_eq!(
            select_detail_pixels([513, 513], [513, 513], [3_840, 2_160]),
            Some(4)
        );
    }

    #[test]
    fn resident_detail_covers_only_matching_or_lower_density_views() {
        let resident = TerrainDetailRequest {
            min: [8, 16],
            max: [80, 96],
            visible_min: [24, 32],
            visible_max: [64, 72],
            pixels_per_cell: 32,
        };
        let inside = TerrainDetailRequest {
            min: [16, 24],
            max: [72, 80],
            visible_min: [20, 28],
            visible_max: [68, 76],
            pixels_per_cell: 16,
        };
        let outside = TerrainDetailRequest {
            visible_max: [81, 76],
            ..inside
        };
        let denser = TerrainDetailRequest {
            pixels_per_cell: 32,
            ..inside
        };
        assert!(resident.covers(inside));
        assert!(!resident.covers(outside));
        assert!(resident.covers(denser));
        assert!(!inside.covers(denser));
    }

    #[test]
    fn modern_mips_filter_in_linear_light_and_preserve_alpha_coverage() {
        let opaque = [
            0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255, 255, 255, 255, 255,
        ];
        let opaque_mips = generate_srgb_mips(2, 2, &opaque).expect("opaque mips");
        assert_eq!((opaque_mips[0].width, opaque_mips[0].height), (1, 1));
        assert_eq!(opaque_mips[0].rgba, [188, 188, 188, 255]);

        let coverage = [255, 0, 0, 0, 0, 0, 255, 255, 255, 0, 0, 0, 255, 0, 0, 0];
        let coverage_mips = generate_srgb_mips(2, 2, &coverage).expect("coverage mips");
        assert_eq!(coverage_mips[0].rgba, [0, 0, 255, 64]);
    }

    #[test]
    fn stages_source_scaled_geometry_and_three_texture_layers() {
        let mut bytes = blend_fixture();
        let sentinel = bytes
            .windows(4)
            .position(|window| window == [0x00, 0x00, 0xDA, 0x7A])
            .expect("blend sentinel");
        bytes[sentinel - 4..sentinel].copy_from_slice(&(-1_i32).to_le_bytes());
        let limits = MapLimits::default();
        let map = parse_map(&bytes, "blend.map", limits).expect("MAP fixture");
        let height = decode_map_height(&map, limits).expect("height fixture");
        let blend = decode_map_blend(&map, &height, limits).expect("blend fixture");
        let mut textures = TextureResourceManager::default();
        textures
            .insert(b"Base", 128, 128, texture_sheet())
            .expect("texture sheet");

        let terrain = StagedTerrain::from_map(
            &height,
            &blend,
            &textures,
            TerrainStagingOptions::SOURCE_BACKGROUND,
        )
        .expect("staged terrain");

        assert_eq!((terrain.width(), terrain.height()), (8, 2));
        assert_eq!(terrain.cells().len(), 7);
        assert_eq!(terrain.vertices().len(), 16);
        assert_eq!(terrain.indices().len(), 42);
        assert_eq!(terrain.indices()[..6], [1, 8, 0, 1, 9, 8]);
        assert!(terrain.indices().chunks_exact(3).all(|triangle| {
            let &[first, second, third] = triangle else {
                unreachable!("exact terrain triangle chunks contain three indices")
            };
            let position = |index: u32| {
                terrain
                    .vertices()
                    .get(index as usize)
                    .expect("validated terrain index")
                    .position()
            };
            let first = position(first);
            let second = position(second);
            let third = position(third);
            (second[0] - first[0]) * (third[1] - first[1])
                - (second[1] - first[1]) * (third[0] - first[0])
                > 0.0
        }));
        assert_eq!(
            terrain.vertices()[9].position().map(f32::to_bits),
            [10.0_f32, 10.0, 90.0].map(f32::to_bits)
        );
        assert_eq!((terrain.texture_width(), terrain.texture_height()), (56, 8));
        assert_eq!(terrain.custom_edge_cell_count(), 0);
        assert!(terrain.cells()[5].primary().is_some());
        assert!(terrain.cells()[6].extra().is_some());

        let texture = terrain.texture_rgba();
        assert!(
            texture
                .chunks_exact(4)
                .any(|pixel| pixel == [255, 0, 0, 255])
        );
        assert!(texture.chunks_exact(4).any(|pixel| pixel[1] > pixel[2]));
        assert!(texture.chunks_exact(4).any(|pixel| pixel[2] > pixel[1]));
        let virtual_source = terrain.virtual_source().expect("virtual terrain source");
        assert_eq!(virtual_source.cell_size(), [7, 1]);
        assert_eq!(virtual_source.cell_bytes().len(), 7 * VIRTUAL_CELL_BYTES);
        assert_eq!(virtual_source.source_tile_grid_width(), 2);
        assert_eq!(virtual_source.edge_tile_grid_width(), 1);

        let (minimum, maximum) = terrain.bounds();
        let request = terrain
            .detail_request(
                [minimum[0], minimum[1]],
                [maximum[0], maximum[1]],
                [1_280, 800],
            )
            .expect("detail request");
        let detail = terrain.detail_patch(request).expect("detail patch");
        assert_eq!((detail.texture_width(), detail.texture_height()), (224, 32));
        assert_eq!(detail.index_count().expect("detail indices"), 42);
        assert_eq!(detail.edge_index_count().expect("detail edges"), 0);
        let full_detail = StagedTerrain::from_map(
            &height,
            &blend,
            &textures,
            TerrainStagingOptions::new(32).expect("detail resolution"),
        )
        .expect("full detail terrain");
        assert_eq!(detail.texture_rgba(), full_detail.texture_rgba());
    }

    #[test]
    fn stages_custom_edges_as_separate_geometry_and_texture() {
        let limits = MapLimits::default();
        let bytes = blend_fixture();
        let map = parse_map(&bytes, "blend.map", limits).expect("MAP fixture");
        let height = decode_map_height(&map, limits).expect("height fixture");
        let blend = decode_map_blend(&map, &height, limits).expect("blend fixture");
        let mut textures = TextureResourceManager::default();
        textures
            .insert(b"Base", 128, 128, texture_sheet())
            .expect("texture sheet");
        textures
            .insert(b"Shore", 64, 64, edge_sheet())
            .expect("edge sheet");

        let terrain = StagedTerrain::from_map(
            &height,
            &blend,
            &textures,
            TerrainStagingOptions::SOURCE_BACKGROUND,
        )
        .expect("staged terrain");

        assert_eq!(terrain.custom_edge_cell_count(), 1);
        assert_eq!(terrain.edge_indices().len(), 6);
        assert!(terrain.edge_texture_rgba().chunks_exact(4).any(|pixel| {
            pixel[0] == 240 && pixel[1] == 48 && pixel[2] == 192 && pixel[3] == 255
        }));
        assert!(
            terrain
                .edge_texture_rgba()
                .chunks_exact(4)
                .any(|pixel| pixel[3] == 0)
        );

        let (minimum, maximum) = terrain.bounds();
        let request = terrain
            .detail_request(
                [minimum[0], minimum[1]],
                [maximum[0], maximum[1]],
                [1_280, 800],
            )
            .expect("detail request");
        let detail = terrain.detail_patch(request).expect("detail patch");
        assert_eq!(detail.edge_index_count().expect("detail edges"), 6);
        assert!(
            detail
                .edge_texture_rgba()
                .chunks_exact(4)
                .any(|pixel| pixel[3] == 255)
        );
    }

    #[test]
    fn legacy_uv_adjustment_is_relative_to_the_selected_tile() {
        let adjusted = legacy_adjust_uv(
            [[0.25, 0.5], [0.5, 0.5], [0.5, 0.25], [0.25, 0.25]],
            [0, 40, 40, 40],
        );

        assert_eq!(adjusted[0][1].to_bits(), 0.875_f32.to_bits());
        assert_eq!(adjusted[1].map(f32::to_bits), [0.5, 0.5].map(f32::to_bits));
    }

    #[test]
    fn modern_macro_variation_is_deterministic_and_matches_streamed_detail() {
        let limits = MapLimits::default();
        let bytes = blend_fixture();
        let map = parse_map(&bytes, "blend.map", limits).expect("MAP fixture");
        let height = decode_map_height(&map, limits).expect("height fixture");
        let blend = decode_map_blend(&map, &height, limits).expect("blend fixture");
        let mut textures = TextureResourceManager::default();
        textures
            .insert(b"Base", 128, 128, texture_sheet())
            .expect("texture sheet");
        textures
            .insert(b"Shore", 64, 64, edge_sheet())
            .expect("edge sheet");
        let modern_options = TerrainStagingOptions::SOURCE_BACKGROUND
            .with_compatibility(TerrainCompatibilityPolicy::Modern);
        let modern = StagedTerrain::from_map(&height, &blend, &textures, modern_options)
            .expect("modern terrain");
        let repeated = StagedTerrain::from_map(&height, &blend, &textures, modern_options)
            .expect("repeated modern terrain");
        let legacy = StagedTerrain::from_map(
            &height,
            &blend,
            &textures,
            TerrainStagingOptions::SOURCE_BACKGROUND,
        )
        .expect("legacy terrain");
        assert_eq!(modern.texture_rgba(), repeated.texture_rgba());
        assert_ne!(modern.texture_rgba(), legacy.texture_rgba());

        let (minimum, maximum) = modern.bounds();
        let request = modern
            .detail_request(
                [minimum[0], minimum[1]],
                [maximum[0], maximum[1]],
                [1_280, 800],
            )
            .expect("detail request");
        let detail = modern.detail_patch(request).expect("modern detail");
        let full_detail = StagedTerrain::from_map(
            &height,
            &blend,
            &textures,
            TerrainStagingOptions::new(32)
                .expect("detail resolution")
                .with_compatibility(TerrainCompatibilityPolicy::Modern),
        )
        .expect("full modern detail");
        assert_eq!(detail.texture_rgba(), full_detail.texture_rgba());
    }

    fn blend_fixture() -> Vec<u8> {
        let hex = include_str!("../../cic-formats/tests/fixtures/blend.map.hex");
        let digits = hex
            .bytes()
            .filter(u8::is_ascii_hexdigit)
            .collect::<Vec<_>>();
        digits
            .chunks_exact(2)
            .map(|pair| {
                let pair = std::str::from_utf8(pair).expect("ASCII fixture");
                u8::from_str_radix(pair, 16).expect("valid fixture")
            })
            .collect()
    }

    fn texture_sheet() -> Vec<u8> {
        let mut rgba = vec![0_u8; 128 * 128 * 4];
        fill(&mut rgba, 0, 96, [255, 0, 0, 255]);
        fill(&mut rgba, 32, 96, [0, 255, 0, 255]);
        fill(&mut rgba, 0, 64, [0, 0, 255, 255]);
        fill(&mut rgba, 32, 64, [255, 255, 0, 255]);
        rgba
    }

    fn edge_sheet() -> Vec<u8> {
        let mut rgba = vec![0_u8; 64 * 64 * 4];
        for y in 0..64 {
            for x in 0..64 {
                let local_x = x % 16;
                let color = if local_x < 4 {
                    [255, 255, 255, 255]
                } else if local_x < 12 {
                    [240, 48, 192, 255]
                } else {
                    [0, 0, 0, 255]
                };
                let offset = (y * 64 + x) * 4;
                rgba[offset..offset + 4].copy_from_slice(&color);
            }
        }
        rgba
    }

    fn fill(rgba: &mut [u8], origin_x: usize, origin_y: usize, color: [u8; 4]) {
        for y in origin_y..origin_y + 32 {
            for x in origin_x..origin_x + 32 {
                let offset = (y * 128 + x) * 4;
                rgba[offset..offset + 4].copy_from_slice(&color);
            }
        }
    }
}
