// Commanders in Chief
// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only
//
// The batching scheme, vertex format, clip handling, placeholder policy, and capture path are
// original project presentation. The per-family draw-data composition is not: it is derived from
// Electronic Arts' GPL-3.0 source release, GeneralsGameCode revision
// 9f7abb866f5afd446db14149979e744c7216baaf. Each family's entry indices come from its accessors in
// `Core/GameEngine/Include/GameClient/Gadget<Family>.h` and its geometry from the matching
// `Core/GameEngineDevice/Source/W3DDevice/GameClient/GUI/Gadget/W3D<Family>.cpp`:
//
// - Push button: `GadgetPushButton.h` fixes the unselected art at entries 0, 5, 6 and the pushed
//   art at 1, 3, 4; `W3DPushButton.cpp` (`W3DGadgetPushButtonImageDrawThree`) fixes the repeating
//   centre, the partial final piece, the ends-last order, and the `centerWidth <= 0` branch.
// - Radio button: `GadgetRadioButton.h` fixes 0, 1, 2 per slot and the selected triple at hilite
//   3, 4, 5; `W3DRadioButton.cpp` (`W3DGadgetRadioButtonImageDraw`) fixes the geometry, including
//   its selected-first state order and its right end running to the control's own right edge.
// - Check box: `GadgetCheckBox.h` fixes the unchecked box at 1 and the checked box at 2;
//   `W3DCheckBox.cpp` (`W3DGadgetCheckBoxImageDraw`) draws only that box, three pixels down and
//   six shorter than the control, with the background image commented out in the source itself.
// - Text entry: `GadgetTextEntry.h` fixes left 0, right 1, centre 2, small centre 3;
//   `W3DTextEntry.cpp` (`W3DGadgetTextEntryImageDraw`) fixes the whole-centre loop and the
//   deliberately overrunning small-centre loop that the right end then covers.
// - Vertical slider: `GadgetSlider.h` fixes top 0, bottom 1, centre 2, small centre 3;
//   `W3DVerticalSlider.cpp` (`W3DGadgetVerticalSliderImageDraw`) fixes the stacked geometry and the
//   half-and-half branch taken when the two ends alone are taller than the control.
// - Horizontal slider: `W3DHorizontalSlider.cpp` (`W3DGadgetHorizontalSliderImageDraw`) draws a row
//   of tick squares whose art comes from fixed slots — fill and blank from the disabled slot,
//   highlight from the hilite slot — whatever the control's own state, scaled by the display width
//   against a 800-pixel reference.
// - Progress bar: `GadgetProgressBar.h` fixes background left 0, right 1, centre 2 and bar right 5,
//   centre 6; `W3DProgressBar.cpp` (`W3DGadgetProgressBarImageDraw`) fixes the three-piece
//   background and the bar inset ten pixels horizontally and five vertically.
// - Tab control: `GadgetTabControl.h` fixes the background at 0 and the eight tabs at 1 through 8;
//   `W3DTabControl.cpp` (`W3DGadgetTabControlImageDraw`) and `GadgetTabControl.cpp`
//   (`GadgetTabControlComputeTabRegion`) fix the strip's origin and per-tab state selection.
// - Every other family — list box, combo box, static text, plain windows — draws one stretched
//   image from entry 0, which is `W3DGameWindow.cpp`'s `W3DGameWinDefaultDraw` and the identical
//   openings of `W3DListBox.cpp`, `W3DComboBox.cpp`, and `W3DStaticText.cpp`.
//
// A partial repeating piece trims texture coordinates where the source sets a clip region, which
// samples the same pixels without a state change. The untinted-image rule follows the same files:
// `winDrawImage` takes no colour argument, so a slot's `COLOR` belongs to the colour-only draw path.
// No C++ was copied or translated line by line. The draw order this layer preserves comes from
// `cic-ui`, which documents its own provenance.

//! Batched staging of a retained-UI frame into renderer-ready geometry.
//!
//! A [`cic_ui::UiFrame`] is an ordered list of instructions. Staging turns it into vertices,
//! indices, and batches, breaking a batch only when the bound texture page or the scissor rectangle
//! changes, so submission stays in the frame's stable order while still batching adjacent work.
//!
//! Text is deliberately not shaped here. A frame's text runs are staged as [`StagedUiText`] and
//! shaped at render time by the font set the caller supplies, because deterministic captures must
//! never fall back to host fonts.

