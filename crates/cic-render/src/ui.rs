// Commanders in Chief
// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only
//
// This staging layer is original project presentation. It consumes the renderer-neutral frames
// `cic-ui` produces and emits batched geometry; it derives no algorithm from the legacy Direct3D UI
// renderer. The draw order it preserves comes from `cic-ui`, which documents its own provenance.

//! Batched staging of a retained-UI frame into renderer-ready geometry.
//!
//! A [`cic_ui::UiFrame`] is an ordered list of instructions. Staging turns it into vertices,
//! indices, and batches, breaking a batch only when the bound texture page or the scissor rectangle
//! changes, so submission stays in the frame's stable order while still batching adjacent work.
//!
//! Text is deliberately not shaped here. A frame's text runs are staged as [`StagedUiText`] and
//! shaped at render time by the font set the caller supplies, because deterministic captures must
//! never fall back to host fonts.

use cic_ui::{UiFrame, UiFrameItem, UiRect};

/// One staged vertex: position in viewport pixels, texture coordinate, straight RGBA, and whether
/// the fragment samples its page.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiVertex {
    position: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
    textured: u32,
}

impl UiVertex {
    /// The byte stride one vertex occupies in a vertex buffer.
    ///
    /// Two floats of position, two of texture coordinate, four of colour, and one 32-bit flag.
    pub const STRIDE: u64 = 36;

    /// Returns the position in viewport pixels.
    #[must_use]
    pub const fn position(self) -> [f32; 2] {
        self.position
    }

    /// Returns the straight RGBA colour.
    #[must_use]
    pub const fn color(self) -> [f32; 4] {
        self.color
    }

    /// Returns whether the fragment samples its bound page.
    #[must_use]
    pub const fn is_textured(self) -> bool {
        self.textured == 1
    }
}

/// One uploaded RGBA8 texture page that mapped images address regions of.
///
/// Pages arrive already decoded, so this crate never learns an image file format; `cic-tools` owns
/// decoding and hands over pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiTexturePage {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl UiTexturePage {
    /// Creates a page from tightly packed, non-premultiplied RGBA8 rows.
    ///
    /// # Errors
    ///
    /// Returns [`UiStagingError::InvalidViewport`] for a zero extent, or
    /// [`UiStagingError::PageSizeMismatch`] when the byte count is not `width * height * 4`.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, UiStagingError> {
        if width == 0 || height == 0 {
            return Err(UiStagingError::InvalidViewport { width, height });
        }
        let expected = usize::try_from(width)
            .ok()
            .and_then(|width| usize::try_from(height).ok().map(|height| width * height))
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(UiStagingError::PageSizeMismatch {
                expected: 0,
                actual: rgba.len(),
            })?;
        if rgba.len() != expected {
            return Err(UiStagingError::PageSizeMismatch {
                expected,
                actual: rgba.len(),
            });
        }
        Ok(Self {
            width,
            height,
            rgba,
        })
    }

    /// Returns the page width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the page height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Returns the packed RGBA8 rows.
    #[must_use]
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }
}

/// A named texture page region a mapped image resolves to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiImageBinding {
    /// Which uploaded page holds the image.
    pub page: usize,
    /// Normalized `[left, top, right, bottom]` coordinates within that page.
    pub uv: [f32; 4],
}

/// One contiguous run of indices sharing a page and scissor rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiBatch {
    first_index: u32,
    index_count: u32,
    page: Option<usize>,
    scissor: Option<UiRect>,
}

impl UiBatch {
    /// Returns the first index this batch draws.
    #[must_use]
    pub const fn first_index(self) -> u32 {
        self.first_index
    }

    /// Returns how many indices this batch draws.
    #[must_use]
    pub const fn index_count(self) -> u32 {
        self.index_count
    }

    /// Returns the page this batch samples, absent for colour-only geometry.
    #[must_use]
    pub const fn page(self) -> Option<usize> {
        self.page
    }

    /// Returns the scissor rectangle in effect, absent when unclipped.
    #[must_use]
    pub const fn scissor(self) -> Option<UiRect> {
        self.scissor
    }
}

