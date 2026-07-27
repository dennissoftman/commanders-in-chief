//! One bounded, viewport-derived terrain texture request.
//!
//! Lifted out of the terrain pipeline so the virtual-page bookkeeping in
//! [`crate::terrain_virtual`] depends on the *shape* of a detail request rather than on a terrain
//! implementation. That inversion is what lets the residency logic be tested — and ported — without
//! a GPU, a heightfield, or a decoder anywhere in the picture.

/// A rectangular region of terrain requested at a particular texel density.
///
/// `minimum`/`maximum` bound the region to stage, which is deliberately larger than
/// `visible_minimum`/`visible_maximum`: staging a margin beyond what is on screen is what keeps a
/// pan from exposing unstaged terrain for a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerrainDetailRequest {
    min: [u32; 2],
    max: [u32; 2],
    visible_min: [u32; 2],
    visible_max: [u32; 2],
    pixels_per_cell: u32,
}

impl TerrainDetailRequest {
    /// Builds a request from an explicit staged region, visible region, and density.
    #[must_use]
    pub const fn new(
        min: [u32; 2],
        max: [u32; 2],
        visible_min: [u32; 2],
        visible_max: [u32; 2],
        pixels_per_cell: u32,
    ) -> Self {
        Self {
            min,
            max,
            visible_min,
            visible_max,
            pixels_per_cell,
        }
    }

    /// Builds a request whose staged and visible regions coincide.
    #[must_use]
    pub const fn uniform(min: [u32; 2], max: [u32; 2], pixels_per_cell: u32) -> Self {
        Self::new(min, max, min, max, pixels_per_cell)
    }

    /// Returns the inclusive lower corner of the staged region.
    #[must_use]
    pub const fn minimum(self) -> [u32; 2] {
        self.min
    }

    /// Returns the exclusive upper corner of the staged region.
    #[must_use]
    pub const fn maximum(self) -> [u32; 2] {
        self.max
    }

    /// Returns the inclusive lower corner of the visible region.
    #[must_use]
    pub const fn visible_minimum(self) -> [u32; 2] {
        self.visible_min
    }

    /// Returns the exclusive upper corner of the visible region.
    #[must_use]
    pub const fn visible_maximum(self) -> [u32; 2] {
        self.visible_max
    }

    /// Returns the requested texels per terrain cell.
    #[must_use]
    pub const fn density(self) -> u32 {
        self.pixels_per_cell
    }
}
