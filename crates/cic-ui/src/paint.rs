//! What to draw: a solved layout and its state turned into rectangles and text runs.
//!
//! # Why this is here and not in the renderer
//!
//! Nothing in it needs a GPU. Deciding that a focused button gets an outline, that a checkbox's
//! indicator is a square at its leading edge, that a slider's knob sits a fraction of the way along its
//! track, and that a scrollable container's contents shift up by its offset — all of that is arithmetic
//! over a solved layout, and all of it is where the mistakes are. Put in the renderer it would be
//! testable only by capturing an image; put here it is testable by asserting on a list.
//!
//! The renderer's remaining job is genuinely graphical: rasterise the glyphs, pack them into an atlas,
//! and turn this list into vertices.
//!
//! # Why a layout names a role and not a colour
//!
//! The same argument the string table makes about text. An authored colour is a decision about how the
//! interface looks, spread across every screen file, and changing it means finding every literal. So a
//! node says what it *is* — [`Style::Card`], [`Style::Caption`] — and a [`Theme`] decides what that
//! looks like. A layout file contains no colour at all.
//!
//! # Why most nodes draw nothing
//!
//! A layout tree is mostly structure. A `Panel` with no style is a row or a column whose job is to place
//! its children, and giving it a background would paint the screen over in nested rectangles. So the
//! absence of a style means invisible, and only a widget kind that *is* something — a button, a slider —
//! draws without being asked.
//!
//! # Why colours are sRGB bytes and leave as linear floats
//!
//! A theme is authored the way a designer states a colour, which is a byte per channel in sRGB. A
//! shader writing to an sRGB target must emit **linear** values, because the hardware applies the
//! encoding on the way out. Passing `byte / 255` straight through is the mistake that makes every
//! surface too bright and every gradient wrong, and it is invisible in a unit test that only compares
//! numbers to themselves. [`Colour::to_linear`] is therefore the one conversion, and it has its own
//! test against known values.

use crate::geometry::{Rect, Viewport};
use crate::layout::{Align, Style, Widget};
use crate::solve::{Measure, Solved, SolvedNode};
use crate::state::Interface;
use crate::strings::StringTable;
use crate::transition::Reveal;

/// A colour, as authored: one byte per channel, sRGB encoded, straight alpha.
///
/// Straight rather than premultiplied because that is how a colour is written down and how the shader's
/// blend state is configured. Bytes rather than floats because a theme is authored, and `0x1a` is a
/// thing a person writes while `0.10196079` is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Colour {
    /// Red, sRGB encoded.
    pub red: u8,
    /// Green, sRGB encoded.
    pub green: u8,
    /// Blue, sRGB encoded.
    pub blue: u8,
    /// Opacity, linear.
    pub alpha: u8,
}

impl Colour {
    /// Fully transparent, which is what a node with nothing to draw uses.
    pub const NONE: Self = Self::rgba(0, 0, 0, 0);

    /// An opaque colour.
    #[must_use]
    pub const fn rgb(red: u8, green: u8, blue: u8) -> Self {
        Self::rgba(red, green, blue, 0xff)
    }

    /// A colour with an opacity.
    #[must_use]
    pub const fn rgba(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }

    /// A colour from a packed `0xRRGGBBAA`, which is how a theme reads most legibly.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub const fn hex(packed: u32) -> Self {
        Self::rgba(
            (packed >> 24) as u8,
            (packed >> 16) as u8,
            (packed >> 8) as u8,
            packed as u8,
        )
    }

    /// The same colour with a different opacity.
    #[must_use]
    pub const fn with_alpha(self, alpha: u8) -> Self {
        Self { alpha, ..self }
    }

    /// Whether this colour would draw nothing.
    #[must_use]
    pub const fn is_invisible(self) -> bool {
        self.alpha == 0
    }

    /// Linear premultiplication-free floats, with the sRGB encoding removed from the colour channels.
    ///
    /// Alpha is *not* transformed: it is a coverage fraction rather than a light intensity, and applying
    /// a transfer function to it is a separate, equally common mistake.
    #[must_use]
    pub fn to_linear(self) -> [f32; 4] {
        [
            srgb_to_linear(self.red),
            srgb_to_linear(self.green),
            srgb_to_linear(self.blue),
            f32::from(self.alpha) / 255.0,
        ]
    }
}

/// The sRGB electro-optical transfer function, on one channel byte.
fn srgb_to_linear(channel: u8) -> f32 {
    let encoded = f32::from(channel) / 255.0;
    if encoded <= 0.040_45 {
        encoded / 12.92
    } else {
        ((encoded + 0.055) / 1.055).powf(2.4)
    }
}

/// Where a text run sits along its box.
///
/// Its own type rather than [`Align`], which has a `Stretch` that means nothing for a single line of
/// text. A consumer that had to handle a fourth impossible case would either guess or panic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    /// Against the left edge.
    Leading,
    /// Centred.
    Center,
    /// Against the right edge.
    Trailing,
}

/// What one primitive is.
///
/// Two kinds, because a consumer needs exactly two things: a quad of one colour, and a string to
/// rasterise. Borders, focus rings, sliders and carets are all built from the first, so the renderer
/// never learns what a checkbox looks like.
#[derive(Debug, Clone, PartialEq)]
pub enum Content<'a> {
    /// A solid rectangle.
    Fill {
        /// Where, in physical pixels.
        rect: Rect,
        /// What colour.
        colour: Colour,
    },
    /// A line of text.
    Text {
        /// The box to place it in, in physical pixels. Vertically centred within it.
        rect: Rect,
        /// What to draw. For a text entry this includes any uncommitted composition, because that is
        /// what the user has to be able to see.
        text: &'a str,
        /// What colour.
        colour: Colour,
        /// The size to rasterise at, in **physical pixels** — already multiplied by the display scale,
        /// because the rasteriser wants the size it will actually draw.
        size: f32,
        /// Where in `rect` it sits horizontally.
        align: TextAlign,
    },
}

/// One thing to draw, and the region it is confined to.
///
/// The clip travels with every primitive rather than as a push-and-pop marker in the sequence. A marker
/// would make the list a state machine every consumer has to replay identically, and the one that gets
/// it wrong leaks a scissor into the rest of the frame. Carried per primitive, a consumer sets the
/// scissor when it changes and cannot be wrong about which primitives it covers.
#[derive(Debug, Clone, PartialEq)]
pub struct Primitive<'a> {
    /// The region this primitive is confined to, in physical pixels. The viewport when unconfined.
    pub clip: Rect,
    /// What to draw.
    pub content: Content<'a>,
}

/// Every colour and measurement the interface is drawn with.
///
/// Named for roles rather than for the widgets that use them, so one entry serves a button, a tab, and a
/// list row without three fields that have to be kept in step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    /// Behind a full screen.
    pub backdrop: Colour,
    /// A card's face.
    pub card: Colour,
    /// A hairline, and the outline around a card or a control.
    pub border: Colour,
    /// The wash a modal puts over what is behind it.
    pub scrim: Colour,
    /// Body text.
    pub text: Colour,
    /// Text that is secondary: a caption, or a label beside a control.
    pub muted: Colour,
    /// Text that wants attention.
    pub warning: Colour,
    /// A control's face at rest.
    pub control: Colour,
    /// A control's face under the pointer.
    pub control_hovered: Colour,
    /// A control's face while pressed.
    pub control_armed: Colour,
    /// The outline a focused control gets, replacing its border.
    pub focus: Colour,
    /// The accent: a slider's filled portion, a checkbox's mark, a selected row.
    pub accent: Colour,
    /// A slider's unfilled track, and a scrollbar's thumb.
    pub track: Colour,
    /// The caret in a text entry.
    pub caret: Colour,
    /// The underline beneath an uncommitted composition.
    pub composition: Colour,
    /// Body text size, in logical units.
    pub text_size: f32,
    /// A title's text size, in logical units.
    pub title_size: f32,
    /// A caption's text size, in logical units.
    pub caption_size: f32,
    /// How thick a border, a focus ring, and a divider are, in logical units.
    pub border_width: f32,
    /// Space between a checkbox's indicator and its label, in logical units.
    pub indicator_gap: f32,
    /// How thick a slider's track is, in logical units.
    pub track_thickness: f32,
    /// How wide a slider's knob is, in logical units.
    pub knob_width: f32,
    /// Space between a text entry's edge and its text, in logical units.
    pub text_inset: f32,
    /// How wide the caret is, in logical units.
    pub caret_width: f32,
    /// How wide a scrollbar is, in logical units.
    pub scrollbar_width: f32,
}

