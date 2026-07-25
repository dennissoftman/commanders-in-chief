// Commanders in Chief
// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only
//
// Unicode shaping and glyph rasterization use `cosmic-text` 0.19 (MIT OR Apache-2.0) and its
// `wgpu` glyph renderer `glyphon` 0.12 (MIT OR Apache-2.0 OR Zlib), both permissive and compatible
// with this project's GPL-3.0-only licence. `glyphon` 0.12 declares `wgpu ^30.0.0`, which unifies
// with the workspace's `wgpu` 30 rather than pulling a second copy — verified with `cargo tree`.
// ADR 0010 selected this pair; these versions are the current releases that satisfy it.
//
// Neither library defines UI semantics. Layout rectangles, colours, sizes, and draw order come from
// `cic-ui`; this module only shapes text into those rectangles.

//! Explicit font sets for shaping retained-UI text.
//!
//! Fonts are always supplied as bytes by the caller. Nothing here enumerates host fonts, because a
//! deterministic capture that silently picked up a platform face would produce a different hash on a
//! different machine — and retail ships no font files at all, so a project-supplied face is the
//! ordinary path rather than a fallback.

use std::sync::Arc;

use cic_ui::UiTextAlign;
use cosmic_text::{Align, fontdb};
use glyphon::{
    Attrs, Buffer, Cache, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache, TextArea,
    TextAtlas, TextBounds, TextRenderer, Viewport, Weight,
};

use crate::ui::StagedUiText;

/// A failure while preparing or drawing text.
#[derive(Debug)]
pub enum UiTextError {
    /// No font source could be loaded from the supplied bytes.
    NoUsableFont,
    /// `glyphon` could not prepare the atlas for this frame.
    Prepare(glyphon::PrepareError),
    /// `glyphon` could not draw the prepared text.
    Draw(glyphon::RenderError),
}

impl std::fmt::Display for UiTextError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoUsableFont => {
                formatter.write_str("no usable font was supplied; UI text cannot be shaped")
            }
            Self::Prepare(error) => write!(formatter, "cannot prepare UI text: {error}"),
            Self::Draw(error) => write!(formatter, "cannot draw UI text: {error}"),
        }
    }
}

impl std::error::Error for UiTextError {}

/// An explicit set of fonts plus the GPU resources needed to draw shaped text.
pub struct UiFontSet {
    fonts: FontSystem,
    swash: SwashCache,
    atlas: TextAtlas,
    renderer: TextRenderer,
    viewport: Viewport,
    families: Vec<String>,
}

