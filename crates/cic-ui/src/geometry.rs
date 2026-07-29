//! Rectangles, and the one conversion that separates logical units from physical pixels.
//!
//! # Why two unit systems rather than one
//!
//! A layout file is authored in **logical units** and drawn in **physical pixels**, and the ratio
//! between them is a property of the display rather than of the layout. Authoring in pixels is the
//! charter's named failure mode: a 32-pixel button is comfortable at 96 DPI and a smear at 200, so a
//! layout that names pixels is a layout that is correct on exactly one monitor.
//!
//! Everything in [`crate::layout`] is therefore logical, everything out of [`crate::solve`] is
//! physical, and [`Viewport`] is the only place the factor between them appears.
//!
//! # Why edges are snapped and sizes are not
//!
//! A resolved rectangle almost never lands on whole pixels — a third of a 1000-pixel column is
//! 333.33 — and drawing on a half pixel is what makes a border look grey instead of black. The naive
//! fix, rounding position and size separately, *introduces* a defect: two adjacent boxes at
//! `x = 333.33, w = 333.33` and `x = 666.67, w = 333.33` round to `333 + 333 = 666` against `667`,
//! leaving a one-pixel seam the author did not write.
//!
//! Snapping the **edges** instead cannot do that. Each edge rounds to the same integer no matter
//! which rectangle asks, so a shared edge stays shared and the width is whatever the difference
//! turns out to be. Adjacency survives; an exact width does not, and of the two only adjacency is
//! visible.

use serde::{Deserialize, Serialize};

/// An axis-aligned rectangle, with the origin at the top left and Y increasing downward.
///
/// Y-down because this is a screen-space type and every input event, glyph raster, and swapchain
/// image this has to agree with is already Y-down. The world-space parts of the engine are Z-up and
/// that is not a contradiction: they describe different spaces and never share a rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rect {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Extent along X. Never negative once solved.
    pub width: f32,
    /// Extent along Y. Never negative once solved.
    pub height: f32,
}

impl Rect {
    /// A rectangle at the origin with no extent.
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 0.0,
    };

    /// Builds a rectangle from a position and an extent.
    #[must_use]
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// The right edge.
    #[must_use]
    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    /// The bottom edge.
    #[must_use]
    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    /// Whether a point falls inside, treating the left and top edges as inside and the right and
    /// bottom as outside.
    ///
    /// Half-open on purpose. Two rectangles sharing an edge would otherwise both claim a click on
    /// it, and which one won would depend on iteration order — a hit test that is not a function of
    /// the geometry alone.
    #[must_use]
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }

    /// Shrinks by a per-side inset, clamping to zero rather than inverting.
    ///
    /// Clamping rather than allowing a negative extent because padding larger than its box is an
    /// authoring mistake whose sensible reading is "nothing fits", and a negative width would
    /// otherwise propagate into every child as a nonsense available size.
    #[must_use]
    pub fn inset(&self, insets: Insets) -> Self {
        let width = (self.width - insets.left - insets.right).max(0.0);
        let height = (self.height - insets.top - insets.bottom).max(0.0);
        Self {
            x: self.x + insets.left,
            y: self.y + insets.top,
            width,
            height,
        }
    }

    /// Whether this rectangle covers no pixels at all.
    ///
    /// What a consumer checks before drawing: a clip that has shrunk to nothing means everything inside
    /// it is off screen, and issuing the draw anyway is work with no result.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }

    /// Moves the rectangle without changing its extent.
    ///
    /// What a scroll offset does to everything inside a scrollable container.
    #[must_use]
    pub fn translated(&self, x: f32, y: f32) -> Self {
        Self {
            x: self.x + x,
            y: self.y + y,
            width: self.width,
            height: self.height,
        }
    }

    /// The overlap with another rectangle, collapsed to zero extent when they do not overlap.
    ///
    /// Zero rather than negative for the reason [`Self::inset`] clamps: a negative extent propagates
    /// into whatever consumes it as a nonsense size, a long way from the disjoint pair that caused it.
    /// Nested clips compose by intersection, so this has to stay total.
    #[must_use]
    pub fn intersection(&self, other: Self) -> Self {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        Self {
            x: left,
            y: top,
            width: (right - left).max(0.0),
            height: (bottom - top).max(0.0),
        }
    }

    /// Rounds every edge to a whole pixel, keeping shared edges shared.
    ///
    /// See the module documentation for why this rounds edges rather than position and size.
    #[must_use]
    pub fn snapped(&self) -> Self {
        let left = self.x.round();
        let top = self.y.round();
        let right = self.right().round();
        let bottom = self.bottom().round();
        Self {
            x: left,
            y: top,
            width: (right - left).max(0.0),
            height: (bottom - top).max(0.0),
        }
    }
}

/// Per-side spacing, in whichever unit system the rectangle it applies to is in.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Insets {
    /// Space at the left edge.
    #[serde(default)]
    pub left: f32,
    /// Space at the top edge.
    #[serde(default)]
    pub top: f32,
    /// Space at the right edge.
    #[serde(default)]
    pub right: f32,
    /// Space at the bottom edge.
    #[serde(default)]
    pub bottom: f32,
}