impl Default for Theme {
    /// The shell's own look: a dark slate field with a desaturated blue accent.
    ///
    /// Dark because a strategy interface sits beside a map for hours and a bright shell is the thing
    /// that gets turned off first. Desaturated because the map is where colour carries meaning —
    /// faction, terrain, threat — and a shell competing with it makes both harder to read.
    fn default() -> Self {
        Self {
            backdrop: Colour::hex(0x11_15_1a_ff),
            card: Colour::hex(0x1b_21_29_ff),
            border: Colour::hex(0x37_41_4d_ff),
            scrim: Colour::hex(0x06_08_0a_b0),
            text: Colour::hex(0xdd_e3_ea_ff),
            muted: Colour::hex(0x8c_98_a6_ff),
            warning: Colour::hex(0xe0_9a_4a_ff),
            control: Colour::hex(0x25_2d_37_ff),
            control_hovered: Colour::hex(0x30_3b_47_ff),
            control_armed: Colour::hex(0x1a_20_27_ff),
            focus: Colour::hex(0x6f_9e_c8_ff),
            accent: Colour::hex(0x4c_7a_a3_ff),
            track: Colour::hex(0x2c_34_3e_ff),
            caret: Colour::hex(0xdd_e3_ea_ff),
            composition: Colour::hex(0xa8_c4_de_ff),
            text_size: 16.0,
            title_size: 30.0,
            caption_size: 13.0,
            border_width: 1.0,
            indicator_gap: 8.0,
            track_thickness: 4.0,
            knob_width: 12.0,
            text_inset: 8.0,
            caret_width: 1.0,
            scrollbar_width: 4.0,
        }
    }
}

impl Theme {
    /// The colour and logical size a label's role draws at.
    #[must_use]
    pub fn text_role(&self, style: Option<Style>) -> (Colour, f32) {
        match style {
            Some(Style::Title) => (self.text, self.title_size),
            Some(Style::Caption) => (self.muted, self.caption_size),
            Some(Style::Warning) => (self.warning, self.text_size),
            _ => (self.text, self.text_size),
        }
    }
}

/// Turns solved layouts into primitives, given a theme and something that can measure text.
///
/// Holds the theme and the metrics rather than taking them per call, because a host has one of each for
/// the life of a window and threading them through every call site is how two parts of a frame end up
/// drawn at different sizes.
#[derive(Debug, Clone, Copy)]
pub struct Painter<'t, M> {
    theme: &'t Theme,
    metrics: &'t M,
    viewport: Viewport,
}

impl<'t, M: Measure> Painter<'t, M> {
    /// Builds a painter for one surface.
    #[must_use]
    pub const fn new(theme: &'t Theme, metrics: &'t M, viewport: Viewport) -> Self {
        Self {
            theme,
            metrics,
            viewport,
        }
    }

    /// The theme in force.
    #[must_use]
    pub const fn theme(&self) -> &'t Theme {
        self.theme
    }

    /// The surface being drawn to.
    #[must_use]
    pub const fn viewport(&self) -> Viewport {
        self.viewport
    }

    /// Draws one solved screen at a reveal, appending to an existing list.
    ///
    /// Appending rather than returning is what lets a host keep one buffer for the life of a window, and
    /// what lets a modal be drawn over the screen behind it by calling this twice.
    ///
    /// The [`Reveal`] is how a screen transition reaches the drawing layer, and it is the whole of it: an
    /// opacity multiplying every colour's alpha and an offset moving every rectangle. Its offset is a
    /// *fraction* of the viewport rather than a distance, because nothing outside the solver in this crate
    /// knows how many pixels anything is — the conversion happens here, where the viewport is.
    ///
    /// A screen revealed to nothing appends nothing, so a change that has faded one out costs no vertices
    /// at all rather than a screenful of invisible ones.
    pub fn paint_revealed<'a>(
        &self,
        out: &mut Vec<Primitive<'a>>,
        solved: &'a Solved,
        interface: &'a Interface,
        strings: &'a StringTable,
        reveal: Reveal,
    ) {
        if reveal.is_hidden() {
            return;
        }
        let bounds = self.viewport.bounds();
        let mut pass = Pass {
            theme: self.theme,
            metrics: self.metrics,
            scale: self.viewport.scale(),
            solved,
            interface,
            strings,
            out,
            opacity: reveal.opacity.clamp(0.0, 1.0),
            shift: [
                reveal.offset[0] * bounds.width,
                reveal.offset[1] * bounds.height,
            ],
        };
        pass.walk(bounds);
    }

    /// Draws one solved screen, fully revealed and in place.
    pub fn paint_into<'a>(
        &self,
        out: &mut Vec<Primitive<'a>>,
        solved: &'a Solved,
        interface: &'a Interface,
        strings: &'a StringTable,
    ) {
        self.paint_revealed(out, solved, interface, strings, Reveal::SHOWN);
    }

    /// Draws one solved screen.
    #[must_use]
    pub fn paint<'a>(
        &self,
        solved: &'a Solved,
        interface: &'a Interface,
        strings: &'a StringTable,
    ) -> Vec<Primitive<'a>> {
        let mut out = Vec::new();
        self.paint_into(&mut out, solved, interface, strings);
        out
    }

    /// Where an input method should put its candidate window: a rectangle around the **caret**.
    ///
    /// The narrowing that text metrics make possible. [`Interface::ime_cursor_area`] reports the whole
    /// field, because that module has no way to measure text; here the caret's offset along the string
    /// is known, so the candidate list can appear beside the character being composed rather than beside
    /// the box containing it. On a wide field those are a long way apart.
    ///
    /// Falls back to the field when the focused node is a text entry whose value is absent, which is
    /// what [`Interface::ime_cursor_area`] would have said anyway.
    #[must_use]
    pub fn ime_cursor_area(&self, solved: &Solved, interface: &Interface) -> Option<Rect> {
        let field = interface.ime_cursor_area(solved)?;
        let id = interface.focus()?;
        let text = interface.text(id)?;
        let size = self.viewport.to_physical(self.theme.text_size);
        let inset = self.viewport.to_physical(self.theme.text_inset);
        let before = self.advance(character_prefix(text.text(), text.cursor()));
        let width = self.viewport.to_physical(self.theme.caret_width).max(1.0);
        Some(
            Rect::new(
                field.x + inset + before,
                field.y,
                width,
                field.height.max(size),
            )
            .snapped(),
        )
    }

    /// The physical width of a string at the theme's body size.
    fn advance(&self, text: &str) -> f32 {
        self.viewport
            .to_physical(self.metrics.advance(text, self.theme.text_size))
    }
}