use cic_formats::WndDrawDataSlot;
use cic_ui::{
    UiControlFamily, UiDrawState, UiFrame, UiFrameItem, UiRect, UiSlotImages, UiTextAlign,
};

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
    /// The image's own size in pixels, which the source reads to size a composed piece.
    pub size: [i32; 2],
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
    /// Where the run sits inside its rectangle.
    pub align: UiTextAlign,
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
    /// A control takes an image-drawing path and carries slot art, but declares nothing at the
    /// indices its own family reads. Nothing was staged, which is the source's own early return,
    /// so this records art the layout will never show.
    UncomposedArt {
        /// The family whose indices found nothing.
        family: &'static str,
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
                    slot,
                    color,
                    border_color,
                    border,
                    images,
                    image_offset,
                    family,
                    state,
                    image_draw,
                    ..
                } => {
                    if *image_draw {
                        // One extra quad of headroom, so a repeating piece that would overrun the
                        // limit is staged and reported rather than silently truncated.
                        let budget = limits.max_quads.saturating_sub(quads).saturating_add(1);
                        let composed = compose(
                            *family,
                            *rect,
                            *image_offset,
                            images,
                            *state,
                            *slot,
                            canvas,
                            budget,
                            bind_image,
                        );
                        match composed {
                            // A composed piece draws untinted: the source's `winDrawImage` takes no
                            // colour, and a slot's `COLOR` belongs to the colour-only fill path.
                            // Retail leaves an unused red in that field beside a valid image, so
                            // multiplying by it turns every textured control red.
                            Composed::Pieces(pieces) => {
                                for (piece_rect, binding) in pieces {
                                    quads += 1;
                                    staged.push_piece(
                                        piece_rect,
                                        &binding,
                                        [255, 255, 255, 255],
                                        scissor,
                                        quads,
                                        limits,
                                    )?;
                                }
                            }
                            Composed::Unbound(name) => {
                                staged.diagnostics.push(UiStagingDiagnostic {
                                    item: index,
                                    kind: UiStagingDiagnosticKind::UnboundImage { name },
                                });
                                quads += 1;
                                staged.push_quad(
                                    *rect,
                                    [0.0; 4],
                                    UI_PLACEHOLDER_COLOR,
                                    None,
                                    scissor,
                                    quads,
                                    limits,
                                )?;
                            }
                            // The source's draw procedures return early when their own indices hold
                            // nothing, so nothing draws. Art at other indices would never appear,
                            // which is worth reporting even though the result is correct.
                            Composed::Undeclared => {
                                if !images.is_empty() {
                                    staged.diagnostics.push(UiStagingDiagnostic {
                                        item: index,
                                        kind: UiStagingDiagnosticKind::UncomposedArt {
                                            family: family.name(),
                                        },
                                    });
                                }
                            }
                        }
                    } else {
                        let fill = color.map_or([0, 0, 0, 0], channels);
                        if fill[3] > 0 {
                            quads += 1;
                            staged
                                .push_quad(*rect, [0.0; 4], fill, None, scissor, quads, limits)?;
                        }
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
                            align: run.align,
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

    fn push_piece(
        &mut self,
        rect: UiRect,
        piece: &UiImageBinding,
        tint: [u8; 4],
        scissor: Option<UiRect>,
        quads: usize,
        limits: UiStagingLimits,
    ) -> Result<(), UiStagingError> {
        self.push_quad(
            rect,
            piece.uv,
            tint,
            Some(piece.page),
            scissor,
            quads,
            limits,
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one quad's complete bound state; a struct would only move the same fields behind                   another name"
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

/// One composed piece: where it draws, and the page region it samples.
type Piece = (UiRect, UiImageBinding);

/// What one family's composition produced for one control.
enum Composed {
    /// Pieces to stage, in the order the source draws them, so a later piece covers an earlier one.
    Pieces(Vec<Piece>),
    /// A declared mapped image the caller could not bind. Geometry depends on the bound piece's
    /// pixel size, so nothing could be composed and the control stages a placeholder instead.
    Unbound(Box<str>),
    /// The family's own indices declare no image, which is where each source draw procedure
    /// returns early. Nothing draws.
    Undeclared,
}

/// Resolves a control's declared image names into bound page regions, remembering the first name
/// that failed to bind so an unresolved resource stays visible.
struct SlotBinder<'a> {
    images: &'a UiSlotImages,
    bind: &'a dyn Fn(&str) -> Option<UiImageBinding>,
    unbound: Option<Box<str>>,
}

impl<'a> SlotBinder<'a> {
    fn new(images: &'a UiSlotImages, bind: &'a dyn Fn(&str) -> Option<UiImageBinding>) -> Self {
        Self {
            images,
            bind,
            unbound: None,
        }
    }

    /// Returns one slot entry's bound region, absent when the entry declares nothing or its name
    /// does not bind.
    fn image(&mut self, slot: WndDrawDataSlot, index: usize) -> Option<UiImageBinding> {
        let name = self.images.image(slot, index)?;
        let binding = (self.bind)(name);
        if binding.is_none() && self.unbound.is_none() {
            self.unbound = Some(name.into());
        }
        binding
    }

    /// Turns a family's collected pieces into a result, distinguishing "the layout declares nothing
    /// here" from "the layout declares art that would not bind".
    fn finish(self, pieces: Option<Vec<Piece>>) -> Composed {
        match (pieces, self.unbound) {
            (Some(pieces), _) if !pieces.is_empty() => Composed::Pieces(pieces),
            (_, Some(name)) => Composed::Unbound(name),
            _ => Composed::Undeclared,
        }
    }
}

/// The display width the source's horizontal slider scales its tick squares against.
///
/// `W3DHorizontalSlider.cpp` divides `TheDisplay->getWidth()` by `DEFAULT_DISPLAY_WIDTH`.
const DEFAULT_DISPLAY_WIDTH: f32 = 800.0;

/// Composes one control's slot art into the pieces its family draws.
///
/// Every branch reproduces the matching `W3DGadget*ImageDraw`, including which slot each piece
/// comes from: those are not always the control's current state slot, and the header comment
/// records the source for each.
#[expect(
    clippy::too_many_arguments,
    reason = "one control's complete composition input; a struct would only move the same fields                   behind another name"
)]
fn compose(
    family: UiControlFamily,
    rect: UiRect,
    image_offset: (i32, i32),
    images: &UiSlotImages,
    state: UiDrawState,
    slot: WndDrawDataSlot,
    canvas: [u32; 2],
    budget: usize,
    bind: &dyn Fn(&str) -> Option<UiImageBinding>,
) -> Composed {
    let mut binder = SlotBinder::new(images, bind);
    let pieces = match family {
        UiControlFamily::PushButton => push_button(&mut binder, rect, image_offset, state, slot),
        UiControlFamily::RadioButton => radio_button(&mut binder, rect, image_offset, state, slot),
        UiControlFamily::CheckBox => check_box(&mut binder, rect, image_offset, state, slot),
        UiControlFamily::TextEntry => text_entry(&mut binder, rect, image_offset, slot, budget),
        UiControlFamily::VerticalSlider => {
            vertical_slider(&mut binder, rect, image_offset, slot, budget)
        }
        UiControlFamily::HorizontalSlider => {
            horizontal_slider(&mut binder, rect, state, canvas, budget)
        }
        UiControlFamily::ProgressBar => {
            progress_bar(&mut binder, rect, image_offset, state, slot, budget)
        }
        UiControlFamily::TabControl(geometry) => {
            tab_control(&mut binder, rect, image_offset, geometry, slot)
        }
        UiControlFamily::Simple => stretched(&mut binder, rect, image_offset, slot),
    };
    binder.finish(pieces)
}

/// `W3DGameWinDefaultDraw`'s image path, shared by list boxes, combo boxes, and static text: one
/// entry-0 image covering the control from its image offset.
fn stretched(
    binder: &mut SlotBinder<'_>,
    rect: UiRect,
    image_offset: (i32, i32),
    slot: WndDrawDataSlot,
) -> Option<Vec<Piece>> {
    let image = binder.image(slot, 0)?;
    Some(vec![(offset_rect(rect, image_offset), image)])
}

/// `W3DGadgetPushButtonImageDrawThree`.
fn push_button(
    binder: &mut SlotBinder<'_>,
    rect: UiRect,
    image_offset: (i32, i32),
    state: UiDrawState,
    slot: WndDrawDataSlot,
) -> Option<Vec<Piece>> {
    // `GadgetPushButton.h`: unselected art is left 0, middle 5, right 6; pushed art is 1, 3, 4.
    let (left, centre, right) = if state.selected { (1, 3, 4) } else { (0, 5, 6) };
    let left = binder.image(slot, left)?;
    let centre = binder.image(slot, centre)?;
    let right = binder.image(slot, right)?;
    let (x_offset, y_offset) = image_offset;
    let top = rect.y + y_offset;
    let left_end = rect.x + left.size[0] + x_offset;
    let right_start = rect.x + rect.width - right.size[0] + x_offset;

    // The source's `centerWidth <= 0` branch: with no room between the ends, each takes half.
    if right_start - left_end <= 0 {
        let split = rect.x + x_offset + rect.width / 2;
        return Some(vec![
            (
                UiRect {
                    x: rect.x + x_offset,
                    y: top,
                    width: split - (rect.x + x_offset),
                    height: rect.height + y_offset,
                },
                left,
            ),
            (
                UiRect {
                    x: split,
                    y: top,
                    width: rect.x + rect.width - split,
                    height: rect.height,
                },
                right,
            ),
        ]);
    }

    // The centre's pieces are a pixel taller per image offset than the ends, which is the source's
    // own `end.y = start.y + size.y + yOffset` against the ends' plain `size.y`.
    let mut pieces = trimmed_row(left_end, right_start, top, rect.height + y_offset, centre);
    pieces.push((
        UiRect {
            x: rect.x + x_offset,
            y: top,
            width: left.size[0],
            height: rect.height,
        },
        left,
    ));
    pieces.push((
        UiRect {
            x: right_start,
            y: top,
            width: right.size[0],
            height: rect.height,
        },
        right,
    ));
    Some(pieces)
}

/// `W3DGadgetRadioButtonImageDraw`.
fn radio_button(
    binder: &mut SlotBinder<'_>,
    rect: UiRect,
    image_offset: (i32, i32),
    state: UiDrawState,
    slot: WndDrawDataSlot,
) -> Option<Vec<Piece>> {
    // A selected radio button reads the hilite slot's second triple whatever its enablement,
    // because the source tests `WIN_STATE_SELECTED` before it tests the enabled bit.
    let (slot, base) = if state.selected {
        (WndDrawDataSlot::Hilite, 3)
    } else {
        (slot, 0)
    };
    let left = binder.image(slot, base)?;
    let centre = binder.image(slot, base + 1)?;
    let right = binder.image(slot, base + 2)?;
    let (x_offset, y_offset) = image_offset;
    let top = rect.y + y_offset;
    let left_end = rect.x + left.size[0] + x_offset;
    let right_start = rect.x + rect.width - right.size[0] + x_offset;

    let mut pieces = trimmed_row(left_end, right_start, top, rect.height, centre);
    pieces.push((
        UiRect {
            x: rect.x + x_offset,
            y: top,
            width: left.size[0],
            height: rect.height,
        },
        left,
    ));
    // The right end runs to the control's own right edge rather than to its image width, which is
    // the source's `end.x = origin.x + size.x`.
    pieces.push((
        UiRect {
            x: right_start,
            y: top,
            width: rect.x + rect.width - right_start,
            height: rect.height,
        },
        right,
    ));
    Some(pieces)
}

/// `W3DGadgetCheckBoxImageDraw`, whose background image the source itself leaves commented out.
fn check_box(
    binder: &mut SlotBinder<'_>,
    rect: UiRect,
    image_offset: (i32, i32),
    state: UiDrawState,
    slot: WndDrawDataSlot,
) -> Option<Vec<Piece>> {
    let image = binder.image(slot, if state.selected { 2 } else { 1 })?;
    let side = rect.height - 6;
    Some(vec![(
        UiRect {
            x: rect.x + image_offset.0,
            y: rect.y + 3,
            width: side,
            height: side,
        },
        image,
    )])
}

/// `W3DGadgetTextEntryImageDraw`.
fn text_entry(
    binder: &mut SlotBinder<'_>,
    rect: UiRect,
    image_offset: (i32, i32),
    slot: WndDrawDataSlot,
    budget: usize,
) -> Option<Vec<Piece>> {
    let left = binder.image(slot, 0)?;
    let right = binder.image(slot, 1)?;
    let centre = binder.image(slot, 2)?;
    let small = binder.image(slot, 3)?;
    let (x_offset, y_offset) = image_offset;
    let top = rect.y + y_offset;
    let left_end = rect.x + left.size[0] + x_offset;
    let right_start = rect.x + rect.width - right.size[0] + x_offset;

    let mut pieces = whole_row(left_end, right_start, top, rect.height, centre, budget);
    let filled = pieces
        .last()
        .map_or(left_end, |(rect, _)| rect.x + rect.width);
    // The source deliberately draws one small piece more than fits, overrunning into where the
    // right end will draw over it.
    pieces.extend(overrun_row(
        filled,
        right_start,
        top,
        rect.height,
        small,
        budget,
    ));
    pieces.push((
        UiRect {
            x: rect.x + x_offset,
            y: top,
            width: left.size[0],
            height: rect.height,
        },
        left,
    ));
    pieces.push((
        UiRect {
            x: right_start,
            y: top,
            width: right.size[0],
            height: rect.height,
        },
        right,
    ));
    Some(pieces)
}

/// `W3DGadgetVerticalSliderImageDraw`.
fn vertical_slider(
    binder: &mut SlotBinder<'_>,
    rect: UiRect,
    image_offset: (i32, i32),
    slot: WndDrawDataSlot,
    budget: usize,
) -> Option<Vec<Piece>> {
    let top_image = binder.image(slot, 0)?;
    let bottom_image = binder.image(slot, 1)?;
    let centre = binder.image(slot, 2)?;
    let small = binder.image(slot, 3)?;
    let (x_offset, y_offset) = image_offset;
    let left = rect.x + x_offset;

    // The source's "the two ends alone are taller than the control" branch: each end takes half.
    if top_image.size[1] + bottom_image.size[1] >= rect.height {
        let split = rect.y + rect.height / 2;
        return Some(vec![
            (
                UiRect {
                    x: left,
                    y: rect.y + y_offset,
                    width: top_image.size[0],
                    height: split - (rect.y + y_offset),
                },
                top_image,
            ),
            (
                UiRect {
                    x: left,
                    y: split,
                    width: bottom_image.size[0],
                    height: rect.y + y_offset + rect.height - split,
                },
                bottom_image,
            ),
        ]);
    }

    let top_end = rect.y + top_image.size[1] + y_offset;
    let bottom_start = rect.y + rect.height - bottom_image.size[1] + y_offset;
    let mut pieces = whole_column(top_end, bottom_start, left, centre, budget);
    let filled = pieces
        .last()
        .map_or(top_end, |(rect, _)| rect.y + rect.height);
    pieces.extend(overrun_column(filled, bottom_start, left, small, budget));
    pieces.push((
        UiRect {
            x: left,
            y: rect.y + y_offset,
            width: top_image.size[0],
            height: top_image.size[1],
        },
        top_image,
    ));
    pieces.push((
        UiRect {
            x: left,
            y: bottom_start,
            width: bottom_image.size[0],
            height: bottom_image.size[1],
        },
        bottom_image,
    ));
    Some(pieces)
}

/// `W3DGadgetHorizontalSliderImageDraw`, a row of tick squares filled up to the current position.
///
/// Two things here are the source's, not this project's: the art comes from fixed slots whatever
/// the control's own state — fill and blank from the disabled slot, highlight from the hilite slot
/// — and the square's size scales with the display width against a 800-pixel reference rather than
/// with the control.
fn horizontal_slider(
    binder: &mut SlotBinder<'_>,
    rect: UiRect,
    state: UiDrawState,
    canvas: [u32; 2],
    budget: usize,
) -> Option<Vec<Piece>> {
    let fill = binder.image(WndDrawDataSlot::Disabled, 0)?;
    let blank = binder.image(WndDrawDataSlot::Disabled, 1)?;
    let highlight = binder.image(WndDrawDataSlot::Hilite, 0);

    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        reason = "viewport widths and piece sizes are small integers, and the source truncates here                   too"
    )]
    let box_width = ((fill.size[0] as f32)
        * (f32::from(u16::try_from(canvas[0]).unwrap_or(u16::MAX)))
        / DEFAULT_DISPLAY_WIDTH) as i32;
    let box_width = box_width.max(1);
    let padding = 2;

    let span = if state.maximum > state.minimum {
        #[expect(
            clippy::cast_precision_loss,
            reason = "slider bounds are small integers"
        )]
        let fraction =
            (state.value - state.minimum) as f32 / (state.maximum - state.minimum) as f32;
        #[expect(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            reason = "control widths are small integers"
        )]
        let scaled = (fraction * rect.width as f32) as i32;
        rect.x + scaled
    } else {
        rect.x
    };

    let mut boxes = 0;
    let mut selected = 0;
    let mut start = rect.x;
    let mut end = start + box_width;
    while end < rect.x + rect.width && boxes < budget {
        if start <= span && state.value != state.minimum {
            selected += 1;
        }
        start = end + padding;
        end = start + box_width;
        boxes += 1;
    }

    // The row is centred in whatever width the whole squares did not cover.
    let covered = end - rect.x - box_width;
    let origin_x = rect.x + (rect.width - covered) / 2;
    let mut pieces = Vec::new();
    if state.hilited
        && let Some(highlight) = highlight
    {
        let side = box_width + padding;
        for index in 0..=boxes {
            pieces.push((
                UiRect {
                    x: origin_x - side / 2 + i32::try_from(index).unwrap_or(0) * side,
                    y: rect.y + box_width / 3,
                    width: side,
                    height: side,
                },
                highlight,
            ));
        }
    }
    for index in 0..boxes {
        let image = if index < selected { fill } else { blank };
        pieces.push((
            UiRect {
                x: origin_x + i32::try_from(index).unwrap_or(0) * (box_width + padding),
                y: rect.y,
                width: box_width,
                height: box_width,
            },
            image,
        ));
    }
    Some(pieces)
}