/// One text run staged for shaping at render time.
#[derive(Debug, Clone, PartialEq)]
pub struct StagedUiText {
    /// The rectangle the run is laid out in, in viewport pixels.
    pub rect: UiRect,
    /// The text to shape. Already resolved from a label by the caller.
    pub text: String,
    /// The requested font family, empty when the control declares none.
    pub family: String,
    /// The requested point size, already scaled for the viewport by the caller.
    pub size: f32,
    /// Whether a bold face is requested.
    pub bold: bool,
    /// Straight RGBA colour.
    pub color: [u8; 4],
    /// The scissor rectangle in effect, absent when unclipped.
    pub scissor: Option<UiRect>,
}

/// Why one frame item could not be staged as authored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiStagingDiagnosticKind {
    /// A quad named a mapped image the caller could not bind. A visible placeholder was staged.
    UnboundImage {
        /// The image name exactly as the layout spelled it.
        name: Box<str>,
    },
    /// A `PopClip` arrived with no matching `PushClip`. It was ignored.
    UnbalancedPopClip,
    /// A frame ended with clips still open. They were closed.
    UnclosedClips {
        /// How many remained open.
        depth: usize,
    },
    /// A text run was staged as a placeholder bar because no font set can shape it.
    UnshapeableText {
        /// The text that would have been shaped.
        text: Box<str>,
    },
}

/// One non-fatal staging observation, in frame order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiStagingDiagnostic {
    item: usize,
    kind: UiStagingDiagnosticKind,
}

impl UiStagingDiagnostic {
    /// Returns the frame-item index the observation applies to.
    #[must_use]
    pub const fn item(&self) -> usize {
        self.item
    }

    /// Returns the observation detail.
    #[must_use]
    pub const fn kind(&self) -> &UiStagingDiagnosticKind {
        &self.kind
    }
}

/// Whether staging shapes text or substitutes a visible placeholder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UiTextPolicy {
    /// Stage text runs for shaping. The caller must supply a font set at render time.
    #[default]
    Shape,
    /// Stage a placeholder bar per run instead, for a capture with no font set available.
    Placeholder,
}

/// Explicit bounds for staging one frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiStagingLimits {
    /// Maximum quads staged from one frame.
    pub max_quads: usize,
    /// Maximum clip nesting depth.
    pub max_clip_depth: usize,
    /// Maximum text runs staged from one frame.
    pub max_text_runs: usize,
}

impl Default for UiStagingLimits {
    fn default() -> Self {
        Self {
            max_quads: 65_536,
            max_clip_depth: 64,
            max_text_runs: 4_096,
        }
    }
}

/// A staging failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiStagingError {
    /// A viewport extent was not positive.
    InvalidViewport {
        /// The rejected width.
        width: u32,
        /// The rejected height.
        height: u32,
    },
    /// The frame stages more quads than [`UiStagingLimits::max_quads`].
    TooManyQuads {
        /// The configured limit.
        limit: usize,
    },
    /// The frame nests clips deeper than [`UiStagingLimits::max_clip_depth`].
    ClipTooDeep {
        /// The configured limit.
        limit: usize,
    },
    /// The frame stages more text runs than [`UiStagingLimits::max_text_runs`].
    TooManyTextRuns {
        /// The configured limit.
        limit: usize,
    },
    /// A texture page's byte count does not match its declared dimensions.
    PageSizeMismatch {
        /// The byte count the dimensions require.
        expected: usize,
        /// The byte count supplied.
        actual: usize,
    },
}

impl std::fmt::Display for UiStagingError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidViewport { width, height } => {
                write!(formatter, "viewport {width}x{height} is not positive")
            }
            Self::TooManyQuads { limit } => {
                write!(formatter, "frame exceeds the {limit}-quad limit")
            }
            Self::ClipTooDeep { limit } => {
                write!(formatter, "frame exceeds the {limit}-clip-depth limit")
            }
            Self::TooManyTextRuns { limit } => {
                write!(formatter, "frame exceeds the {limit}-text-run limit")
            }
            Self::PageSizeMismatch { expected, actual } => write!(
                formatter,
                "texture page needs {expected} bytes but {actual} were supplied"
            ),
        }
    }
}