/// The walk's invariant state.
///
/// A struct rather than eight parameters threaded through every helper, for the reason the solver's
/// arrange pass is one: the recursion carries the same references unchanged at every level.
struct Pass<'a, 'p, 't, M> {
    theme: &'t Theme,
    metrics: &'t M,
    scale: f32,
    solved: &'a Solved,
    interface: &'a Interface,
    strings: &'a StringTable,
    out: &'p mut Vec<Primitive<'a>>,
    /// Multiplies every colour's alpha, so a whole screen fades from one place.
    opacity: f32,
    /// Moves every rectangle and every clip, in physical pixels.
    shift: [f32; 2],
}

/// What an enclosing scrollable container imposes on everything inside it.
#[derive(Debug, Clone, Copy)]
struct Frame {
    /// One past the last node this applies to.
    end: usize,
    /// How far up the contents have been scrolled, in physical pixels.
    offset: f32,
    /// The region the contents are confined to.
    clip: Rect,
}

impl<'a, M: Measure> Pass<'a, '_, '_, M> {
    /// Emits every node in drawing order, carrying scroll offsets and clips down the tree.
    ///
    /// Iterative rather than recursive over a flat pre-order sequence: the solver already flattened the
    /// tree, and a stack of enclosing frames popped by index reproduces the nesting without a second
    /// traversal. A node's descendants are the `subtree - 1` entries after it, which is what makes the
    /// end of a frame arithmetic.
    fn walk(&mut self, bounds: Rect) {
        // The clip starts at the shifted viewport rather than the viewport, so a screen moved sideways is
        // confined to where it has moved *to*. Left unshifted it would be free to draw over whatever slid
        // in beside it.
        let bounds = bounds.translated(self.shift[0], self.shift[1]);
        let mut frames: Vec<Frame> = Vec::new();
        for index in 0..self.solved.len() {
            while frames.last().is_some_and(|frame| index >= frame.end) {
                frames.pop();
            }
            let (offset, clip) = frames
                .last()
                .map_or((0.0, bounds), |frame| (frame.offset, frame.clip));
            let Some(node) = self.solved.get(index) else {
                continue;
            };
            let rect = node
                .rect
                .translated(self.shift[0], self.shift[1] - offset)
                .snapped();
            // A tab page that is not the chosen one is solved but not shown. Its rectangles are real and
            // correct for when its tab *is* chosen, so the only thing that stops it drawing over the page in
            // front of it is this — and hit testing and focus order skip it by the same flag, which is what
            // keeps "on screen" one answer rather than three.
            if node.visible && !clip.intersection(rect).is_empty() {
                self.node(index, node, rect, clip);
            }
            // Pushed whether or not the container itself was visible, so a child that *is* visible still
            // gets the right offset. A clip that has collapsed simply drops everything inside it.
            if node.widget == Widget::Scroll {
                let inner = node
                    .id
                    .as_deref()
                    .map_or(0.0, |id| self.interface.scroll(id));
                frames.push(Frame {
                    end: index + node.subtree,
                    offset: offset + inner,
                    clip: clip.intersection(rect),
                });
            }
        }
    }

    /// Emits one node's own drawing.
    fn node(&mut self, index: usize, node: &'a SolvedNode, rect: Rect, clip: Rect) {
        match node.widget {
            Widget::Panel => self.surface(node, rect, clip),
            Widget::Label => self.label(node, rect, clip),
            Widget::Button => self.button(node, rect, clip),
            Widget::Checkbox => self.checkbox(node, rect, clip),
            Widget::Slider => self.slider(node, rect, clip),
            Widget::TextEntry => self.entry(node, rect, clip),
            Widget::List | Widget::Tabs => self.selector(index, node, rect, clip),
            Widget::Scroll => self.scrollbar(index, node, rect, clip),
        }
    }

    /// A panel, which draws only what its role asks for.
    fn surface(&mut self, node: &SolvedNode, rect: Rect, clip: Rect) {
        match node.style {
            Some(Style::Scrim) => self.fill(clip, rect, self.theme.scrim),
            Some(Style::Card) => {
                self.fill(clip, rect, self.theme.card);
                self.stroke(clip, rect, self.theme.border);
            }
            Some(Style::Divider) => self.fill(clip, rect, self.theme.border),
            // Structure. The common case, and it draws nothing at all.
            _ => {}
        }
    }

    fn label(&mut self, node: &'a SolvedNode, rect: Rect, clip: Rect) {
        let (colour, size) = self.theme.text_role(node.style);
        let default = if node.style == Some(Style::Title) {
            TextAlign::Center
        } else {
            TextAlign::Leading
        };
        self.text(clip, rect, node, colour, size, default);
    }

    fn button(&mut self, node: &'a SolvedNode, rect: Rect, clip: Rect) {
        let id = node.id.as_deref();
        let face = if id.is_some() && id == self.interface.armed() {
            self.theme.control_armed
        } else if id.is_some() && id == self.interface.hover() {
            self.theme.control_hovered
        } else {
            self.theme.control
        };
        self.fill(clip, rect, face);
        self.outline(clip, rect, node);
        self.text(
            clip,
            rect,
            node,
            self.theme.text,
            self.theme.text_size,
            TextAlign::Center,
        );
    }

    fn checkbox(&mut self, node: &'a SolvedNode, rect: Rect, clip: Rect) {
        // A square just larger than the text beside it, so the indicator reads as belonging to the
        // label rather than as a separate element that happens to be next to it.
        let side = rect.height.min(self.physical(self.theme.text_size) * 1.2);
        let box_rect = Rect::new(rect.x, rect.y + (rect.height - side) / 2.0, side, side).snapped();
        let hovered = node.id.as_deref().is_some() && node.id.as_deref() == self.interface.hover();
        self.fill(
            clip,
            box_rect,
            if hovered {
                self.theme.control_hovered
            } else {
                self.theme.control
            },
        );
        self.outline(clip, box_rect, node);
        // The mark is inset rather than an authored glyph, because a tick would need the font this
        // module deliberately cannot reach.
        if node
            .id
            .as_deref()
            .is_some_and(|id| self.interface.toggle(id).unwrap_or(false))
        {
            let mark = box_rect.inset(crate::geometry::Insets::uniform(side * 0.28));
            self.fill(clip, mark.snapped(), self.theme.accent);
        }
        let gap = self.physical(self.theme.indicator_gap);
        let label = Rect::new(
            box_rect.right() + gap,
            rect.y,
            (rect.right() - box_rect.right() - gap).max(0.0),
            rect.height,
        );
        self.text(
            clip,
            label,
            node,
            self.theme.text,
            self.theme.text_size,
            TextAlign::Leading,
        );
    }

    fn slider(&mut self, node: &SolvedNode, rect: Rect, clip: Rect) {
        let Some(range) = node.range else { return };
        let thickness = self.physical(self.theme.track_thickness);
        let knob_width = self.physical(self.theme.knob_width).min(rect.width);
        let track = Rect::new(
            rect.x,
            rect.y + (rect.height - thickness) / 2.0,
            rect.width,
            thickness,
        )
        .snapped();
        self.fill(clip, track, self.theme.track);

        let value = node
            .id
            .as_deref()
            .and_then(|id| self.interface.slide(id))
            .unwrap_or(range.min);
        // The knob's *travel* is the track less the knob's own width, so the knob stays inside the
        // control at both ends rather than hanging half out of it.
        let travel = (rect.width - knob_width).max(0.0);
        let at = rect.x + travel * range.fraction(value);
        let filled = Rect::new(
            track.x,
            track.y,
            (at + knob_width / 2.0 - track.x).max(0.0),
            track.height,
        );
        self.fill(clip, filled.snapped(), self.theme.accent);
        let knob = Rect::new(at, rect.y, knob_width, rect.height).snapped();
        let hovered = node.id.as_deref().is_some() && node.id.as_deref() == self.interface.hover();
        self.fill(
            clip,
            knob,
            if hovered {
                self.theme.control_hovered
            } else {
                self.theme.control
            },
        );
        self.outline(clip, knob, node);
    }