/// `W3DGadgetProgressBarImageDraw`: a three-piece background with the bar drawn inside it.
fn progress_bar(
    binder: &mut SlotBinder<'_>,
    rect: UiRect,
    image_offset: (i32, i32),
    state: UiDrawState,
    slot: WndDrawDataSlot,
    budget: usize,
) -> Option<Vec<Piece>> {
    let left = binder.image(slot, 0)?;
    let right = binder.image(slot, 1)?;
    let centre = binder.image(slot, 2)?;
    let bar_right = binder.image(slot, 5)?;
    let bar_centre = binder.image(slot, 6)?;
    let (x_offset, y_offset) = image_offset;
    let top = rect.y + y_offset;
    let left_end = rect.x + left.size[0] + x_offset;
    let right_start = rect.x + rect.width - right.size[0] + x_offset;

    let mut pieces = trimmed_row(left_end, right_start, top, rect.height, centre);
    pieces.push((
        UiRect {
            x: rect.x + x_offset,
            y: top,
            width: left.size[0],
            height: rect.height,
        },
        left,
    ));
    pieces.push((
        UiRect {
            x: right_start,
            y: top,
            width: right.size[0],
            height: rect.height,
        },
        right,
    ));

    // The bar sits ten pixels inside the background horizontally and five vertically, and the
    // source fills the rest of the track with the bar's right piece rather than leaving it empty.
    let bar_width = ((rect.width - 20) * state.value.clamp(0, 100)) / 100;
    let filled = bar_width / bar_centre.size[0].max(1);
    let track = (rect.width - 20) / bar_centre.size[0].max(1);
    let bar_top = rect.y + y_offset + 5;
    let bar_height = rect.height - 10;
    let mut x = rect.x + 10;
    for _ in 0..filled.min(i32::try_from(budget).unwrap_or(i32::MAX)) {
        pieces.push((
            UiRect {
                x,
                y: bar_top,
                width: bar_centre.size[0],
                height: bar_height,
            },
            bar_centre,
        ));
        x += bar_centre.size[0];
    }
    for _ in 0..(track - filled)
        .max(0)
        .min(i32::try_from(budget).unwrap_or(i32::MAX))
    {
        pieces.push((
            UiRect {
                x,
                y: bar_top,
                width: bar_right.size[0],
                height: bar_height,
            },
            bar_right,
        ));
        x += bar_right.size[0].max(1);
    }
    Some(pieces)
}

