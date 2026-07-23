// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Renderer-only rectangle visualization of a decoded WND document.
//!
//! This stages each window's stored `SCREENRECT` as a flat colored quad, in source
//! (depth-first) order, proving the immutable `cic_formats::WndDocument` value can drive a
//! deterministic renderer capture. It has no bearing on the eventual retained UI runtime:
//! parent-relative positioning, classic/Modern scaling policy, images, text, and
//! gadget-specific visuals are explicitly deferred to later gates. Every window rectangle is
//! treated as an absolute screen-space rectangle in the root window's declared creation
//! resolution, which also becomes the capture's canvas size.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

use cic_formats::{WndDocument, WndWindow};

const MAX_WND_VERTICES: usize = 65_536;
const MAX_WND_INDICES: usize = 98_304;
const WINDOW_COLORS: [[f32; 4]; 6] = [
    [0.20, 0.55, 1.0, 0.55],
    [0.95, 0.55, 0.15, 0.55],
    [0.25, 0.85, 0.45, 0.55],
    [0.85, 0.25, 0.75, 0.55],
    [0.95, 0.85, 0.20, 0.55],
    [0.30, 0.85, 0.85, 0.55],
];

/// One clip-space vertex for a staged WND window rectangle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WndQuadVertex {
    position: [f32; 3],
    color: [f32; 4],
}

impl WndQuadVertex {
    #[must_use]
    pub const fn position(self) -> [f32; 3] {
        self.position
    }

    #[must_use]
    pub const fn color(self) -> [f32; 4] {
        self.color
    }
}

/// Bounded, source-ordered quad geometry for one decoded WND document.
#[derive(Debug, Clone, PartialEq)]
pub struct StagedWndScene {
    canvas: [u32; 2],
    vertices: Vec<WndQuadVertex>,
    indices: Vec<u32>,
    window_count: usize,
}

impl StagedWndScene {
    /// Stages every window's rectangle from `document`, in source (depth-first) order.
    ///
    /// The first top-level window's declared creation resolution becomes the capture canvas
    /// size; every rectangle is mapped into that space directly, without parent-relative
    /// offsetting or any scaling policy.
    ///
    /// # Errors
    ///
    /// Returns a structured error when the document has no windows, the canvas resolution is
    /// non-positive, or staged geometry would exceed the explicit presentation limits.
    pub fn from_document(document: &WndDocument) -> Result<Self, WndSceneError> {
        let root = document.windows().first().ok_or(WndSceneError::NoWindows)?;
        let (canvas_width, canvas_height) = root.rect().creation_resolution();
        let canvas = [
            u32::try_from(canvas_width).map_err(|_| WndSceneError::InvalidCanvas)?,
            u32::try_from(canvas_height).map_err(|_| WndSceneError::InvalidCanvas)?,
        ];
        if canvas[0] == 0 || canvas[1] == 0 {
            return Err(WndSceneError::InvalidCanvas);
        }
        let mut staged = Self {
            canvas,
            vertices: Vec::new(),
            indices: Vec::new(),
            window_count: 0,
        };
        for window in document.windows() {
            staged.push_window(window)?;
        }
        Ok(staged)
    }

    /// Returns the capture canvas size (the root window's creation resolution).
    #[must_use]
    pub const fn canvas(&self) -> [u32; 2] {
        self.canvas
    }

    /// Returns staged quad vertices, four per window, in source order.
    #[must_use]
    pub fn vertices(&self) -> &[WndQuadVertex] {
        &self.vertices
    }