    fn entry(&mut self, node: &'a SolvedNode, rect: Rect, clip: Rect) {
        self.fill(clip, rect, self.theme.control_armed);
        self.outline(clip, rect, node);
        let inset = self.physical(self.theme.text_inset);
        let inner = Rect::new(
            rect.x + inset,
            rect.y,
            (rect.width - inset * 2.0).max(0.0),
            rect.height,
        );
        let size = self.physical(self.theme.text_size);
        let Some(field) = node.id.as_deref().and_then(|id| self.interface.text(id)) else {
            return;
        };
        self.push(
            clip,
            Content::Text {
                rect: inner,
                // The text to *draw*, which includes the uncommitted composition. Drawing the value
                // instead would hide what the user is in the middle of typing.
                text: field.text(),
                colour: self.theme.text,
                size,
                align: TextAlign::Leading,
            },
        );
        // An uncommitted composition is underlined, which is the convention that tells a user this is
        // not real text yet. Marking a span of one string is why the composition lives inside the field.
        if let Some(composing) = field.composition() {
            let from = self.advance(character_prefix(field.text(), composing.start));
            let to = self.advance(character_prefix(field.text(), composing.end));
            let thickness = self.physical(self.theme.border_width).max(1.0);
            let underline = Rect::new(
                inner.x + from,
                rect.bottom() - inset.max(thickness * 2.0),
                (to - from).max(thickness),
                thickness,
            );
            self.fill(clip, underline.snapped(), self.theme.composition);
        }
        if node.id.as_deref() == self.interface.focus() {
            let at = self.advance(character_prefix(field.text(), field.cursor()));
            let width = self.physical(self.theme.caret_width).max(1.0);
            let height = (rect.height - inset).max(size);
            let caret = Rect::new(
                inner.x + at,
                rect.y + (rect.height - height) / 2.0,
                width,
                height,
            );
            self.fill(clip, caret.snapped(), self.theme.caret);
        }
    }

    /// A list or a tab strip: its own frame, plus a highlight behind the chosen child.
    ///
    /// The highlight is emitted here rather than when the child is reached, because the child is drawn
    /// after its parent and a highlight drawn on top of a row would cover the row's text.
    fn selector(&mut self, index: usize, node: &SolvedNode, rect: Rect, clip: Rect) {
        self.fill(clip, rect, self.theme.control_armed);
        self.outline(clip, rect, node);
        let chosen = node
            .id
            .as_deref()
            .and_then(|id| self.interface.selection(id))
            .unwrap_or(0);
        if let Some(child) = self
            .solved
            .child(index, chosen)
            .and_then(|at| self.solved.get(at))
        {
            self.fill(clip, child.rect, self.theme.accent);
        }
    }

    /// A scrollable container, which draws nothing but an indicator of how far along it is.
    ///
    /// Only when there is something to scroll. A bar that is always there and sometimes cannot move is
    /// the same information as no bar, plus a distraction.
    fn scrollbar(&mut self, index: usize, node: &SolvedNode, rect: Rect, clip: Rect) {
        let limit = self.solved.scroll_limit(index);
        if limit <= 0.0 || rect.height <= 0.0 {
            return;
        }
        let offset = node
            .id
            .as_deref()
            .map_or(0.0, |id| self.interface.scroll(id));
        let width = self.physical(self.theme.scrollbar_width);
        // The thumb's share of the bar is the view's share of the content, which is the one relation
        // that makes a scrollbar's length mean something.
        let thumb = (rect.height * rect.height / (rect.height + limit)).max(width);
        let travel = (rect.height - thumb).max(0.0);
        let at = rect.y + travel * (offset / limit).clamp(0.0, 1.0);
        let bar = Rect::new(rect.right() - width, at, width, thumb);
        self.fill(clip, bar.snapped(), self.theme.track);
    }

    /// A focus ring where focused, and the ordinary border otherwise.
    ///
    /// One replaces the other rather than sitting inside it, so a focused control changes colour in
    /// place instead of growing by a pixel and nudging everything around it.
    fn outline(&mut self, clip: Rect, rect: Rect, node: &SolvedNode) {
        let focused = node.id.as_deref().is_some() && node.id.as_deref() == self.interface.focus();
        let colour = if focused {
            self.theme.focus
        } else {
            self.theme.border
        };
        self.stroke(clip, rect, colour);
    }

    /// A rectangle's outline, as four fills.
    ///
    /// Four fills rather than a primitive of its own, because a consumer that knows how to draw a quad
    /// already knows how to draw a border and should not have to learn twice.
    fn stroke(&mut self, clip: Rect, rect: Rect, colour: Colour) {
        let width = self.physical(self.theme.border_width).max(1.0);
        if colour.is_invisible() || rect.width < width || rect.height < width {
            return;
        }
        for side in [
            Rect::new(rect.x, rect.y, rect.width, width),
            Rect::new(rect.x, rect.bottom() - width, rect.width, width),
            Rect::new(rect.x, rect.y + width, width, rect.height - width * 2.0),
            Rect::new(
                rect.right() - width,
                rect.y + width,
                width,
                rect.height - width * 2.0,
            ),
        ] {
            self.fill(clip, side.snapped(), colour);
        }
    }

    /// A node's own text, when it has any.
    fn text(
        &mut self,
        clip: Rect,
        rect: Rect,
        node: &'a SolvedNode,
        colour: Colour,
        size: f32,
        default: TextAlign,
    ) {
        let Some(text) = self.text_of(node) else {
            return;
        };
        if text.is_empty() || rect.is_empty() {
            return;
        }
        self.push(
            clip,
            Content::Text {
                rect,
                text,
                colour,
                size: self.physical(size),
                align: align_of(node.align, default),
            },
        );
    }

    /// What text a node draws: whatever the host stored against its id, or its key resolved.
    ///
    /// The stored value wins, and that is the channel for text nobody can put in a string table — a
    /// countdown, a chosen map's name, a player's typed handle. Without it a host would have to write
    /// into the string table, where a per-frame value does not belong.
    fn text_of(&self, node: &'a SolvedNode) -> Option<&'a str> {
        if let Some(field) = node.id.as_deref().and_then(|id| self.interface.text(id)) {
            return Some(field.text());
        }
        node.text_key.as_deref().map(|key| self.strings.text(key))
    }

    /// The physical width of a string at the theme's body size.
    fn advance(&self, text: &str) -> f32 {
        self.metrics.advance(text, self.theme.text_size) * self.scale
    }

    /// A logical measurement in physical pixels.
    fn physical(&self, logical: f32) -> f32 {
        logical * self.scale
    }

    fn fill(&mut self, clip: Rect, rect: Rect, colour: Colour) {
        if colour.is_invisible() || rect.is_empty() {
            return;
        }
        self.push(clip, Content::Fill { rect, colour });
    }

    fn push(&mut self, clip: Rect, content: Content<'a>) {
        // One place, so a screen's opacity cannot reach some of its primitives and miss others -- which is
        // exactly what would happen if each widget applied it for itself.
        let content = if self.opacity >= 1.0 {
            content
        } else {
            faded(&content, self.opacity)
        };
        self.out.push(Primitive { clip, content });
    }
}

/// Scales a primitive's alpha, for a screen partway through a change.
fn faded<'a>(content: &Content<'a>, opacity: f32) -> Content<'a> {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let scale = |colour: Colour| {
        colour.with_alpha(
            (f32::from(colour.alpha) * opacity)
                .round()
                .clamp(0.0, 255.0) as u8,
        )
    };
    match *content {
        Content::Fill { rect, colour } => Content::Fill {
            rect,
            colour: scale(colour),
        },
        Content::Text {
            rect,
            text,
            colour,
            size,
            align,
        } => Content::Text {
            rect,
            text,
            colour: scale(colour),
            size,
            align,
        },
    }
}