/// `W3DGadgetTabControlImageDraw` over `GadgetTabControlComputeTabRegion`.
fn tab_control(
    binder: &mut SlotBinder<'_>,
    rect: UiRect,
    image_offset: (i32, i32),
    geometry: cic_ui::UiTabGeometry,
    slot: WndDrawDataSlot,
) -> Option<Vec<Piece>> {
    // The background is the default draw's entry-0 image; a tab control with none still draws its
    // tabs, so an absent background is not the family's early return.
    let mut pieces = Vec::new();
    if let Some(background) = binder.image(slot, 0) {
        pieces.push((offset_rect(rect, image_offset), background));
    }

    let count = i32::try_from(geometry.count).unwrap_or(0);
    let horizontal = geometry.edge == TAB_EDGE_TOP || geometry.edge == TAB_EDGE_BOTTOM;
    // `TP_CENTER` is 0, `TP_TOPLEFT` 1, and `TP_BOTTOMRIGHT` 2; the spare space goes before the
    // strip, half of it when centred.
    let spare = if horizontal {
        rect.width - 2 * geometry.pane_border - count * geometry.width
    } else {
        rect.height - 2 * geometry.pane_border - count * geometry.height
    };
    let shift = match geometry.orientation {
        TAB_ORIENTATION_CENTER => spare / 2,
        TAB_ORIENTATION_BOTTOM_RIGHT => spare,
        _ => 0,
    };
    let (mut x, mut y) = match geometry.edge {
        TAB_EDGE_BOTTOM => (
            rect.x + geometry.pane_border + shift,
            rect.y + rect.height - geometry.pane_border - geometry.height,
        ),
        TAB_EDGE_RIGHT => (
            rect.x + rect.width - geometry.pane_border - geometry.width,
            rect.y + geometry.pane_border + shift,
        ),
        TAB_EDGE_LEFT => (
            rect.x + geometry.pane_border,
            rect.y + geometry.pane_border + shift,
        ),
        _ => (
            rect.x + geometry.pane_border + shift,
            rect.y + geometry.pane_border,
        ),
    };
    let (step_x, step_y) = if horizontal {
        (geometry.width, 0)
    } else {
        (0, geometry.height)
    };

    for tab in 0..geometry.count {
        let tab_slot = if geometry.disabled[tab] {
            WndDrawDataSlot::Disabled
        } else if tab == geometry.active {
            WndDrawDataSlot::Hilite
        } else {
            WndDrawDataSlot::Enabled
        };
        if let Some(image) = binder.image(tab_slot, tab + 1) {
            pieces.push((
                UiRect {
                    x,
                    y,
                    width: geometry.width,
                    height: geometry.height,
                },
                image,
            ));
        }
        x += step_x;
        y += step_y;
    }
    (!pieces.is_empty()).then_some(pieces)
}