impl Insets {
    /// No spacing on any side.
    pub const ZERO: Self = Self {
        left: 0.0,
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
    };

    /// The same spacing on all four sides.
    #[must_use]
    pub const fn uniform(amount: f32) -> Self {
        Self {
            left: amount,
            top: amount,
            right: amount,
            bottom: amount,
        }
    }

    /// Total spacing along X.
    #[must_use]
    pub fn horizontal(&self) -> f32 {
        self.left + self.right
    }

    /// Total spacing along Y.
    #[must_use]
    pub fn vertical(&self) -> f32 {
        self.top + self.bottom
    }

    /// Multiplies every side, for converting logical spacing to physical.
    #[must_use]
    pub fn scaled(&self, factor: f32) -> Self {
        Self {
            left: self.left * factor,
            top: self.top * factor,
            right: self.right * factor,
            bottom: self.bottom * factor,
        }
    }
}

/// The surface a layout is solved against: how many physical pixels there are, and how many of them
/// one logical unit is worth.
///
/// Held together rather than passed as two numbers because they are only meaningful as a pair, and
/// because the renderer already learned this lesson once — a size passed separately from the target
/// it described is how a frame ended up one resize behind.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    width: u32,
    height: u32,
    scale: f32,
}

/// Largest viewport edge accepted, matching the renderer's capture bound so a layout cannot be
/// solved for a surface no pass could allocate.
const MAX_VIEWPORT_DIMENSION: u32 = 8_192;

/// Bounds on the scale factor.
///
/// A scale of zero collapses every rectangle to nothing and a negative one mirrors the interface, so
/// both are rejected rather than clamped: they can only come from a bad query or a bad calculation,
/// and silently substituting 1.0 would hide which.
const MIN_SCALE: f32 = 0.25;
const MAX_SCALE: f32 = 8.0;

impl Viewport {
    /// Builds a viewport from a physical size and a scale factor.
    ///
    /// # Errors
    ///
    /// Returns [`ViewportError`] when either dimension is zero or past
    /// [`MAX_VIEWPORT_DIMENSION`], or when the scale is not finite and inside
    /// `[MIN_SCALE, MAX_SCALE]`.
    pub fn new(width: u32, height: u32, scale: f32) -> Result<Self, ViewportError> {
        if width == 0 || height == 0 {
            return Err(ViewportError::EmptySurface { width, height });
        }
        if width > MAX_VIEWPORT_DIMENSION || height > MAX_VIEWPORT_DIMENSION {
            return Err(ViewportError::SurfaceTooLarge {
                width,
                height,
                limit: MAX_VIEWPORT_DIMENSION,
            });
        }
        if !scale.is_finite() || !(MIN_SCALE..=MAX_SCALE).contains(&scale) {
            return Err(ViewportError::Scale { scale });
        }
        Ok(Self {
            width,
            height,
            scale,
        })
    }

    /// Physical width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Physical height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Physical pixels per logical unit.
    #[must_use]
    pub const fn scale(&self) -> f32 {
        self.scale
    }

    /// The whole surface as a physical rectangle.
    // Both dimensions are bounded by 8192, which every integer below is exactly representable in
    // `f32`, so this conversion cannot lose anything.
    #[allow(clippy::cast_precision_loss)]
    #[must_use]
    pub fn bounds(&self) -> Rect {
        Rect::new(0.0, 0.0, self.width as f32, self.height as f32)
    }

    /// Converts a logical measurement to physical pixels.
    #[must_use]
    pub fn to_physical(&self, logical: f32) -> f32 {
        logical * self.scale
    }
}

/// Why a viewport could not be built.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ViewportError {
    /// A dimension was zero.
    EmptySurface {
        /// Requested width.
        width: u32,
        /// Requested height.
        height: u32,
    },
    /// A dimension exceeded the bound.
    SurfaceTooLarge {
        /// Requested width.
        width: u32,
        /// Requested height.
        height: u32,
        /// The largest accepted edge.
        limit: u32,
    },
    /// The scale factor was not usable.
    Scale {
        /// The rejected factor.
        scale: f32,
    },
}

impl std::fmt::Display for ViewportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptySurface { width, height } => {
                write!(formatter, "a viewport cannot be {width}x{height}")
            }
            Self::SurfaceTooLarge {
                width,
                height,
                limit,
            } => write!(
                formatter,
                "a viewport of {width}x{height} exceeds the {limit} pixel limit"
            ),
            Self::Scale { scale } => write!(
                formatter,
                "a scale factor of {scale} is outside [{MIN_SCALE}, {MAX_SCALE}]"
            ),
        }
    }
}

impl std::error::Error for ViewportError {}

#[cfg(test)]
mod tests {
    // Every float compared below is either an integer-valued pixel coordinate produced by snapping or
    // an exactly-representable product of small binary fractions, so exact comparison is the assertion
    // being made rather than a tolerance nobody chose.
    #![allow(clippy::float_cmp)]

    use super::{Insets, Rect, Viewport, ViewportError};