/// Where text sits: what the layout asked for, or the widget kind's own default.
///
/// `align` defaults to `Stretch`, which is what makes this unambiguous. On a node with children it
/// governs the cross axis; on one without, there is nothing to stretch, so an author writing
/// `align: "center"` on a label can only have meant the text — while a defaulted `Stretch` leaves the
/// widget kind to decide, so a button's label is centred without every button having to say so.
fn align_of(align: Align, default: TextAlign) -> TextAlign {
    match align {
        Align::Start => TextAlign::Leading,
        Align::Center => TextAlign::Center,
        Align::End => TextAlign::Trailing,
        Align::Stretch => default,
    }
}

/// The prefix of `text` up to a **character** index, without allocating.
///
/// Character indices throughout, because that is what a cursor is. Slicing by the index directly puts
/// the boundary inside a multi-byte character and panics, which is the failure the whole convention
/// exists to avoid.
fn character_prefix(text: &str, characters: usize) -> &str {
    let offset = text
        .char_indices()
        .nth(characters)
        .map_or(text.len(), |(offset, _)| offset);
    &text[..offset]
}

#[cfg(test)]
mod tests {
    // Every figure compared below is either an integer pixel coordinate produced by snapping or an
    // exactly-representable product of small binary fractions, so exact comparison is the assertion.
    #![allow(clippy::float_cmp)]

    use super::{Colour, Content, Painter, Primitive, TextAlign, Theme, character_prefix};
    use crate::geometry::Rect;
    use crate::layout::{
        Align, Direction, FORMAT_VERSION, Layout, Node, Range, Sizing, Style, Widget,
    };
    use crate::solve::{Measure, Solved, solve, solve_selected};
    use crate::state::Interface;
    use crate::transition::Reveal;
    use crate::{StringTable, Viewport};

    /// A fixed-width stand-in for a font: every character is half the text size wide.
    ///
    /// Fixed width on purpose. A proportional stub would make every expected caret position depend on
    /// which letters the test happened to type, which tests the arithmetic of the stub rather than the
    /// arithmetic being checked.
    struct Monospace;

    impl Measure for Monospace {
        fn measure(&self, node: &Node, _available: [f32; 2]) -> [f32; 2] {
            let characters = node.text_key.as_ref().map_or(0, String::len);
            #[allow(clippy::cast_precision_loss)]
            [characters as f32 * 8.0, 16.0]
        }

        fn advance(&self, text: &str, size: f32) -> f32 {
            #[allow(clippy::cast_precision_loss)]
            {
                text.chars().count() as f32 * size / 2.0
            }
        }
    }

    fn viewport() -> Viewport {
        Viewport::new(400, 200, 1.0).expect("viewport")
    }

    fn solved(root: Node) -> Solved {
        let layout = Layout {
            format_version: FORMAT_VERSION,
            root,
        };
        solve(&layout, viewport(), &Monospace)
    }