/// `TABEDGE`'s `TP_TOP_SIDE`.
const TAB_EDGE_TOP: i32 = 3;
/// `TABEDGE`'s `TP_RIGHT_SIDE`.
const TAB_EDGE_RIGHT: i32 = 4;
/// `TABEDGE`'s `TP_LEFT_SIDE`.
const TAB_EDGE_LEFT: i32 = 5;
/// `TABEDGE`'s `TP_BOTTOM_SIDE`.
const TAB_EDGE_BOTTOM: i32 = 6;
/// `TABORIENTATION`'s `TP_CENTER`.
const TAB_ORIENTATION_CENTER: i32 = 0;
/// `TABORIENTATION`'s `TP_BOTTOMRIGHT`.
const TAB_ORIENTATION_BOTTOM_RIGHT: i32 = 2;

/// Returns a rectangle moved by a control's `IMAGEOFFSET`.
const fn offset_rect(rect: UiRect, image_offset: (i32, i32)) -> UiRect {
    UiRect {
        x: rect.x + image_offset.0,
        y: rect.y + image_offset.1,
        ..rect
    }
}

/// Repeats a piece rightwards, trimming the final partial one to the remaining width.
///
/// The source draws that final piece whole under a clip region ending at `to`; trimming texture
/// coordinates samples exactly the same pixels without a clip state change, keeping the batch
/// intact. Unbounded repetition cannot happen: the piece is at least one pixel wide, and the caller
/// staging these hits the quad limit before a wide control can produce an unbounded list.
fn trimmed_row(from: i32, to: i32, top: i32, height: i32, piece: UiImageBinding) -> Vec<Piece> {
    let width = piece.size[0].max(1);
    let mut pieces = Vec::new();
    let mut x = from;
    while x + width <= to {
        pieces.push((
            UiRect {
                x,
                y: top,
                width,
                height,
            },
            piece,
        ));
        x += width;
    }
    if x < to {
        pieces.push((
            UiRect {
                x,
                y: top,
                width: to - x,
                height,
            },
            piece.trimmed_to_width(to - x, width),
        ));
    }
    pieces
}