    /// Returns staged triangle indices, six per window, in source order.
    #[must_use]
    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    /// Returns the total number of staged windows (top-level and nested).
    #[must_use]
    pub const fn window_count(&self) -> usize {
        self.window_count
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

    fn push_window(&mut self, window: &WndWindow) -> Result<(), WndSceneError> {
        self.reserve_geometry(4, 6)?;
        let color = WINDOW_COLORS[self.window_count % WINDOW_COLORS.len()];
        let base =
            u32::try_from(self.vertices.len()).map_err(|_| WndSceneError::GeometryTooLarge)?;
        let (upper_left_x, upper_left_y) = window.rect().upper_left();
        let (bottom_right_x, bottom_right_y) = window.rect().bottom_right();
        let top_left = self.ndc(upper_left_x, upper_left_y);
        let bottom_left = self.ndc(upper_left_x, bottom_right_y);
        let bottom_right = self.ndc(bottom_right_x, bottom_right_y);
        let top_right = self.ndc(bottom_right_x, upper_left_y);
        self.vertices.extend([
            quad_vertex(top_left, color),
            quad_vertex(bottom_left, color),
            quad_vertex(bottom_right, color),
            quad_vertex(top_right, color),
        ]);
        self.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        self.window_count = self
            .window_count
            .checked_add(1)
            .ok_or(WndSceneError::GeometryTooLarge)?;
        for child in window.children() {
            self.push_window(child)?;
        }
        Ok(())
    }

    #[allow(clippy::cast_possible_truncation)]
    fn ndc(&self, x: i32, y: i32) -> [f32; 2] {
        let ndc_x = (f64::from(x) / f64::from(self.canvas[0]) * 2.0 - 1.0) as f32;
        let ndc_y = (1.0 - f64::from(y) / f64::from(self.canvas[1]) * 2.0) as f32;
        [ndc_x, ndc_y]
    }

    fn reserve_geometry(
        &mut self,
        vertex_count: usize,
        index_count: usize,
    ) -> Result<(), WndSceneError> {
        let vertices = self
            .vertices
            .len()
            .checked_add(vertex_count)
            .ok_or(WndSceneError::GeometryTooLarge)?;
        let indices = self
            .indices
            .len()
            .checked_add(index_count)
            .ok_or(WndSceneError::GeometryTooLarge)?;
        if vertices > MAX_WND_VERTICES || indices > MAX_WND_INDICES {
            return Err(WndSceneError::GeometryTooLarge);
        }
        self.vertices.reserve(vertex_count);
        self.indices.reserve(index_count);
        Ok(())
    }
}

fn quad_vertex(xy: [f32; 2], color: [f32; 4]) -> WndQuadVertex {
    WndQuadVertex {
        position: [xy[0], xy[1], 0.0],
        color,
    }
}

/// A structured WND scene staging failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WndSceneError {
    /// The document declared no windows to stage (never reachable through
    /// [`cic_formats::parse_wnd`], which itself rejects window-less documents; retained here
    /// as a defensive bound on this staging API's own precondition).
    NoWindows,
    /// The root window's creation resolution is not a positive, representable size.
    InvalidCanvas,
    /// Staged geometry would exceed the explicit presentation limits.
    GeometryTooLarge,
}

impl Display for WndSceneError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoWindows => formatter.write_str("WND document has no windows to stage"),
            Self::InvalidCanvas => {
                formatter.write_str("WND root creation resolution is not positive")
            }
            Self::GeometryTooLarge => {
                formatter.write_str("WND scene exceeds staged geometry limits")
            }
        }
    }
}

impl Error for WndSceneError {}

#[cfg(test)]
mod tests {
    use cic_formats::{WndLimits, parse_wnd};

    use super::StagedWndScene;

    #[test]
    fn stages_every_window_rect_in_source_order() {
        let bytes = b"FILE_VERSION = 1\n\
WINDOW\n\
  WINDOWTYPE = PUSHBUTTON;\n\
  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 100 50 CREATIONRESOLUTION: 800 600;\n\
  CHILD\n\
    WINDOW\n\
      WINDOWTYPE = STATICTEXT;\n\
      SCREENRECT = UPPERLEFT: 10 10 BOTTOMRIGHT: 90 40 CREATIONRESOLUTION: 800 600;\n\
    END\n\
  ENDALLCHILDREN\n\
END\n";
        let document = parse_wnd(bytes, WndLimits::default()).expect("valid WND");
        let scene = StagedWndScene::from_document(&document).expect("staged scene");
        assert_eq!(scene.canvas(), [800, 600]);
        assert_eq!(scene.window_count(), 2);
        assert_eq!(scene.vertices().len(), 8);
        assert_eq!(scene.indices().len(), 12);
        assert_eq!(
            scene.vertices()[0].position()[2].to_bits(),
            0.0_f32.to_bits()
        );
    }
}