    #[test]
    fn an_inset_shrinks_from_every_side() {
        let rect = Rect::new(10.0, 20.0, 100.0, 50.0);
        let inset = rect.inset(Insets {
            left: 5.0,
            top: 4.0,
            right: 3.0,
            bottom: 2.0,
        });
        assert_eq!(inset, Rect::new(15.0, 24.0, 92.0, 44.0));
    }

    #[test]
    fn padding_larger_than_its_box_yields_nothing_rather_than_a_negative_extent() {
        // A negative width would otherwise reach every child as an available size, where it reads as
        // a layout bug a long way from the padding that caused it.
        let rect = Rect::new(0.0, 0.0, 10.0, 10.0);
        let inset = rect.inset(Insets::uniform(20.0));
        assert_eq!(inset.width, 0.0);
        assert_eq!(inset.height, 0.0);
    }

    #[test]
    fn snapping_keeps_adjacent_edges_shared() {
        // The whole reason edges are snapped rather than positions and sizes. Three columns of a
        // 1000-pixel row land on thirds; rounding each width independently loses a pixel between
        // the second and third, which draws as a seam.
        let first = Rect::new(0.0, 0.0, 333.333_34, 10.0).snapped();
        let second = Rect::new(333.333_34, 0.0, 333.333_34, 10.0).snapped();
        let third = Rect::new(666.666_7, 0.0, 333.333_34, 10.0).snapped();
        assert_eq!(first.right(), second.x, "first and second must touch");
        assert_eq!(second.right(), third.x, "second and third must touch");
        assert_eq!(third.right(), 1000.0, "the row must still end where it did");
    }

    #[test]
    fn a_hit_test_is_half_open_so_a_shared_edge_belongs_to_one_box() {
        let left = Rect::new(0.0, 0.0, 10.0, 10.0);
        let right = Rect::new(10.0, 0.0, 10.0, 10.0);
        assert!(left.contains(0.0, 0.0), "the left edge is inside");
        assert!(!left.contains(10.0, 0.0), "the right edge is outside");
        assert!(right.contains(10.0, 0.0), "and belongs to the next box");
        // Exactly one of them claims the shared edge, whatever order they are tested in.
        assert_ne!(left.contains(10.0, 5.0), right.contains(10.0, 5.0));
    }

    #[test]
    fn a_viewport_rejects_a_surface_or_scale_it_cannot_mean() {
        assert_eq!(
            Viewport::new(0, 100, 1.0),
            Err(ViewportError::EmptySurface {
                width: 0,
                height: 100
            })
        );
        assert!(matches!(
            Viewport::new(100_000, 100, 1.0),
            Err(ViewportError::SurfaceTooLarge { .. })
        ));
        assert!(matches!(
            Viewport::new(100, 100, 0.0),
            Err(ViewportError::Scale { .. })
        ));
        assert!(matches!(
            Viewport::new(100, 100, f32::NAN),
            Err(ViewportError::Scale { .. })
        ));
        assert!(matches!(
            Viewport::new(100, 100, -1.0),
            Err(ViewportError::Scale { .. })
        ));
    }

    #[test]
    fn an_intersection_collapses_rather_than_inverting_when_there_is_no_overlap() {
        // Nested clips compose by intersection, so a disjoint pair has to yield "nothing" and not a
        // negative extent that reaches a consumer as a nonsense size.
        let left = Rect::new(0.0, 0.0, 10.0, 10.0);
        let right = Rect::new(20.0, 0.0, 10.0, 10.0);
        let empty = left.intersection(right);
        // The axis that does not overlap collapses; the other keeps its span, which is why a consumer
        // asks `is_empty` rather than comparing an extent to zero itself.
        assert_eq!(empty.width, 0.0);
        assert_eq!(empty.height, 10.0);
        assert!(empty.is_empty());
        assert!(
            Rect::new(20.0, 20.0, 10.0, 10.0)
                .intersection(left)
                .is_empty(),
            "disjoint on both axes is empty too"
        );
        // Overlapping keeps the shared region, whichever way round it is asked.
        let overlap = Rect::new(5.0, 5.0, 10.0, 10.0);
        assert_eq!(left.intersection(overlap), Rect::new(5.0, 5.0, 5.0, 5.0));
        assert_eq!(overlap.intersection(left), Rect::new(5.0, 5.0, 5.0, 5.0));
        assert!(!left.intersection(overlap).is_empty());
    }

    #[test]
    fn translating_moves_a_rectangle_without_resizing_it() {
        let rect = Rect::new(10.0, 20.0, 30.0, 40.0);
        assert_eq!(
            rect.translated(0.0, -5.0),
            Rect::new(10.0, 15.0, 30.0, 40.0)
        );
    }

    #[test]
    fn scale_is_the_only_place_logical_becomes_physical() {
        let viewport = Viewport::new(1920, 1200, 1.5).expect("valid viewport");
        assert_eq!(viewport.to_physical(32.0), 48.0);
        assert_eq!(viewport.bounds(), Rect::new(0.0, 0.0, 1920.0, 1200.0));
        assert_eq!(Insets::uniform(8.0).scaled(1.5), Insets::uniform(12.0));
    }
}