/// Repeats a piece rightwards in whole pieces only, leaving any remainder to the caller.
fn whole_row(
    from: i32,
    to: i32,
    top: i32,
    height: i32,
    piece: UiImageBinding,
    budget: usize,
) -> Vec<Piece> {
    let width = piece.size[0].max(1);
    let mut pieces = Vec::new();
    let mut x = from;
    while x + width <= to && pieces.len() < budget {
        pieces.push((
            UiRect {
                x,
                y: top,
                width,
                height,
            },
            piece,
        ));
        x += width;
    }
    pieces
}

/// Repeats a piece rightwards one piece past the gap, which is the source's own `pieces + 1`: the
/// overrun draws under the end piece the caller adds afterwards.
fn overrun_row(
    from: i32,
    to: i32,
    top: i32,
    height: i32,
    piece: UiImageBinding,
    budget: usize,
) -> Vec<Piece> {
    let width = piece.size[0].max(1);
    let count = ((to - from) / width + 1).max(0);
    (0..count.min(i32::try_from(budget).unwrap_or(i32::MAX)))
        .map(|index| {
            (
                UiRect {
                    x: from + index * width,
                    y: top,
                    width,
                    height,
                },
                piece,
            )
        })
        .collect()
}

/// [`whole_row`] stacked downwards.
fn whole_column(from: i32, to: i32, left: i32, piece: UiImageBinding, budget: usize) -> Vec<Piece> {
    let height = piece.size[1].max(1);
    let mut pieces = Vec::new();
    let mut y = from;
    while y + height <= to && pieces.len() < budget {
        pieces.push((
            UiRect {
                x: left,
                y,
                width: piece.size[0],
                height,
            },
            piece,
        ));
        y += height;
    }
    pieces
}

/// [`overrun_row`] stacked downwards.
fn overrun_column(
    from: i32,
    to: i32,
    left: i32,
    piece: UiImageBinding,
    budget: usize,
) -> Vec<Piece> {
    let height = piece.size[1].max(1);
    let count = ((to - from) / height + 1).max(0);
    (0..count.min(i32::try_from(budget).unwrap_or(i32::MAX)))
        .map(|index| {
            (
                UiRect {
                    x: left,
                    y: from + index * height,
                    width: piece.size[0],
                    height,
                },
                piece,
            )
        })
        .collect()
}