impl UiFontSet {
    /// Builds a font set from explicit font file bytes.
    ///
    /// The first supplied face is the default family, so a layout naming a family nothing supplies
    /// still renders in a known face instead of disappearing.
    ///
    /// # Errors
    ///
    /// Returns [`UiTextError::NoUsableFont`] when no supplied byte slice parses as a font.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        fonts: &[Vec<u8>],
    ) -> Result<Self, UiTextError> {
        let sources = fonts
            .iter()
            .map(|bytes| fontdb::Source::Binary(Arc::new(bytes.clone())));
        let font_system = FontSystem::new_with_fonts(sources);
        let families: Vec<String> = font_system
            .db()
            .faces()
            .flat_map(|face| face.families.iter().map(|(name, _)| name.clone()))
            .collect();
        if families.is_empty() {
            return Err(UiTextError::NoUsableFont);
        }
        let cache = Cache::new(device);
        let viewport = Viewport::new(device, &cache);
        let mut atlas = TextAtlas::new(device, queue, &cache, format);
        let renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        Ok(Self {
            fonts: font_system,
            swash: SwashCache::new(),
            atlas,
            renderer,
            viewport,
            families,
        })
    }

    /// Returns every family name the supplied faces provide, in load order.
    #[must_use]
    pub fn families(&self) -> &[String] {
        &self.families
    }

    /// Shapes and uploads every staged run for one frame.
    ///
    /// A run's requested family is used when the set provides it, and the set's first family
    /// otherwise; the substitution is silent here because [`crate::StagedUiFrame`] already reports
    /// which families a layout asked for.
    ///
    /// # Errors
    ///
    /// Returns [`UiTextError::Prepare`] when the glyph atlas cannot accept the frame.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        canvas: [u32; 2],
        runs: &[StagedUiText],
    ) -> Result<(), UiTextError> {
        let [width, height] = canvas;
        self.viewport.update(queue, Resolution { width, height });
        let default_family = self.families.first().cloned().unwrap_or_default();

        let mut buffers = Vec::with_capacity(runs.len());
        for run in runs {
            let size = if run.size > 0.0 { run.size } else { 12.0 };
            let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(size, size * 1.25));
            #[expect(
                clippy::cast_precision_loss,
                reason = "layout rectangles are small pixel counts"
            )]
            buffer.set_size(
                Some(run.rect.width.max(0) as f32),
                Some(run.rect.height.max(0) as f32),
            );
            let family = if self.families.contains(&run.family) {
                run.family.as_str()
            } else {
                default_family.as_str()
            };
            let weight = if run.bold {
                Weight::BOLD
            } else {
                Weight::NORMAL
            };
            let attrs = Attrs::new().family(Family::Name(family)).weight(weight);
            // `drawButtonText` centres a button's text on both axes. Horizontal centring is the
            // shaper's own alignment; the vertical half is a placement offset computed below from
            // the shaped height, since the source centres the measured text block in the control.
            let align = match run.align {
                UiTextAlign::Centered => Some(Align::Center),
                UiTextAlign::TopLeft => None,
            };
            buffer.set_text(&run.text, &attrs, Shaping::Advanced, align);
            buffer.shape_until_scroll(&mut self.fonts, false);
            buffers.push(buffer);
        }

        let areas = runs.iter().zip(&buffers).map(|(run, buffer)| {
            let bounds = run.scissor.unwrap_or(run.rect);
            let top_offset = match run.align {
                UiTextAlign::Centered => {
                    let lines = buffer.layout_runs().count().max(1);
                    #[expect(clippy::cast_precision_loss, reason = "a shaped run has few lines")]
                    let shaped_height = buffer.metrics().line_height * lines as f32;
                    #[expect(
                        clippy::cast_precision_loss,
                        reason = "layout rectangles are small pixel counts"
                    )]
                    let available = run.rect.height as f32;
                    ((available - shaped_height) / 2.0).max(0.0)
                }
                UiTextAlign::TopLeft => 0.0,
            };
            TextArea {
                buffer,
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "layout rectangles are small pixel counts"
                )]
                left: run.rect.x as f32,
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "layout rectangles are small pixel counts"
                )]
                top: run.rect.y as f32 + top_offset,
                scale: 1.0,
                bounds: TextBounds {
                    left: bounds.x,
                    top: bounds.y,
                    right: bounds.x + bounds.width,
                    bottom: bounds.y + bounds.height,
                },
                default_color: glyphon::Color::rgba(
                    run.color[0],
                    run.color[1],
                    run.color[2],
                    run.color[3],
                ),
                custom_glyphs: &[],
            }
        });

        self.renderer
            .prepare(
                device,
                queue,
                &mut self.fonts,
                &mut self.atlas,
                &self.viewport,
                areas,
                &mut self.swash,
            )
            .map_err(UiTextError::Prepare)
    }

    /// Draws the text prepared for this frame into an open render pass.
    ///
    /// # Errors
    ///
    /// Returns [`UiTextError::Draw`] when the prepared atlas cannot be bound.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) -> Result<(), UiTextError> {
        self.renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(UiTextError::Draw)
    }

    /// Releases atlas space that no longer backs a prepared glyph.
    pub fn trim(&mut self) {
        self.atlas.trim();
    }
}