impl std::error::Error for UiStagingError {}

/// The colour a missing mapped image is drawn in, so an unresolved resource is visible rather than
/// silently absent.
pub const UI_PLACEHOLDER_COLOR: [u8; 4] = [255, 0, 255, 160];

/// One staged retained-UI frame.
#[derive(Debug, Clone, PartialEq)]
pub struct StagedUiFrame {
    canvas: [u32; 2],
    vertices: Vec<UiVertex>,
    indices: Vec<u32>,
    batches: Vec<UiBatch>,
    text: Vec<StagedUiText>,
    diagnostics: Vec<UiStagingDiagnostic>,
}

impl StagedUiFrame {
    /// Stages one frame for an explicit viewport.
    ///
    /// `bind_image` resolves a mapped-image name to an uploaded page region. Returning `None` stages
    /// a visible placeholder and records a diagnostic, which is the ordinary path for the images
    /// retail names but never defines.
    ///
    /// # Errors
    ///
    /// Returns a structured error for a non-positive viewport or any exceeded staging limit.
    #[expect(
        clippy::too_many_lines,
        reason = "one arm per frame-item kind; splitting the walk would separate clip tracking from                   the items that depend on it"
    )]
    pub fn from_frame(
        frame: &UiFrame,
        canvas: [u32; 2],
        text_policy: UiTextPolicy,
        limits: UiStagingLimits,
        bind_image: &dyn Fn(&str) -> Option<UiImageBinding>,
    ) -> Result<Self, UiStagingError> {
        let [width, height] = canvas;
        if width == 0 || height == 0 {
            return Err(UiStagingError::InvalidViewport { width, height });
        }
        let mut staged = Self {
            canvas,
            vertices: Vec::new(),
            indices: Vec::new(),
            batches: Vec::new(),
            text: Vec::new(),
            diagnostics: Vec::new(),
        };
        let mut clips: Vec<UiRect> = Vec::new();
        let mut quads = 0_usize;

        for (index, item) in frame.items().iter().enumerate() {
            let scissor = clips.last().copied();
            match item {
                UiFrameItem::PushClip { rect } => {
                    if clips.len() >= limits.max_clip_depth {
                        return Err(UiStagingError::ClipTooDeep {
                            limit: limits.max_clip_depth,
                        });
                    }
                    // Nested clips intersect, so an inner region can never draw outside an outer one.
                    clips.push(scissor.map_or(*rect, |outer| intersect(outer, *rect)));
                }
                UiFrameItem::PopClip => {
                    if clips.pop().is_none() {
                        staged.diagnostics.push(UiStagingDiagnostic {
                            item: index,
                            kind: UiStagingDiagnosticKind::UnbalancedPopClip,
                        });
                    }
                }
                UiFrameItem::Quad {
                    rect,
                    image,
                    color,
                    border_color,
                    border,
                    ..
                } => {
                    let binding = image.as_deref().map(|name| (name, bind_image(name)));
                    let (page, uv, fill) = match binding {
                        Some((name, None)) => {
                            staged.diagnostics.push(UiStagingDiagnostic {
                                item: index,
                                kind: UiStagingDiagnosticKind::UnboundImage {
                                    name: name.to_owned().into_boxed_str(),
                                },
                            });
                            (None, [0.0; 4], UI_PLACEHOLDER_COLOR)
                        }
                        Some((_, Some(binding))) => (
                            Some(binding.page),
                            binding.uv,
                            color.map_or([255, 255, 255, 255], channels),
                        ),
                        None => (None, [0.0; 4], color.map_or([0, 0, 0, 0], channels)),
                    };
                    if fill[3] > 0 {
                        quads += 1;
                        staged.push_quad(*rect, uv, fill, page, scissor, quads, limits)?;
                    }
                    // The original draws a border only for a control declaring `BORDER`; a border
                    // colour on its own is inert, and most retail controls carry one.
                    let edge_color = border_color
                        .map(channels)
                        .filter(|_| *border)
                        .filter(|color| color[3] > 0);
                    if let Some(edge_color) = edge_color {
                        for edge in border_edges(*rect) {
                            quads += 1;
                            staged.push_quad(
                                edge, [0.0; 4], edge_color, None, scissor, quads, limits,
                            )?;
                        }
                    }
                }
                UiFrameItem::Text(run) => {
                    if staged.text.len() >= limits.max_text_runs {
                        return Err(UiStagingError::TooManyTextRuns {
                            limit: limits.max_text_runs,
                        });
                    }
                    let color = run.color.map_or([255, 255, 255, 255], channels);
                    let text = if run.masked {
                        // A secret field renders one mask glyph per character, never its contents.
                        "*".repeat(run.label.chars().count())
                    } else {
                        run.label.clone()
                    };
                    match text_policy {
                        UiTextPolicy::Shape => staged.text.push(StagedUiText {
                            rect: run.rect,
                            text,
                            family: run
                                .font
                                .as_ref()
                                .map(|(name, _, _)| name.clone())
                                .unwrap_or_default(),
                            #[expect(
                                clippy::cast_precision_loss,
                                reason = "point sizes are small integers"
                            )]
                            size: run.font.as_ref().map_or(12.0, |(_, size, _)| *size as f32),
                            bold: run.font.as_ref().is_some_and(|(_, _, bold)| *bold),
                            color,
                            scissor,
                        }),
                        UiTextPolicy::Placeholder => {
                            staged.diagnostics.push(UiStagingDiagnostic {
                                item: index,
                                kind: UiStagingDiagnosticKind::UnshapeableText {
                                    text: text.into_boxed_str(),
                                },
                            });
                            quads += 1;
                            staged.push_quad(
                                placeholder_bar(run.rect),
                                [0.0; 4],
                                color,
                                None,
                                scissor,
                                quads,
                                limits,
                            )?;
                        }
                    }
                }
            }
        }

        if !clips.is_empty() {
            staged.diagnostics.push(UiStagingDiagnostic {
                item: frame.items().len(),
                kind: UiStagingDiagnosticKind::UnclosedClips { depth: clips.len() },
            });
        }
        Ok(staged)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one quad's complete bound state; grouping it into a struct would only move the                   same fields behind another name"
    )]
    fn push_quad(
        &mut self,
        rect: UiRect,
        uv: [f32; 4],
        color: [u8; 4],
        page: Option<usize>,
        scissor: Option<UiRect>,
        quads: usize,
        limits: UiStagingLimits,
    ) -> Result<(), UiStagingError> {
        if quads > limits.max_quads {
            return Err(UiStagingError::TooManyQuads {
                limit: limits.max_quads,
            });
        }
        if rect.width <= 0 || rect.height <= 0 {
            return Ok(());
        }
        #[expect(
            clippy::cast_precision_loss,
            reason = "viewport pixel coordinates are small integers"
        )]
        let (left, top, right, bottom) = (
            rect.x as f32,
            rect.y as f32,
            (rect.x + rect.width) as f32,
            (rect.y + rect.height) as f32,
        );
        let normalized = [
            f32::from(color[0]) / 255.0,
            f32::from(color[1]) / 255.0,
            f32::from(color[2]) / 255.0,
            f32::from(color[3]) / 255.0,
        ];
        let textured = u32::from(page.is_some());
        let base =
            u32::try_from(self.vertices.len()).map_err(|_| UiStagingError::TooManyQuads {
                limit: limits.max_quads,
            })?;
        for (position, coordinate) in [
            ([left, top], [uv[0], uv[1]]),
            ([right, top], [uv[2], uv[1]]),
            ([right, bottom], [uv[2], uv[3]]),
            ([left, bottom], [uv[0], uv[3]]),
        ] {
            self.vertices.push(UiVertex {
                position,
                uv: coordinate,
                color: normalized,
                textured,
            });
        }
        let first_index =
            u32::try_from(self.indices.len()).map_err(|_| UiStagingError::TooManyQuads {
                limit: limits.max_quads,
            })?;
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        // Extend the open batch when nothing bound changed, so adjacent quads submit as one draw.
        match self.batches.last_mut() {
            Some(batch) if batch.page == page && batch.scissor == scissor => {
                batch.index_count += 6;
            }
            _ => self.batches.push(UiBatch {
                first_index,
                index_count: 6,
                page,
                scissor,
            }),
        }
        Ok(())
    }

    /// Returns the target size in pixels.
    #[must_use]
    pub const fn canvas(&self) -> [u32; 2] {
        self.canvas
    }

    /// Returns the staged vertices.
    #[must_use]
    pub fn vertices(&self) -> &[UiVertex] {
        &self.vertices
    }

    /// Returns the staged indices.
    #[must_use]
    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    /// Returns the batches in submission order.
    #[must_use]
    pub fn batches(&self) -> &[UiBatch] {
        &self.batches
    }

    /// Returns the text runs awaiting shaping, in frame order.
    #[must_use]
    pub fn text(&self) -> &[StagedUiText] {
        &self.text
    }

    /// Returns every non-fatal staging observation, in frame order.
    #[must_use]
    pub fn diagnostics(&self) -> &[UiStagingDiagnostic] {
        &self.diagnostics
    }

    /// Returns the vertex buffer bytes in staged order.
    #[must_use]
    pub fn vertex_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.vertices.len() * 36);
        for vertex in &self.vertices {
            for value in vertex.position {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            for value in vertex.uv {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            for value in vertex.color {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            bytes.extend_from_slice(&vertex.textured.to_le_bytes());
        }
        bytes
    }

    /// Returns the index buffer bytes in staged order.
    #[must_use]
    pub fn index_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.indices.len() * 4);
        for index in &self.indices {
            bytes.extend_from_slice(&index.to_le_bytes());
        }
        bytes
    }
}