impl UiImageBinding {
    /// Returns this binding with its texture coordinates trimmed to a partial width.
    ///
    /// The source draws a whole repeating piece under a clip region; trimming the coordinates
    /// samples exactly the visible part instead, which needs no clip state change.
    fn trimmed_to_width(&self, visible: i32, full: i32) -> Self {
        if visible >= full || full <= 0 {
            return *self;
        }
        #[expect(
            clippy::cast_precision_loss,
            reason = "piece widths are small pixel counts"
        )]
        let fraction = visible as f32 / full as f32;
        let [left, top, right, bottom] = self.uv;
        Self {
            page: self.page,
            uv: [left, top, left + (right - left) * fraction, bottom],
            size: [visible, self.size[1]],
        }
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
    use cic_ui::{
        UiClipPolicy, UiFrame, UiLayout, UiLimits, UiPresentation, UiScalePolicy, UiViewport,
    };

    /// An original synthetic layout declaring every family this layer composes, each with art at
    /// the indices its own source draw procedure reads. No retail data appears here.
    const SYNTHETIC_GADGETS: &str = include_str!("../tests/fixtures/synthetic-gadgets.wnd");

    fn layout(width: i32, height: i32) -> UiLayout {
        let document =
            parse_wnd(SYNTHETIC_GADGETS.as_bytes(), WndLimits::default()).expect("decode layout");
        assert!(
            document.diagnostics().is_empty(),
            "synthetic fixture should decode cleanly: {:?}",
            document.diagnostics()
        );
        let viewport = UiViewport::new(width, height).expect("positive viewport");
        UiLayout::instantiate(
            &document,
            UiPresentation::new(viewport, UiScalePolicy::Classic),
            UiLimits::default(),
        )
        .expect("instantiate layout")
    }

    /// Binds every synthetic image. `SynthPanel` sits on its own page so batching is observable.
    fn bind_synth(name: &str) -> Option<UiImageBinding> {
        let size = match name {
            "SynthPanel" => [64, 32],
            "SynthEnd" => [10, 24],
            "SynthMiddle" => [8, 24],
            "SynthBox" | "SynthPicked" => [4, 6],
            _ => return None,
        };
        Some(UiImageBinding {
            page: usize::from(name == "SynthPanel"),
            uv: [0.0, 0.0, 1.0, 1.0],
            size,
        })
    }

    fn stage(frame: &UiFrame, canvas: [u32; 2]) -> StagedUiFrame {
        StagedUiFrame::from_frame(
            frame,
            canvas,
            UiTextPolicy::Shape,
            UiStagingLimits::default(),
            &bind_synth,
        )
        .expect("stage frame")
    }

    /// Returns the staged quads as `(x, y, width, height)`, in submission order.
    fn quads(staged: &StagedUiFrame) -> Vec<(i32, i32, i32, i32)> {
        staged
            .vertices()
            .chunks_exact(4)
            .map(|corners| {
                let [left, top] = corners[0].position();
                let [right, bottom] = corners[2].position();
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "staged positions come from integer pixel rectangles"
                )]
                (
                    left as i32,
                    top as i32,
                    (right - left) as i32,
                    (bottom - top) as i32,
                )
            })
            .collect()
    }

    /// Hides every sibling of one named control, so the staged quads after the panel's own five
    /// belong to that control alone.
    fn isolate(layout: &mut UiLayout, name: &str) -> cic_ui::UiControlId {
        let wanted = layout
            .find(&format!("SynthMenu.wnd:{name}"))
            .expect("named control");
        for child in layout.control(layout.roots()[0]).children().to_vec() {
            if child != wanted {
                layout.set_hidden(child, true);
            }
        }
        wanted
    }

    /// The panel stages one fill plus its four border edges before any child.
    const PANEL_QUADS: usize = 5;

    /// Returns the quads a named control stages, with the panel's own removed.
    fn control_quads(name: &str) -> Vec<(i32, i32, i32, i32)> {
        let mut layout = layout(800, 600);
        isolate(&mut layout, name);
        let staged = stage(&layout.frame(UiClipPolicy::None), [800, 600]);
        quads(&staged).split_off(PANEL_QUADS)
    }

    #[test]
    fn a_push_button_composes_its_ends_over_a_repeating_centre() {
        // Ends are ten pixels wide and the centre eight, so the eighty pixels between them take ten
        // whole centres with nothing left to trim, and both ends draw last.
        let pieces = control_quads("ButtonSynth");
        assert_eq!(pieces.len(), 12);
        assert_eq!(pieces[0], (30, 20, 8, 40));
        assert_eq!(pieces[9], (102, 20, 8, 40));
        assert_eq!(pieces[10], (20, 20, 10, 40));
        assert_eq!(pieces[11], (110, 20, 10, 40));
    }

    #[test]
    fn a_selected_radio_button_reads_the_hilite_slot_while_enabled() {
        let mut layout = layout(800, 600);
        let radio = isolate(&mut layout, "RadioSynth");
        let unselected = stage(&layout.frame(UiClipPolicy::None), [800, 600]);
        layout.select_radio(radio);
        let selected = stage(&layout.frame(UiClipPolicy::None), [800, 600]);

        // Both states compose three pieces, but the selected one reads the hilite slot's second
        // triple — whose centre is a narrower image — so the piece count changes with it.
        assert!(!unselected.vertices().is_empty());
        assert_ne!(
            quads(&unselected).split_off(PANEL_QUADS),
            quads(&selected).split_off(PANEL_QUADS)
        );
        assert!(selected.diagnostics().is_empty());
    }

    #[test]
    fn a_check_box_draws_only_its_box_inset_from_the_left() {
        // The source draws no background for a check box: only the box, three pixels down and six
        // shorter than the control, at the control's own left edge.
        assert_eq!(control_quads("CheckSynth"), [(20, 113, 24, 24)]);
    }

    #[test]
    fn a_text_entry_fills_its_seam_with_the_small_centre_piece() {
        let pieces = control_quads("EntrySynth");
        // Twelve whole eight-pixel centres between the two ten-pixel ends, then four-pixel small
        // centres overrunning by one under the right end, then both ends.
        assert_eq!(pieces[0], (150, 20, 8, 30));
        assert_eq!(pieces[11], (238, 20, 8, 30));
        assert_eq!(pieces[12], (246, 20, 4, 30));
        assert_eq!(pieces[pieces.len() - 2], (140, 20, 10, 30));
        assert_eq!(pieces[pieces.len() - 1], (250, 20, 10, 30));
    }

    #[test]
    fn a_vertical_slider_stacks_its_pieces_and_halves_them_when_the_ends_do_not_fit() {
        let pieces = control_quads("VerticalSynth");
        assert_eq!(pieces[0], (280, 44, 8, 24));
        assert_eq!(pieces[pieces.len() - 2], (280, 20, 10, 24));
        assert_eq!(pieces[pieces.len() - 1], (280, 116, 10, 24));

        // A 30-pixel track cannot hold its own two 24-pixel ends, so each takes half instead.
        assert_eq!(
            control_quads("ShortSynth"),
            [(310, 20, 10, 15), (310, 35, 10, 15)]
        );
    }

    #[test]
    fn a_horizontal_slider_fills_tick_squares_up_to_its_position() {
        let mut layout = layout(800, 600);
        let slider = isolate(&mut layout, "HorizontalSynth");
        let empty =
            quads(&stage(&layout.frame(UiClipPolicy::None), [800, 600])).split_off(PANEL_QUADS);
        assert_eq!(layout.set_slider_value(slider, 5), Some(5));
        let half =
            quads(&stage(&layout.frame(UiClipPolicy::None), [800, 600])).split_off(PANEL_QUADS);

        // The tick count depends only on the control's width, so moving the slider changes which
        // squares are filled rather than how many there are.
        assert!(!empty.is_empty());
        assert_eq!(empty, half);
        // A four-pixel square at the 800-pixel reference width stays four pixels, is square, and
        // sits two pixels of padding from the next.
        assert_eq!(empty[0].2, 4);
        assert_eq!(empty[0].3, 4);
        assert_eq!(empty[1].0 - empty[0].0, 6);
    }

    #[test]
    fn a_progress_bar_draws_its_bar_inside_its_background() {
        let mut layout = layout(800, 600);
        let bar = isolate(&mut layout, "ProgressSynth");
        assert_eq!(layout.set_progress(bar, 50), Some(50));
        let pieces =
            quads(&stage(&layout.frame(UiClipPolicy::None), [800, 600])).split_off(PANEL_QUADS);
        // Every bar piece sits ten pixels inside the background horizontally and five vertically.
        let bar_pieces: Vec<_> = pieces.iter().filter(|piece| piece.1 == 185).collect();
        assert!(!bar_pieces.is_empty());
        for piece in &bar_pieces {
            assert!(piece.0 >= 30);
            assert_eq!(piece.3, 14);
        }
    }

    #[test]
    fn a_tab_control_draws_its_background_then_one_image_per_declared_tab() {
        // `TABEDGE: 3` is the top side and `TABORIENTATION: 1` the top left, so the two 40x20 tabs
        // start at the four-pixel pane border and run rightwards.
        assert_eq!(
            control_quads("TabsSynth"),
            [(240, 150, 140, 100), (244, 154, 40, 20), (284, 154, 40, 20),]
        );
    }

    #[test]
    fn art_at_an_index_the_family_never_reads_is_reported_and_not_painted() {
        let layout = layout(800, 600);
        let staged = stage(&layout.frame(UiClipPolicy::None), [800, 600]);
        assert!(staged.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic.kind(),
            UiStagingDiagnosticKind::UncomposedArt { family } if *family == "Simple"
        )));
        // Nothing is painted for it: the source's own draw procedure returns early there, so a
        // placeholder would invent a control retail never shows.
        assert!(!quads(&staged).iter().any(|piece| piece.1 == 260));
    }

    #[test]
    fn the_draw_callback_name_decides_the_image_path_over_the_status_bit() {
        let layout = layout(800, 600);
        let staged = stage(&layout.frame(UiClipPolicy::None), [800, 600]);
        // `ColorSynth` declares `IMAGE` and a bound entry-0 image, but names the colour-only
        // procedure, so it stages an untextured fill in the slot's own colour.
        let colored = staged
            .vertices()
            .chunks_exact(4)
            .find(|corners| (corners[0].position()[1] - 292.0).abs() < 0.5)
            .expect("colour-only control");
        assert!(!colored[0].is_textured());
        assert!(colored[0].color()[3] > 0.9);
    }

    #[test]
    fn an_unbound_image_stages_a_visible_placeholder_and_reports_it() {
        let mut layout = layout(800, 600);
        isolate(&mut layout, "ButtonSynth");
        let frame = layout.frame(UiClipPolicy::None);
        let staged = StagedUiFrame::from_frame(
            &frame,
            [800, 600],
            UiTextPolicy::Shape,
            UiStagingLimits::default(),
            &|name| (name != "SynthMiddle").then(|| bind_synth(name)).flatten(),
        )
        .expect("stage frame");
        assert!(staged.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic.kind(),
            UiStagingDiagnosticKind::UnboundImage { name } if &**name == "SynthMiddle"
        )));
        // One placeholder covers the whole control, because piece geometry needs the bound size.
        assert_eq!(quads(&staged).split_off(PANEL_QUADS), [(20, 20, 100, 40)]);
    }

    #[test]
    fn staging_batches_by_page_and_carries_text_runs() {
        let layout = layout(800, 600);
        let staged = stage(&layout.frame(UiClipPolicy::None), [800, 600]);

        assert!(staged.vertices()[0].is_textured());
        assert_eq!(staged.batches()[0].page(), Some(1));
        // The panel declares a border, so four one-pixel untextured edges follow its fill and
        // break the batch away from the textured page.
        assert!(staged.batches().len() >= 2);
        assert_eq!(
            staged.indices().len(),
            staged
                .batches()
                .iter()
                .map(|batch| batch.index_count() as usize)
                .sum::<usize>()
        );
        assert_eq!(staged.text().len(), 1);
        assert_eq!(staged.text()[0].text, "GUI:SynthButton");
        assert_eq!(staged.text()[0].family, "Synth Sans");
        assert_eq!(staged.text()[0].color, [255, 255, 255, 255]);
    }

    #[test]
    fn clips_intersect_and_become_batch_boundaries() {
        let layout = layout(800, 600);
        let staged = stage(&layout.frame(UiClipPolicy::ClipToParent), [800, 600]);
        let scissored: Vec<_> = staged
            .batches()
            .iter()
            .filter_map(|batch| batch.scissor())
            .collect();
        assert!(!scissored.is_empty());
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
        let shaped = stage(&frame, [800, 600]);
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
        let first = stage(&frame, [1920, 1080]);
        let second = stage(&frame, [1920, 1080]);
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