    /// Paints one screen with the default theme against a plain string table.
    fn paint<'a>(
        solved: &'a Solved,
        interface: &'a Interface,
        strings: &'a StringTable,
        theme: &'a Theme,
    ) -> Vec<Primitive<'a>> {
        Painter::new(theme, &Monospace, viewport()).paint(solved, interface, strings)
    }

    fn fills(primitives: &[Primitive<'_>]) -> Vec<(Rect, Colour)> {
        primitives
            .iter()
            .filter_map(|primitive| match primitive.content {
                Content::Fill { rect, colour } => Some((rect, colour)),
                Content::Text { .. } => None,
            })
            .collect()
    }

    fn texts<'a>(primitives: &[Primitive<'a>]) -> Vec<(&'a str, f32, TextAlign, Rect)> {
        primitives
            .iter()
            .filter_map(|primitive| match primitive.content {
                Content::Text {
                    text,
                    size,
                    align,
                    rect,
                    ..
                } => Some((text, size, align, rect)),
                Content::Fill { .. } => None,
            })
            .collect()
    }

    #[test]
    fn a_tab_page_that_is_not_showing_draws_nothing() {
        // The third consumer of the solver's visibility flag, after hit testing and focus order. All three
        // pages are solved and all three overlap, so a walk that drew every node would paint the last one
        // over the chosen one and the screen would show whichever tab was authored last whatever was picked.
        let page = |key: &str| Node {
            width: Sizing::Fill(1),
            height: Sizing::Fill(1),
            children: vec![Node {
                widget: Widget::Label,
                text_key: Some(key.to_owned()),
                ..Node::default()
            }],
            ..Node::default()
        };
        let layout = Layout {
            format_version: FORMAT_VERSION,
            root: Node {
                width: Sizing::Fill(1),
                height: Sizing::Fill(1),
                children: vec![
                    Node {
                        id: Some("strip".to_owned()),
                        widget: Widget::Tabs,
                        height: Sizing::Fixed(20.0),
                        pages: Some("pages".to_owned()),
                        children: vec![Node::default(), Node::default()],
                        ..Node::default()
                    },
                    Node {
                        id: Some("pages".to_owned()),
                        direction: Direction::Stack,
                        width: Sizing::Fill(1),
                        height: Sizing::Fill(1),
                        children: vec![page("first"), page("second")],
                        ..Node::default()
                    },
                ],
                ..Node::default()
            },
        };
        layout.validate().expect("the fixture must be valid");

        let mut interface = Interface::new();
        interface.set_selection("strip", 1);
        let solved = solve_selected(&layout, viewport(), &Monospace, &interface);
        let strings = StringTable::new();
        let theme = Theme::default();
        let drawn: Vec<&str> = texts(&paint(&solved, &interface, &strings, &theme))
            .into_iter()
            .map(|(text, ..)| text)
            .collect();
        assert_eq!(drawn, vec!["second"]);
    }

    #[test]
    fn srgb_bytes_leave_as_linear_floats() {
        // The conversion that is invisible in a test comparing numbers to themselves. Passing
        // byte / 255 straight through makes every surface too bright on an sRGB target.
        let white = Colour::rgb(0xff, 0xff, 0xff).to_linear();
        assert_eq!(white, [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(Colour::rgb(0, 0, 0).to_linear(), [0.0, 0.0, 0.0, 1.0]);
        // Mid grey is the case that separates the two implementations: 0.5 encoded is about 0.214
        // linear, not 0.5.
        let grey = Colour::rgb(0x80, 0x80, 0x80).to_linear();
        assert!(
            (grey[0] - 0.215_86).abs() < 1e-4,
            "mid grey linearised to {}",
            grey[0]
        );
        // Alpha is coverage, not intensity, so no transfer function applies to it.
        let half = Colour::rgba(0xff, 0xff, 0xff, 0x80).to_linear();
        assert!((half[3] - 0.501_96).abs() < 1e-5, "alpha was {}", half[3]);
    }

    #[test]
    fn a_packed_hex_colour_unpacks_in_the_order_it_reads() {
        assert_eq!(
            Colour::hex(0x11_22_33_44),
            Colour::rgba(0x11, 0x22, 0x33, 0x44)
        );
        assert_eq!(Colour::hex(0xff_00_00_ff), Colour::rgb(0xff, 0, 0));
        assert!(Colour::NONE.is_invisible());
        assert!(Colour::rgb(0, 0, 0).with_alpha(0).is_invisible());
    }

    #[test]
    fn structure_draws_nothing_at_all() {
        // The default and the common case. Giving every panel a background would paint the screen over
        // in nested rectangles.
        let solved = solved(Node {
            width: Sizing::Fill(1),
            height: Sizing::Fill(1),
            children: vec![Node::default(), Node::default()],
            ..Node::default()
        });
        let (interface, strings, theme) = (Interface::new(), StringTable::new(), Theme::default());
        let painted = paint(&solved, &interface, &strings, &theme);
        assert!(painted.is_empty(), "structure painted {painted:?}");
    }

    #[test]
    fn a_card_draws_a_face_and_four_border_sides() {
        let theme = Theme::default();
        let solved = solved(Node {
            style: Some(Style::Card),
            width: Sizing::Fixed(100.0),
            height: Sizing::Fixed(50.0),
            ..Node::default()
        });
        let (interface, strings) = (Interface::new(), StringTable::new());
        let painted = paint(&solved, &interface, &strings, &theme);
        let drawn = fills(&painted);
        assert_eq!(drawn.len(), 5, "a face and four sides: {drawn:?}");
        assert_eq!(drawn[0], (Rect::new(0.0, 0.0, 100.0, 50.0), theme.card));
        // The four sides together enclose the box, and each is a whole pixel thick.
        for (rect, colour) in &drawn[1..] {
            assert_eq!(*colour, theme.border);
            assert!(rect.width >= 1.0 && rect.height >= 1.0, "{rect:?}");
        }
        assert_eq!(drawn[1].0, Rect::new(0.0, 0.0, 100.0, 1.0));
        assert_eq!(drawn[2].0, Rect::new(0.0, 49.0, 100.0, 1.0));
    }

    #[test]
    fn a_labels_role_chooses_its_colour_and_size() {
        let theme = Theme::default();
        let mut strings = StringTable::new();
        strings.set("t", "Title");
        strings.set("c", "Caption");
        strings.set("w", "Reverting");
        let label = |style: Option<Style>, key: &str| Node {
            widget: Widget::Label,
            style,
            text_key: Some(key.to_owned()),
            width: Sizing::Fill(1),
            ..Node::default()
        };
        let solved = solved(Node {
            width: Sizing::Fill(1),
            height: Sizing::Fill(1),
            children: vec![
                label(Some(Style::Title), "t"),
                label(Some(Style::Caption), "c"),
                label(Some(Style::Warning), "w"),
            ],
            ..Node::default()
        });
        let interface = Interface::new();
        let painted = paint(&solved, &interface, &strings, &theme);
        let drawn = texts(&painted);
        assert_eq!(drawn[0].0, "Title");
        assert_eq!(drawn[0].1, theme.title_size);
        // A title centres itself; the others read from the leading edge.
        assert_eq!(drawn[0].2, TextAlign::Center);
        assert_eq!(drawn[1].1, theme.caption_size);
        assert_eq!(drawn[1].2, TextAlign::Leading);
        assert_eq!(drawn[2].1, theme.text_size);
    }

    #[test]
    fn an_authored_alignment_wins_and_a_defaulted_one_leaves_the_widget_to_decide() {
        // `align` defaults to Stretch, which is what makes this unambiguous: on a childless node there
        // is nothing to stretch, so `center` can only mean the text.
        let theme = Theme::default();
        let mut strings = StringTable::new();
        strings.set("k", "text");
        let solved = solved(Node {
            width: Sizing::Fill(1),
            height: Sizing::Fill(1),
            children: vec![
                Node {
                    widget: Widget::Label,
                    align: Align::Center,
                    text_key: Some("k".to_owned()),
                    ..Node::default()
                },
                Node {
                    id: Some("b".to_owned()),
                    widget: Widget::Button,
                    text_key: Some("k".to_owned()),
                    ..Node::default()
                },
                Node {
                    id: Some("e".to_owned()),
                    widget: Widget::Button,
                    align: Align::End,
                    text_key: Some("k".to_owned()),
                    ..Node::default()
                },
            ],
            ..Node::default()
        });
        let interface = Interface::new();
        let painted = paint(&solved, &interface, &strings, &theme);
        let drawn = texts(&painted);
        assert_eq!(drawn[0].2, TextAlign::Center, "an authored centre wins");
        assert_eq!(drawn[1].2, TextAlign::Center, "a button centres by default");
        assert_eq!(drawn[2].2, TextAlign::Trailing, "and can be overridden");
    }

    #[test]
    fn a_buttons_face_follows_hover_and_arming() {
        let theme = Theme::default();
        let solved = solved(Node {
            id: Some("play".to_owned()),
            widget: Widget::Button,
            width: Sizing::Fixed(80.0),
            height: Sizing::Fixed(20.0),
            ..Node::default()
        });
        let strings = StringTable::new();
        let mut interface = Interface::new();
        assert_eq!(
            fills(&paint(&solved, &interface, &strings, &theme))[0].1,
            theme.control
        );
        interface.handle(&solved, crate::UiEvent::PointerMoved { x: 4.0, y: 4.0 });
        assert_eq!(
            fills(&paint(&solved, &interface, &strings, &theme))[0].1,
            theme.control_hovered
        );
        interface.handle(&solved, crate::UiEvent::PointerPressed { x: 4.0, y: 4.0 });
        assert_eq!(
            fills(&paint(&solved, &interface, &strings, &theme))[0].1,
            theme.control_armed
        );
    }

    #[test]
    fn a_focus_ring_replaces_the_border_rather_than_sitting_outside_it() {
        // In place, so a focused control changes colour instead of growing by a pixel and nudging
        // everything around it.
        let theme = Theme::default();
        let solved = solved(Node {
            id: Some("play".to_owned()),
            widget: Widget::Button,
            width: Sizing::Fixed(80.0),
            height: Sizing::Fixed(20.0),
            ..Node::default()
        });
        let strings = StringTable::new();
        let mut interface = Interface::new();
        let unfocused = fills(&paint(&solved, &interface, &strings, &theme));
        interface.set_focus(Some("play"));
        let focused = fills(&paint(&solved, &interface, &strings, &theme));
        assert_eq!(unfocused.len(), focused.len(), "no extra rectangles");
        for (before, after) in unfocused.iter().zip(&focused) {
            assert_eq!(before.0, after.0, "the geometry must not move");
        }
        assert!(
            focused[1..]
                .iter()
                .all(|(_, colour)| *colour == theme.focus)
        );
    }

    #[test]
    fn a_checkbox_marks_itself_only_when_it_is_on() {
        let theme = Theme::default();
        let solved = solved(Node {
            id: Some("vsync".to_owned()),
            widget: Widget::Checkbox,
            width: Sizing::Fixed(200.0),
            height: Sizing::Fixed(24.0),
            ..Node::default()
        });
        let strings = StringTable::new();
        let mut interface = Interface::new();
        interface.set_toggle("vsync", false);
        let off = fills(&paint(&solved, &interface, &strings, &theme));
        interface.set_toggle("vsync", true);
        let on = fills(&paint(&solved, &interface, &strings, &theme));
        assert_eq!(on.len(), off.len() + 1, "the mark is the one extra fill");
        let (mark, colour) = *on.last().expect("a mark");
        assert_eq!(colour, theme.accent);
        // Inside the indicator, which sits at the leading edge rather than across the whole control.
        assert!(mark.x > 0.0 && mark.right() < 24.0, "{mark:?}");
    }

    #[test]
    fn a_sliders_knob_stays_inside_the_control_at_both_ends() {
        // The failure this arithmetic exists to avoid: a knob positioned by the fraction alone hangs
        // half out of the control at the maximum.
        let theme = Theme::default();
        let solved = solved(Node {
            id: Some("scale".to_owned()),
            widget: Widget::Slider,
            width: Sizing::Fixed(200.0),
            height: Sizing::Fixed(20.0),
            range: Some(Range {
                min: 0.5,
                max: 2.0,
                step: 0.25,
            }),
            ..Node::default()
        });
        let strings = StringTable::new();
        let mut interface = Interface::new();
        for (value, expected) in [(0.5, 0.0), (2.0, 200.0 - theme.knob_width)] {
            interface.set_slide("scale", value);
            let drawn = fills(&paint(&solved, &interface, &strings, &theme));
            let knob = drawn
                .iter()
                .find(|(rect, _)| rect.width == theme.knob_width)
                .expect("a knob");
            assert_eq!(knob.0.x, expected, "at {value}");
            assert!(knob.0.right() <= 200.0, "the knob left the control");
        }
    }

    #[test]
    fn a_text_entry_draws_the_composition_and_marks_it() {
        // Two readers, and using the wrong one is a real bug: what is drawn includes the composition.
        let theme = Theme::default();
        let solved = solved(Node {
            id: Some("name".to_owned()),
            widget: Widget::TextEntry,
            width: Sizing::Fixed(300.0),
            height: Sizing::Fixed(24.0),
            ..Node::default()
        });
        let strings = StringTable::new();
        let mut interface = Interface::new();
        interface.set_text("name", "ab");
        interface.set_focus(Some("name"));
        interface.handle(
            &solved,
            crate::UiEvent::Compose {
                text: "ni".to_owned(),
                cursor: None,
            },
        );
        let painted = paint(&solved, &interface, &strings, &theme);
        let drawn = texts(&painted);
        assert_eq!(drawn[0].0, "abni", "the composition is drawn, not hidden");
        // The underline covers exactly the composed characters: two of them, at 8 physical pixels each.
        let underline = fills(&painted)
            .into_iter()
            .find(|(_, colour)| *colour == theme.composition)
            .expect("a composition underline");
        assert_eq!(underline.0.x, theme.text_inset + 16.0);
        assert_eq!(underline.0.width, 16.0);
    }

    #[test]
    fn a_caret_sits_at_the_cursor_and_only_while_focused() {
        let theme = Theme::default();
        let solved = solved(Node {
            id: Some("name".to_owned()),
            widget: Widget::TextEntry,
            width: Sizing::Fixed(300.0),
            height: Sizing::Fixed(24.0),
            ..Node::default()
        });
        let strings = StringTable::new();
        let mut interface = Interface::new();
        interface.set_text("name", "abcd");
        assert!(
            !fills(&paint(&solved, &interface, &strings, &theme))
                .iter()
                .any(|(_, colour)| *colour == theme.caret),
            "an unfocused field has no caret"
        );
        interface.set_focus(Some("name"));
        let caret = fills(&paint(&solved, &interface, &strings, &theme))
            .into_iter()
            .find(|(_, colour)| *colour == theme.caret)
            .expect("a caret");
        // Four characters at 8 physical pixels each, past the field's own inset.
        assert_eq!(caret.0.x, theme.text_inset + 32.0);
    }

    #[test]
    fn the_input_method_cursor_area_narrows_from_the_field_to_the_caret() {
        // What the metrics buy. On a wide field the candidate window would otherwise appear beside the
        // box rather than beside the character being composed.
        let theme = Theme::default();
        let solved = solved(Node {
            id: Some("name".to_owned()),
            widget: Widget::TextEntry,
            width: Sizing::Fixed(300.0),
            height: Sizing::Fixed(24.0),
            ..Node::default()
        });
        let mut interface = Interface::new();
        interface.set_text("name", "abcdefgh");
        interface.set_focus(Some("name"));
        let field = interface.ime_cursor_area(&solved).expect("a field");
        assert_eq!(field.width, 300.0);
        let painter = Painter::new(&theme, &Monospace, viewport());
        let caret = painter
            .ime_cursor_area(&solved, &interface)
            .expect("a caret area");
        assert!(caret.width < field.width, "{caret:?} is not narrower");
        assert_eq!(caret.x, theme.text_inset + 64.0);
    }

    #[test]
    fn a_scroll_offset_moves_the_contents_and_clips_them_to_the_container() {
        let theme = Theme::default();
        let row = |key: &str| Node {
            style: Some(Style::Card),
            width: Sizing::Fill(1),
            height: Sizing::Fixed(40.0),
            text_key: Some(key.to_owned()),
            ..Node::default()
        };
        let solved = solved(Node {
            id: Some("list".to_owned()),
            widget: Widget::Scroll,
            width: Sizing::Fixed(200.0),
            height: Sizing::Fixed(60.0),
            direction: Direction::Column,
            children: vec![row("a"), row("b"), row("c")],
            ..Node::default()
        });
        let strings = StringTable::new();
        let mut interface = Interface::new();
        let at_rest = fills(&paint(&solved, &interface, &strings, &theme));
        interface.set("list", crate::state::Value::Scroll(30.0));
        let scrolled = paint(&solved, &interface, &strings, &theme);
        let faces: Vec<Rect> = fills(&scrolled)
            .into_iter()
            .filter(|(_, colour)| *colour == theme.card)
            .map(|(rect, _)| rect)
            .collect();
        // Every row moved up by the offset.
        assert_eq!(faces[0].y, -30.0);
        assert_eq!(faces[1].y, 10.0);
        // And everything inside is confined to the container. Its own scrollbar is not: that belongs to
        // the container rather than to its contents, and it sits on the edge it would be clipped by.
        for primitive in &scrolled {
            let inside = matches!(
                primitive.content,
                Content::Fill { colour, .. } if colour == theme.card
            ) || matches!(primitive.content, Content::Text { .. });
            if inside {
                assert_eq!(
                    primitive.clip,
                    Rect::new(0.0, 0.0, 200.0, 60.0),
                    "{:?} escaped the container",
                    primitive.content
                );
            }
        }
        // A bar appears only once there is something to scroll, which is true here in both states.
        assert!(
            at_rest.iter().any(|(_, colour)| *colour == theme.track),
            "an overflowing container needs an indicator"
        );
    }

    #[test]
    fn a_container_that_fits_its_contents_draws_no_scrollbar() {
        // A bar that is always there and sometimes cannot move is the same information as no bar.
        let theme = Theme::default();
        let solved = solved(Node {
            id: Some("list".to_owned()),
            widget: Widget::Scroll,
            width: Sizing::Fixed(200.0),
            height: Sizing::Fixed(200.0),
            children: vec![Node {
                style: Some(Style::Card),
                width: Sizing::Fill(1),
                height: Sizing::Fixed(20.0),
                ..Node::default()
            }],
            ..Node::default()
        });
        let (interface, strings) = (Interface::new(), StringTable::new());
        let painted = paint(&solved, &interface, &strings, &theme);
        assert!(
            !fills(&painted)
                .iter()
                .any(|(_, colour)| *colour == theme.track),
            "nothing to scroll, so nothing to indicate"
        );
    }

    #[test]
    fn a_selectors_highlight_goes_behind_the_chosen_child_and_not_over_it() {
        // Children are drawn after their parent, so a highlight emitted when the child is reached would
        // cover the row's own text.
        let theme = Theme::default();
        let solved = solved(Node {
            id: Some("maps".to_owned()),
            widget: Widget::List,
            width: Sizing::Fixed(200.0),
            height: Sizing::Fixed(90.0),
            children: (0..3)
                .map(|index| Node {
                    widget: Widget::Label,
                    width: Sizing::Fill(1),
                    height: Sizing::Fixed(30.0),
                    text_key: Some(format!("row{index}")),
                    ..Node::default()
                })
                .collect(),
            ..Node::default()
        });
        let strings = StringTable::new();
        let mut interface = Interface::new();
        interface.set_selection("maps", 1);
        let painted = paint(&solved, &interface, &strings, &theme);
        let highlight = fills(&painted)
            .into_iter()
            .find(|(_, colour)| *colour == theme.accent)
            .expect("a highlight");
        assert_eq!(highlight.0, Rect::new(0.0, 30.0, 200.0, 30.0));
        // It comes before the row's text in the list, which is what puts it underneath.
        let highlight_at = painted
            .iter()
            .position(|primitive| {
                matches!(primitive.content, Content::Fill { colour, .. } if colour == theme.accent)
            })
            .expect("a highlight");
        let text_at = painted
            .iter()
            .position(|primitive| matches!(primitive.content, Content::Text { text, .. } if text == "row1"))
            .expect("the row's text");
        assert!(highlight_at < text_at, "the highlight would cover the text");
    }

    #[test]
    fn a_stored_value_overrides_a_labels_key_so_a_countdown_can_be_drawn() {
        // The channel for text nobody can put in a string table. Without it a host would have to write
        // a per-frame value into the table, where it does not belong.
        let theme = Theme::default();
        let mut strings = StringTable::new();
        strings.set("settings.countdown", "Reverting");
        let solved = solved(Node {
            id: Some("countdown".to_owned()),
            widget: Widget::Label,
            style: Some(Style::Warning),
            text_key: Some("settings.countdown".to_owned()),
            width: Sizing::Fill(1),
            ..Node::default()
        });
        let mut interface = Interface::new();
        assert_eq!(
            texts(&paint(&solved, &interface, &strings, &theme))[0].0,
            "Reverting"
        );
        interface.set_text("countdown", "Reverting in 12 s");
        assert_eq!(
            texts(&paint(&solved, &interface, &strings, &theme))[0].0,
            "Reverting in 12 s"
        );
    }

    #[test]
    fn a_missing_string_draws_its_own_key() {
        // The string table's posture, carried through to what is drawn: a blank button reads as a
        // rendering bug while `menu.absent` names its own fix.
        let theme = Theme::default();
        let solved = solved(Node {
            widget: Widget::Label,
            text_key: Some("menu.absent".to_owned()),
            width: Sizing::Fill(1),
            ..Node::default()
        });
        assert_eq!(
            texts(&paint(
                &solved,
                &Interface::new(),
                &StringTable::new(),
                &theme
            ))[0]
                .0,
            "menu.absent"
        );
    }

    #[test]
    fn a_scale_multiplies_every_measurement_the_theme_states() {
        // The charter's DPI item, reaching the drawing layer: a theme is authored in logical units and
        // everything out of the painter is physical.
        let theme = Theme::default();
        let layout = Layout {
            format_version: FORMAT_VERSION,
            root: Node {
                id: Some("name".to_owned()),
                widget: Widget::TextEntry,
                width: Sizing::Fixed(150.0),
                height: Sizing::Fixed(24.0),
                ..Node::default()
            },
        };
        let doubled = Viewport::new(400, 200, 2.0).expect("viewport");
        let solved = solve(&layout, doubled, &Monospace);
        let mut interface = Interface::new();
        interface.set_text("name", "ab");
        interface.set_focus(Some("name"));
        let strings = StringTable::new();
        let painted =
            Painter::new(&theme, &Monospace, doubled).paint(&solved, &interface, &strings);
        assert_eq!(texts(&painted)[0].1, theme.text_size * 2.0);
        let caret = fills(&painted)
            .into_iter()
            .find(|(_, colour)| *colour == theme.caret)
            .expect("a caret");
        // Two characters at 16 physical pixels each, past a doubled inset.
        assert_eq!(caret.0.x, theme.text_inset * 2.0 + 32.0);
        assert_eq!(caret.0.width, theme.caret_width * 2.0);
    }

    #[test]
    fn a_reveal_fades_every_primitive_from_one_place_and_moves_them_together() {
        // Applied where a primitive is pushed rather than by each widget, so a screen's opacity cannot
        // reach some of its primitives and miss others -- which is exactly what would happen if a
        // checkbox and a slider each did it for themselves.
        let theme = Theme::default();
        let solved = solved(Node {
            style: Some(Style::Card),
            width: Sizing::Fixed(100.0),
            height: Sizing::Fixed(50.0),
            ..Node::default()
        });
        let (interface, strings) = (Interface::new(), StringTable::new());
        let painter = Painter::new(&theme, &Monospace, viewport());
        let mut half = Vec::new();
        painter.paint_revealed(
            &mut half,
            &solved,
            &interface,
            &strings,
            Reveal {
                opacity: 0.5,
                // A twelfth of a 400-pixel viewport, so a third of a hundred-wide card.
                offset: [0.25, 0.0],
            },
        );
        let shown = painter.paint(&solved, &interface, &strings);
        assert_eq!(half.len(), shown.len(), "the same primitives, faded");
        for (faded, opaque) in fills(&half).iter().zip(fills(&shown)) {
            // Every rectangle moved by the same amount.
            assert_eq!(faded.0.x, opaque.0.x + 100.0);
            assert_eq!(faded.0.width, opaque.0.width);
            // And every colour's alpha halved, keeping its channels.
            assert_eq!(faded.1.red, opaque.1.red);
            assert_eq!(faded.1.alpha, 128);
        }
        // The clip moved with the screen, so a screen slid sideways cannot draw over what slid in beside it.
        assert_eq!(half[0].clip.x, 100.0);
    }

    #[test]
    fn a_screen_revealed_to_nothing_costs_no_primitives() {
        // A change that has faded one screen out should cost nothing rather than a screenful of invisible
        // rectangles the renderer has to blend.
        let theme = Theme::default();
        let solved = solved(Node {
            style: Some(Style::Card),
            width: Sizing::Fill(1),
            height: Sizing::Fill(1),
            ..Node::default()
        });
        let (interface, strings) = (Interface::new(), StringTable::new());
        let mut out = Vec::new();
        Painter::new(&theme, &Monospace, viewport()).paint_revealed(
            &mut out,
            &solved,
            &interface,
            &strings,
            Reveal {
                opacity: 0.0,
                offset: [0.0, 0.0],
            },
        );
        assert!(out.is_empty());
    }

    #[test]
    fn a_full_reveal_draws_exactly_what_no_reveal_would() {
        // The property that keeps every committed capture valid: a screen at rest goes through the same
        // path as one mid-change, and must come out identical.
        let theme = Theme::default();
        let mut strings = StringTable::new();
        strings.set("k", "Settings");
        let solved = solved(Node {
            id: Some("go".to_owned()),
            widget: Widget::Button,
            text_key: Some("k".to_owned()),
            width: Sizing::Fixed(120.0),
            height: Sizing::Fixed(36.0),
            ..Node::default()
        });
        let interface = Interface::new();
        let painter = Painter::new(&theme, &Monospace, viewport());
        let mut revealed = Vec::new();
        painter.paint_revealed(&mut revealed, &solved, &interface, &strings, Reveal::SHOWN);
        assert_eq!(revealed, painter.paint(&solved, &interface, &strings));
    }

    #[test]
    fn a_character_prefix_does_not_split_a_multi_byte_character() {
        // Slicing by the cursor index directly is what panics the first time somebody types one.
        let text = "aé中";
        assert_eq!(character_prefix(text, 0), "");
        assert_eq!(character_prefix(text, 1), "a");
        assert_eq!(character_prefix(text, 2), "aé");
        assert_eq!(character_prefix(text, 3), "aé中");
        // Past the end saturates rather than panicking.
        assert_eq!(character_prefix(text, 99), "aé中");
    }
}