fn channels(color: cic_formats::WndColor) -> [u8; 4] {
    color.channels()
}

/// Returns the four one-pixel edges of a rectangle, in top, bottom, left, right order.
const fn border_edges(rect: UiRect) -> [UiRect; 4] {
    [
        UiRect {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: 1,
        },
        UiRect {
            x: rect.x,
            y: rect.y + rect.height - 1,
            width: rect.width,
            height: 1,
        },
        UiRect {
            x: rect.x,
            y: rect.y,
            width: 1,
            height: rect.height,
        },
        UiRect {
            x: rect.x + rect.width - 1,
            y: rect.y,
            width: 1,
            height: rect.height,
        },
    ]
}

/// Returns a two-pixel bar along a control's baseline, standing in for unshaped text.
const fn placeholder_bar(rect: UiRect) -> UiRect {
    UiRect {
        x: rect.x + 2,
        y: rect.y + rect.height / 2,
        width: if rect.width > 4 { rect.width - 4 } else { 0 },
        height: 2,
    }
}

fn intersect(outer: UiRect, inner: UiRect) -> UiRect {
    let left = outer.x.max(inner.x);
    let top = outer.y.max(inner.y);
    let right = (outer.x + outer.width).min(inner.x + inner.width);
    let bottom = (outer.y + outer.height).min(inner.y + inner.height);
    UiRect {
        x: left,
        y: top,
        width: (right - left).max(0),
        height: (bottom - top).max(0),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        StagedUiFrame, UiImageBinding, UiStagingDiagnosticKind, UiStagingError, UiStagingLimits,
        UiTextPolicy,
    };
    use cic_formats::{WndLimits, parse_wnd};
    use cic_ui::{UiClipPolicy, UiLayout, UiLimits, UiPresentation, UiScalePolicy, UiViewport};

    fn layout(width: i32, height: i32) -> UiLayout {
        let source = r#"FILE_VERSION = 2;
STARTLAYOUTBLOCK
  LAYOUTINIT = "[None]";
  LAYOUTUPDATE = "[None]";
  LAYOUTSHUTDOWN = "[None]";
ENDLAYOUTBLOCK
WINDOW
  WINDOWTYPE = USER;
  SCREENRECT = UPPERLEFT: 0 0,
               BOTTOMRIGHT: 400 300,
               CREATIONRESOLUTION: 800 600;
  NAME = "SynthMenu.wnd:PanelSynth";
  STATUS = ENABLED;
  ENABLEDDRAWDATA = IMAGE: SynthPanel, COLOR: 10 20 30 255, BORDERCOLOR: 1 2 3 255,
    IMAGE: NoImage, COLOR: 0 0 0 0, BORDERCOLOR: 0 0 0 0,
    IMAGE: NoImage, COLOR: 0 0 0 0, BORDERCOLOR: 0 0 0 0,
    IMAGE: NoImage, COLOR: 0 0 0 0, BORDERCOLOR: 0 0 0 0,
    IMAGE: NoImage, COLOR: 0 0 0 0, BORDERCOLOR: 0 0 0 0,
    IMAGE: NoImage, COLOR: 0 0 0 0, BORDERCOLOR: 0 0 0 0,
    IMAGE: NoImage, COLOR: 0 0 0 0, BORDERCOLOR: 0 0 0 0,
    IMAGE: NoImage, COLOR: 0 0 0 0, BORDERCOLOR: 0 0 0 0,
    IMAGE: NoImage, COLOR: 0 0 0 0, BORDERCOLOR: 0 0 0 0;
CHILD
WINDOW
  WINDOWTYPE = PUSHBUTTON;
  SCREENRECT = UPPERLEFT: 20 20,
               BOTTOMRIGHT: 120 60,
               CREATIONRESOLUTION: 800 600;
  NAME = "SynthMenu.wnd:ButtonSynth";
  STATUS = ENABLED;
  TEXT = "GUI:SynthButton";
  FONT = NAME: "Synth Sans", SIZE: 11, BOLD: 0;
  TEXTCOLOR = ENABLED: 255 255 255 255, ENABLEDBORDER: 0 0 0 255,
              DISABLED: 128 128 128 255, DISABLEDBORDER: 0 0 0 255,
              HILITE: 255 255 0 255, HILITEBORDER: 0 0 0 255;
  ENABLEDDRAWDATA = IMAGE: SynthMissing, COLOR: 255 255 255 255, BORDERCOLOR: 0 0 0 0,
    IMAGE: NoImage, COLOR: 0 0 0 0, BORDERCOLOR: 0 0 0 0,
    IMAGE: NoImage, COLOR: 0 0 0 0, BORDERCOLOR: 0 0 0 0,
    IMAGE: NoImage, COLOR: 0 0 0 0, BORDERCOLOR: 0 0 0 0,
    IMAGE: NoImage, COLOR: 0 0 0 0, BORDERCOLOR: 0 0 0 0,
    IMAGE: NoImage, COLOR: 0 0 0 0, BORDERCOLOR: 0 0 0 0,
    IMAGE: NoImage, COLOR: 0 0 0 0, BORDERCOLOR: 0 0 0 0,
    IMAGE: NoImage, COLOR: 0 0 0 0, BORDERCOLOR: 0 0 0 0,
    IMAGE: NoImage, COLOR: 0 0 0 0, BORDERCOLOR: 0 0 0 0;
END
ENDALLCHILDREN
END
"#;
        let document = parse_wnd(source.as_bytes(), WndLimits::default()).expect("decode layout");
        let viewport = UiViewport::new(width, height).expect("positive viewport");
        UiLayout::instantiate(
            &document,
            UiPresentation::new(viewport, UiScalePolicy::Classic),
            UiLimits::default(),
        )
        .expect("instantiate layout")
    }

    fn bind_synth(name: &str) -> Option<UiImageBinding> {
        (name == "SynthPanel").then_some(UiImageBinding {
            page: 0,
            uv: [0.0, 0.0, 0.5, 0.25],
        })
    }

    #[test]
    fn staging_batches_by_page_and_reports_an_unbound_image() {
        let layout = layout(800, 600);
        let frame = layout.frame(UiClipPolicy::None);
        let staged = StagedUiFrame::from_frame(
            &frame,
            [800, 600],
            UiTextPolicy::Shape,
            UiStagingLimits::default(),
            &bind_synth,
        )
        .expect("stage frame");

        // The panel binds its page, so its quad is textured; the button's image does not resolve, so
        // a placeholder is staged instead and reported.
        assert!(staged.vertices()[0].is_textured());
        assert_eq!(staged.batches()[0].page(), Some(0));
        assert!(staged.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic.kind(),
            UiStagingDiagnosticKind::UnboundImage { name } if &**name == "SynthMissing"
        )));
        // The panel declares a border, so four one-pixel edges follow its fill in the same batch
        // family; the untextured edges break the batch away from the textured page.
        assert!(staged.batches().len() >= 2);
        assert_eq!(
            staged.indices().len(),
            staged
                .batches()
                .iter()
                .map(|batch| batch.index_count() as usize)
                .sum::<usize>()
        );
        // One text run is staged for shaping rather than turned into geometry.
        assert_eq!(staged.text().len(), 1);
        assert_eq!(staged.text()[0].text, "GUI:SynthButton");
        assert_eq!(staged.text()[0].family, "Synth Sans");
        assert_eq!(staged.text()[0].color, [255, 255, 255, 255]);
    }

    #[test]
    fn clips_intersect_and_become_batch_boundaries() {
        let layout = layout(800, 600);
        let frame = layout.frame(UiClipPolicy::ClipToParent);
        let staged = StagedUiFrame::from_frame(
            &frame,
            [800, 600],
            UiTextPolicy::Shape,
            UiStagingLimits::default(),
            &bind_synth,
        )
        .expect("stage clipped frame");
        let scissored: Vec<_> = staged
            .batches()
            .iter()
            .filter_map(|batch| batch.scissor())
            .collect();
        assert!(!scissored.is_empty());
        // Every clipped batch sits inside the panel, which is the only clip in this layout.
        for rect in scissored {
            assert!(rect.x >= 0 && rect.y >= 0);
            assert!(rect.width <= 400 && rect.height <= 300);
        }
        assert!(!staged.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic.kind(),
            UiStagingDiagnosticKind::UnbalancedPopClip
                | UiStagingDiagnosticKind::UnclosedClips { .. }
        )));
    }

    #[test]
    fn placeholder_text_policy_stages_a_visible_bar_and_a_diagnostic() {
        let layout = layout(800, 600);
        let frame = layout.frame(UiClipPolicy::None);
        let shaped = StagedUiFrame::from_frame(
            &frame,
            [800, 600],
            UiTextPolicy::Shape,
            UiStagingLimits::default(),
            &bind_synth,
        )
        .expect("stage frame");
        let placeholder = StagedUiFrame::from_frame(
            &frame,
            [800, 600],
            UiTextPolicy::Placeholder,
            UiStagingLimits::default(),
            &bind_synth,
        )
        .expect("stage frame");
        assert!(placeholder.text().is_empty());
        assert!(placeholder.vertices().len() > shaped.vertices().len());
        assert!(placeholder.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic.kind(),
            UiStagingDiagnosticKind::UnshapeableText { .. }
        )));
    }

    #[test]
    fn staging_is_deterministic_and_bounded() {
        let layout = layout(1920, 1080);
        let frame = layout.frame(UiClipPolicy::None);
        let first = StagedUiFrame::from_frame(
            &frame,
            [1920, 1080],
            UiTextPolicy::Shape,
            UiStagingLimits::default(),
            &bind_synth,
        )
        .expect("stage frame");
        let second = StagedUiFrame::from_frame(
            &frame,
            [1920, 1080],
            UiTextPolicy::Shape,
            UiStagingLimits::default(),
            &bind_synth,
        )
        .expect("stage frame");
        assert_eq!(first, second);
        assert_eq!(first.vertex_bytes(), second.vertex_bytes());
        assert_eq!(
            first.vertex_bytes().len(),
            first.vertices().len() * usize::try_from(super::UiVertex::STRIDE).expect("stride")
        );

        assert_eq!(
            StagedUiFrame::from_frame(
                &frame,
                [0, 600],
                UiTextPolicy::Shape,
                UiStagingLimits::default(),
                &bind_synth
            ),
            Err(UiStagingError::InvalidViewport {
                width: 0,
                height: 600
            })
        );
        let limits = UiStagingLimits {
            max_quads: 1,
            ..UiStagingLimits::default()
        };
        assert_eq!(
            StagedUiFrame::from_frame(&frame, [800, 600], UiTextPolicy::Shape, limits, &bind_synth),
            Err(UiStagingError::TooManyQuads { limit: 1 })
        );
    }
}
