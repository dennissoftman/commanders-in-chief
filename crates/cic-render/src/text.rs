//! The interface font: authored outlines, a coverage rasteriser, and a glyph atlas.
//!
//! # Why the font is written here rather than loaded
//!
//! Two reasons, and the second is the one that decided it.
//!
//! A font file is a **binary asset with its own licence**, and this tree's standing constraint is that
//! nothing in it derives from anything else — see `LICENSING.md`, which exists because a derivation was
//! removed from this project file by file. Committing a typeface would put a second set of obligations
//! into a repository whose whole point is that it has one, and shipping a system font instead makes the
//! rendered result depend on which machine drew it, which is exactly what a capture-based regression
//! harness cannot tolerate.
//!
//! And an outline font *format* is a large amount of code to read a large amount of data, when what a
//! shell needs is legible Latin text at three sizes. What is here instead is a **stroked** typeface: each
//! glyph is a handful of lines and elliptical arcs on a shared grid, and the rasteriser gives them width
//! by measuring the distance from each pixel to the nearest stroke. That is a few hundred lines including
//! the letterforms, it scales to any size because it is geometry rather than a bitmap, and it reads as
//! drafting lettering — which is not an accident anybody needs to apologise for in a military interface.
//!
//! # What it cannot do, stated rather than discovered
//!
//! There are no Chinese, Japanese or Korean glyphs, and there is no way to author tens of thousands of
//! them by hand. A character with no glyph draws as a hollow box, which is the convention and which is
//! *visible* — the failure announces itself rather than silently dropping the character. The
//! input-method work in `cic-ui` is unaffected and is not wasted: the composition model is what would be
//! expensive to retrofit, and a loaded-font path can be added behind [`Font`] without touching it.
//!
//! # The grid
//!
//! One coordinate system for every glyph, in integers so the letterforms are readable as data:
//!
//! ```text
//!  y = 0   top of the ascender
//!  y = 2   top of a capital
//!  y = 10  top of a lowercase x
//!  y = 22  the baseline
//!  y = 28  bottom of the descender
//! ```
//!
//! So `UNITS_PER_LINE` is 28 and a size in pixels maps to it directly: at 16 pixels a capital is 11.4
//! pixels tall, which is the ratio a text face normally has. X grows rightward from the pen position and
//! each glyph states its own advance, so the face is proportional rather than monospaced.
//!
//! # Why the rasteriser measures distance
//!
//! Filling an outline means finding spans between edge crossings and antialiasing them by area, which
//! needs a scanline pass and careful handling of shared edges. A *stroke* has no inside, so the same
//! result comes from one question asked per pixel: how far is this pixel's centre from the nearest
//! segment? Coverage falls from one to zero across the last pixel of the stroke's half-width, which is
//! antialiasing that costs a `clamp`.
//!
//! The pixel's **centre** is the point that matters — `(x + 0.5, y + 0.5)`, not `(x, y)`. Measuring from
//! the corner shifts every glyph half a pixel up and left, which is invisible in a test that compares a
//! rasteriser against itself and obvious the moment two sizes are drawn side by side.

use std::collections::BTreeMap;

/// Grid units from the top of the ascender to the bottom of the descender.
pub const UNITS_PER_LINE: f32 = 28.0;

/// Grid Y of the baseline.
pub const BASELINE: f32 = 22.0;

/// Widest atlas row before it wraps, in pixels.
///
/// 512 keeps every atlas inside one texture on any device this targets, and a row of glyphs at a UI size
/// is nowhere near it, so the packing stays a single wrap rather than a bin-packing problem.
const ATLAS_WIDTH: u32 = 512;

/// One stroke of a glyph.
///
/// Two kinds, because letterforms are two kinds of thing. Straight runs are polylines so a stem, a
/// crossbar and a diagonal are one entry; round parts are elliptical arcs so an `O` is one number-quintet
/// rather than twenty points, and so its smoothness follows the size it is drawn at.
#[derive(Debug, Clone, Copy)]
pub enum Stroke {
    /// A connected run of straight segments.
    Line(&'static [(i8, i8)]),
    /// A part of an ellipse.
    ///
    /// Angles are degrees from the positive X axis increasing *toward positive Y*, which is downward on
    /// screen — so 90 is the bottom of the ellipse. A negative sweep runs the other way. Stating it here
    /// rather than assuming the mathematical convention matters: this grid is Y-down like every other
    /// screen-space type in the engine, and an arc authored against the other convention comes out
    /// mirrored.
    Arc {
        /// Centre X.
        cx: i8,
        /// Centre Y.
        cy: i8,
        /// Half-extent along X.
        rx: i8,
        /// Half-extent along Y.
        ry: i8,
        /// Where the arc begins, in degrees.
        start: i16,
        /// How far it turns, in degrees. Negative runs anticlockwise on screen.
        sweep: i16,
    },
}

const fn line(points: &'static [(i8, i8)]) -> Stroke {
    Stroke::Line(points)
}

const fn arc(cx: i8, cy: i8, rx: i8, ry: i8, start: i16, sweep: i16) -> Stroke {
    Stroke::Arc {
        cx,
        cy,
        rx,
        ry,
        start,
        sweep,
    }
}

/// One glyph: how far the pen moves, and what to draw.
#[derive(Debug, Clone, Copy)]
pub struct Glyph {
    /// How far the pen advances, in grid units.
    pub advance: i8,
    /// The strokes to draw, in grid units.
    pub strokes: &'static [Stroke],
}

/// What a character with no glyph draws as: a hollow box.
///
/// The typographic convention, and the right answer for the reason a missing string renders as its own
/// key. A dropped character is indistinguishable from text that was never typed; a box says the font is
/// the thing that is missing.
const TOFU: Glyph = Glyph {
    advance: 14,
    strokes: &[line(&[(1, 4), (11, 4), (11, 22), (1, 22), (1, 4)])],
};

/// Every glyph, keyed by character.
///
/// Ordered by character so the table is bisectable and so a reader can find one. Capitals occupy `y` 2 to
/// 22, lowercase 10 to 22 with descenders to 28, and each entry's advance is its own.
#[rustfmt::skip]
const GLYPHS: &[(char, Glyph)] = &[
    (' ',  Glyph { advance: 8,  strokes: &[] }),
    ('!',  Glyph { advance: 7,  strokes: &[line(&[(2, 2), (2, 17)]), line(&[(2, 21), (2, 22)])] }),
    ('"',  Glyph { advance: 10, strokes: &[line(&[(1, 2), (1, 7)]), line(&[(5, 2), (5, 7)])] }),
    // A crossbar pair and a stem pair, the stems leaning so it does not read as a grid.
    ('#',  Glyph { advance: 16, strokes: &[
        line(&[(1, 10), (13, 10)]), line(&[(0, 17), (12, 17)]),
        line(&[(5, 4), (3, 22)]), line(&[(11, 4), (9, 22)])] }),
    // An S with a bar through it, which is the same construction the letter uses.
    ('$',  Glyph { advance: 16, strokes: &[
        arc(6, 7, 5, 5, 320, -230), arc(6, 17, 5, 5, 270, 230), line(&[(6, 0), (6, 24)])] }),
    ('%',  Glyph { advance: 18, strokes: &[
        arc(3, 6, 3, 4, 0, 360), arc(13, 18, 3, 4, 0, 360), line(&[(15, 2), (1, 22)])] }),
    // The geometric form rather than the Latin ligature: a small ring over a larger bowl that opens at
    // the lower right, with the tail running out of it. The ligature's crossing backbone needs four
    // strokes to meet at one point and reads as a smudge at a UI size.
    ('&',  Glyph { advance: 18, strokes: &[
        arc(7, 7, 4, 5, 0, 360), arc(6, 17, 5, 5, 100, 260), line(&[(11, 17), (15, 22)])] }),
    ('\'', Glyph { advance: 6,  strokes: &[line(&[(2, 2), (2, 7)])] }),
    ('(',  Glyph { advance: 8,  strokes: &[arc(10, 14, 8, 13, 135, 90)] }),
    (')',  Glyph { advance: 8,  strokes: &[arc(-2, 14, 8, 13, 315, 90)] }),
    ('*',  Glyph { advance: 10, strokes: &[
        line(&[(4, 6), (4, 14)]), line(&[(1, 8), (7, 12)]), line(&[(7, 8), (1, 12)])] }),
    ('+',  Glyph { advance: 14, strokes: &[line(&[(1, 14), (11, 14)]), line(&[(6, 9), (6, 19)])] }),
    (',',  Glyph { advance: 6,  strokes: &[line(&[(3, 21), (1, 25)])] }),
    ('-',  Glyph { advance: 12, strokes: &[line(&[(1, 16), (9, 16)])] }),
    ('.',  Glyph { advance: 6,  strokes: &[line(&[(2, 21), (2, 22)])] }),
    ('/',  Glyph { advance: 10, strokes: &[line(&[(0, 24), (8, 0)])] }),
    // A slashed zero, because 0 and O are the same shape and a settings screen is full of numbers.
    ('0',  Glyph { advance: 16, strokes: &[arc(6, 12, 6, 10, 0, 360), line(&[(3, 18), (9, 6)])] }),
    ('1',  Glyph { advance: 16, strokes: &[
        line(&[(2, 6), (6, 2), (6, 22)]), line(&[(2, 22), (10, 22)])] }),
    ('2',  Glyph { advance: 16, strokes: &[
        arc(6, 8, 6, 6, 180, 200), line(&[(11, 10), (0, 22), (12, 22)])] }),
    ('3',  Glyph { advance: 16, strokes: &[arc(6, 7, 5, 5, 180, 270), arc(6, 17, 5, 5, 270, 270)] }),
    ('4',  Glyph { advance: 16, strokes: &[
        line(&[(9, 2), (0, 16), (12, 16)]), line(&[(9, 2), (9, 22)])] }),
    ('5',  Glyph { advance: 16, strokes: &[
        line(&[(11, 2), (1, 2), (1, 10), (6, 10)]), arc(6, 16, 6, 6, 270, 230)] }),
    ('6',  Glyph { advance: 16, strokes: &[
        arc(6, 16, 6, 6, 0, 360), line(&[(0, 16), (0, 7), (5, 2)])] }),
    ('7',  Glyph { advance: 16, strokes: &[line(&[(0, 2), (12, 2), (4, 22)])] }),
    ('8',  Glyph { advance: 16, strokes: &[arc(6, 7, 5, 5, 0, 360), arc(6, 17, 5, 5, 0, 360)] }),
    ('9',  Glyph { advance: 16, strokes: &[
        arc(6, 8, 6, 6, 0, 360), line(&[(12, 8), (12, 17), (7, 22)])] }),
    (':',  Glyph { advance: 6,  strokes: &[line(&[(2, 13), (2, 14)]), line(&[(2, 21), (2, 22)])] }),
    (';',  Glyph { advance: 6,  strokes: &[line(&[(2, 13), (2, 14)]), line(&[(3, 21), (1, 25)])] }),
    ('<',  Glyph { advance: 12, strokes: &[line(&[(9, 10), (2, 16), (9, 22)])] }),
    ('=',  Glyph { advance: 14, strokes: &[line(&[(1, 12), (11, 12)]), line(&[(1, 17), (11, 17)])] }),
    ('>',  Glyph { advance: 12, strokes: &[line(&[(2, 10), (9, 16), (2, 22)])] }),
    ('?',  Glyph { advance: 14, strokes: &[
        arc(6, 7, 5, 5, 180, 200), line(&[(10, 9), (6, 13), (6, 17)]), line(&[(6, 21), (6, 22)])] }),
    ('@',  Glyph { advance: 18, strokes: &[
        arc(7, 13, 7, 9, 20, 320), arc(7, 13, 3, 4, 0, 360), line(&[(10, 10), (10, 17), (14, 17)])] }),
    ('A',  Glyph { advance: 16, strokes: &[
        line(&[(0, 22), (6, 2), (12, 22)]), line(&[(2, 16), (10, 16)])] }),
    ('B',  Glyph { advance: 16, strokes: &[
        line(&[(6, 2), (0, 2), (0, 22), (6, 22)]), line(&[(0, 12), (6, 12)]),
        arc(6, 7, 5, 5, -90, 180), arc(6, 17, 6, 5, -90, 180)] }),
    ('C',  Glyph { advance: 16, strokes: &[arc(6, 12, 6, 10, 50, 260)] }),
    ('D',  Glyph { advance: 16, strokes: &[
        line(&[(5, 2), (0, 2), (0, 22), (5, 22)]), arc(5, 12, 7, 10, -90, 180)] }),
    ('E',  Glyph { advance: 15, strokes: &[
        line(&[(12, 2), (0, 2), (0, 22), (12, 22)]), line(&[(0, 12), (9, 12)])] }),
    ('F',  Glyph { advance: 14, strokes: &[
        line(&[(12, 2), (0, 2), (0, 22)]), line(&[(0, 12), (9, 12)])] }),
    ('G',  Glyph { advance: 17, strokes: &[
        arc(6, 12, 6, 10, 0, 320), line(&[(12, 12), (6, 12)])] }),
    ('H',  Glyph { advance: 16, strokes: &[
        line(&[(0, 2), (0, 22)]), line(&[(12, 2), (12, 22)]), line(&[(0, 12), (12, 12)])] }),
    ('I',  Glyph { advance: 8,  strokes: &[line(&[(4, 2), (4, 22)])] }),
    ('J',  Glyph { advance: 14, strokes: &[line(&[(9, 2), (9, 17)]), arc(5, 17, 4, 5, 0, 180)] }),
    ('K',  Glyph { advance: 16, strokes: &[
        line(&[(0, 2), (0, 22)]), line(&[(11, 2), (1, 13)]), line(&[(3, 11), (12, 22)])] }),
    ('L',  Glyph { advance: 14, strokes: &[line(&[(0, 2), (0, 22), (11, 22)])] }),
    ('M',  Glyph { advance: 20, strokes: &[line(&[(0, 22), (0, 2), (8, 14), (16, 2), (16, 22)])] }),
    ('N',  Glyph { advance: 16, strokes: &[line(&[(0, 22), (0, 2), (12, 22), (12, 2)])] }),
    ('O',  Glyph { advance: 17, strokes: &[arc(6, 12, 6, 10, 0, 360)] }),
    ('P',  Glyph { advance: 15, strokes: &[
        line(&[(0, 22), (0, 2), (6, 2)]), arc(6, 8, 5, 6, -90, 180), line(&[(6, 14), (0, 14)])] }),
    ('Q',  Glyph { advance: 17, strokes: &[arc(6, 12, 6, 10, 0, 360), line(&[(7, 17), (13, 24)])] }),
    ('R',  Glyph { advance: 16, strokes: &[
        line(&[(0, 22), (0, 2), (6, 2)]), arc(6, 8, 5, 6, -90, 180),
        line(&[(6, 14), (0, 14)]), line(&[(5, 14), (12, 22)])] }),
    ('S',  Glyph { advance: 16, strokes: &[arc(6, 7, 6, 5, 320, -230), arc(6, 17, 6, 5, 270, 230)] }),
    ('T',  Glyph { advance: 15, strokes: &[line(&[(0, 2), (12, 2)]), line(&[(6, 2), (6, 22)])] }),
    ('U',  Glyph { advance: 16, strokes: &[
        line(&[(0, 2), (0, 15)]), line(&[(12, 2), (12, 15)]), arc(6, 15, 6, 7, 180, -180)] }),
    ('V',  Glyph { advance: 16, strokes: &[line(&[(0, 2), (6, 22), (12, 2)])] }),
    ('W',  Glyph { advance: 22, strokes: &[line(&[(0, 2), (4, 22), (9, 9), (14, 22), (18, 2)])] }),
    ('X',  Glyph { advance: 16, strokes: &[line(&[(0, 2), (12, 22)]), line(&[(12, 2), (0, 22)])] }),
    ('Y',  Glyph { advance: 16, strokes: &[
        line(&[(0, 2), (6, 12)]), line(&[(12, 2), (6, 12)]), line(&[(6, 12), (6, 22)])] }),
    ('Z',  Glyph { advance: 15, strokes: &[line(&[(0, 2), (12, 2), (0, 22), (12, 22)])] }),
    ('[',  Glyph { advance: 9,  strokes: &[line(&[(6, 2), (2, 2), (2, 24), (6, 24)])] }),
    ('\\', Glyph { advance: 10, strokes: &[line(&[(0, 0), (8, 24)])] }),
    (']',  Glyph { advance: 9,  strokes: &[line(&[(2, 2), (6, 2), (6, 24), (2, 24)])] }),
    ('^',  Glyph { advance: 10, strokes: &[line(&[(1, 8), (5, 3), (9, 8)])] }),
    ('_',  Glyph { advance: 16, strokes: &[line(&[(0, 25), (14, 25)])] }),
    ('`',  Glyph { advance: 6,  strokes: &[line(&[(1, 2), (4, 6)])] }),
    ('a',  Glyph { advance: 14, strokes: &[arc(5, 16, 5, 6, 0, 360), line(&[(10, 10), (10, 22)])] }),
    ('b',  Glyph { advance: 15, strokes: &[line(&[(0, 2), (0, 22)]), arc(6, 16, 6, 6, 0, 360)] }),
    ('c',  Glyph { advance: 13, strokes: &[arc(6, 16, 6, 6, 45, 270)] }),
    ('d',  Glyph { advance: 15, strokes: &[line(&[(12, 2), (12, 22)]), arc(6, 16, 6, 6, 0, 360)] }),
    // Swept anticlockwise from the crossbar's right end, so the aperture lands at the lower right where
    // an `e` has one. Clockwise puts it at the top and the letter reads as a barred `o`.
    ('e',  Glyph { advance: 14, strokes: &[arc(6, 16, 6, 6, 0, -315), line(&[(0, 16), (12, 16)])] }),
    ('f',  Glyph { advance: 9,  strokes: &[
        line(&[(3, 22), (3, 5)]), arc(6, 5, 3, 3, 180, 90), line(&[(0, 10), (7, 10)])] }),
    ('g',  Glyph { advance: 15, strokes: &[
        arc(6, 16, 6, 6, 0, 360), line(&[(12, 10), (12, 25)]), arc(7, 25, 5, 3, 0, 140)] }),
    ('h',  Glyph { advance: 15, strokes: &[
        line(&[(0, 2), (0, 22)]), arc(6, 16, 6, 6, 180, 180), line(&[(12, 16), (12, 22)])] }),
    ('i',  Glyph { advance: 7,  strokes: &[line(&[(2, 10), (2, 22)]), line(&[(2, 5), (2, 6)])] }),
    ('j',  Glyph { advance: 9,  strokes: &[
        line(&[(5, 10), (5, 25)]), arc(2, 25, 3, 3, 0, 90), line(&[(5, 5), (5, 6)])] }),
    ('k',  Glyph { advance: 13, strokes: &[
        line(&[(0, 2), (0, 22)]), line(&[(9, 10), (1, 17)]), line(&[(3, 15), (10, 22)])] }),
    ('l',  Glyph { advance: 7,  strokes: &[line(&[(2, 2), (2, 22)])] }),
    ('m',  Glyph { advance: 22, strokes: &[
        line(&[(0, 10), (0, 22)]), arc(5, 15, 5, 5, 180, 180), line(&[(10, 16), (10, 22)]),
        arc(15, 15, 5, 5, 180, 180), line(&[(20, 16), (20, 22)])] }),
    ('n',  Glyph { advance: 15, strokes: &[
        line(&[(0, 10), (0, 22)]), arc(6, 16, 6, 6, 180, 180), line(&[(12, 16), (12, 22)])] }),
    ('o',  Glyph { advance: 15, strokes: &[arc(6, 16, 6, 6, 0, 360)] }),
    ('p',  Glyph { advance: 15, strokes: &[line(&[(0, 10), (0, 28)]), arc(6, 16, 6, 6, 0, 360)] }),
    ('q',  Glyph { advance: 15, strokes: &[line(&[(12, 10), (12, 28)]), arc(6, 16, 6, 6, 0, 360)] }),
    ('r',  Glyph { advance: 10, strokes: &[line(&[(0, 10), (0, 22)]), arc(5, 15, 5, 5, 180, 110)] }),
    ('s',  Glyph { advance: 13, strokes: &[arc(5, 13, 5, 3, 320, -230), arc(5, 19, 5, 3, 270, 230)] }),
    ('t',  Glyph { advance: 10, strokes: &[
        line(&[(3, 4), (3, 19)]), arc(6, 19, 3, 3, 180, 90), line(&[(0, 10), (7, 10)])] }),
    ('u',  Glyph { advance: 15, strokes: &[
        line(&[(0, 10), (0, 16)]), arc(6, 16, 6, 6, 180, -180), line(&[(12, 10), (12, 22)])] }),
    ('v',  Glyph { advance: 14, strokes: &[line(&[(0, 10), (6, 22), (12, 10)])] }),
    ('w',  Glyph { advance: 20, strokes: &[line(&[(0, 10), (4, 22), (8, 13), (12, 22), (16, 10)])] }),
    ('x',  Glyph { advance: 13, strokes: &[line(&[(0, 10), (11, 22)]), line(&[(11, 10), (0, 22)])] }),
    ('y',  Glyph { advance: 14, strokes: &[line(&[(0, 10), (6, 22)]), line(&[(12, 10), (4, 28)])] }),
    ('z',  Glyph { advance: 13, strokes: &[line(&[(0, 10), (11, 10), (0, 22), (11, 22)])] }),
    ('{',  Glyph { advance: 9,  strokes: &[
        line(&[(7, 2), (4, 2), (4, 12), (1, 13), (4, 14), (4, 24), (7, 24)])] }),
    ('|',  Glyph { advance: 6,  strokes: &[line(&[(2, 0), (2, 26)])] }),
    ('}',  Glyph { advance: 9,  strokes: &[
        line(&[(1, 2), (4, 2), (4, 12), (7, 13), (4, 14), (4, 24), (1, 24)])] }),
    ('~',  Glyph { advance: 12, strokes: &[line(&[(0, 16), (3, 13), (7, 19), (10, 16)])] }),
];

/// The interface typeface.
///
/// A unit struct rather than loaded state, because the glyphs are compiled in. It is a type rather than
/// free functions so a loaded-font path can be added later without every caller changing: everything that
/// draws text asks a `Font` for a glyph and an advance, and nothing asks the table.
#[derive(Debug, Clone, Copy, Default)]
pub struct Font;

impl Font {
    /// The built-in typeface.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// One character's glyph, or the hollow box that stands in for anything absent.
    #[must_use]
    pub fn glyph(&self, character: char) -> Glyph {
        GLYPHS
            .iter()
            .find_map(|(candidate, glyph)| (*candidate == character).then_some(*glyph))
            .unwrap_or(TOFU)
    }

    /// Whether a character has a glyph of its own.
    #[must_use]
    pub fn has_glyph(&self, character: char) -> bool {
        GLYPHS.iter().any(|(candidate, _)| *candidate == character)
    }

    /// Every character with a glyph, in order.
    pub fn characters(&self) -> impl Iterator<Item = char> + '_ {
        GLYPHS.iter().map(|(character, _)| *character)
    }

    /// How far the pen moves over a string, at a size in pixels.
    ///
    /// Summed in grid units and scaled once, so the answer is exactly the sum of the positions the
    /// shaper will place glyphs at. Scaling each advance and then summing would drift by a fraction of a
    /// pixel per character, and the caret would sit slightly wrong on a long string.
    #[must_use]
    pub fn advance(&self, text: &str, size: f32) -> f32 {
        let units: i32 = text
            .chars()
            .map(|character| i32::from(self.glyph(character).advance))
            .sum();
        #[allow(clippy::cast_precision_loss)]
        {
            units as f32 * size / UNITS_PER_LINE
        }
    }

    /// How tall a line is, at a size in pixels.
    ///
    /// The size itself, since the grid is defined as ascender top to descender bottom.
    #[must_use]
    pub fn line_height(&self, size: f32) -> f32 {
        size
    }

    /// How far the baseline sits below the top of a line, at a size in pixels.
    #[must_use]
    pub fn ascent(&self, size: f32) -> f32 {
        size * BASELINE / UNITS_PER_LINE
    }

    /// How thick a stroke is, at a size in pixels.
    ///
    /// Proportional to the size so weight is consistent, with a floor of one pixel: below that a stroke
    /// is a partial-coverage smear and the text reads as grey rather than as letters.
    #[must_use]
    pub fn stroke_width(&self, size: f32) -> f32 {
        (size / 12.0).max(1.0)
    }

    /// Rasterises one character at a size in pixels.
    ///
    /// Returns nothing when the glyph has no strokes — a space — because an empty bitmap is not something
    /// to pack or draw.
    #[must_use]
    pub fn rasterise(&self, character: char, size: f32) -> Option<Coverage> {
        rasterise(self.glyph(character), size, self.stroke_width(size))
    }
}

/// One glyph's antialiased coverage, and where it sits relative to the pen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coverage {
    /// Bitmap width in pixels.
    pub width: u32,
    /// Bitmap height in pixels.
    pub height: u32,
    /// Pixels from the pen position to the bitmap's left edge. Negative when a glyph leans left of it.
    pub left: i32,
    /// Pixels from the top of the line to the bitmap's top edge.
    pub top: i32,
    /// One byte of coverage per pixel, row-major.
    pub data: Vec<u8>,
}

/// A straight segment in pixel space.
#[derive(Debug, Clone, Copy)]
struct Segment {
    from: [f32; 2],
    to: [f32; 2],
}

impl Segment {
    /// The distance from a point to this segment.
    fn distance(&self, point: [f32; 2]) -> f32 {
        let along = [self.to[0] - self.from[0], self.to[1] - self.from[1]];
        let offset = [point[0] - self.from[0], point[1] - self.from[1]];
        let length_squared = along[0] * along[0] + along[1] * along[1];
        // A degenerate segment is a dot, which is what a full stop and a dieresis are made of, so this
        // has to be a supported case rather than a guard against division.
        let t = if length_squared <= f32::EPSILON {
            0.0
        } else {
            ((offset[0] * along[0] + offset[1] * along[1]) / length_squared).clamp(0.0, 1.0)
        };
        let nearest = [self.from[0] + along[0] * t, self.from[1] + along[1] * t];
        let delta = [point[0] - nearest[0], point[1] - nearest[1]];
        (delta[0] * delta[0] + delta[1] * delta[1]).sqrt()
    }
}

/// Turns a glyph's strokes into pixel-space segments, flattening arcs.
fn flatten(glyph: Glyph, scale: f32) -> Vec<Segment> {
    let mut segments = Vec::new();
    for stroke in glyph.strokes {
        match stroke {
            Stroke::Line(points) => {
                for pair in points.windows(2) {
                    segments.push(Segment {
                        from: [f32::from(pair[0].0) * scale, f32::from(pair[0].1) * scale],
                        to: [f32::from(pair[1].0) * scale, f32::from(pair[1].1) * scale],
                    });
                }
            }
            Stroke::Arc {
                cx,
                cy,
                rx,
                ry,
                start,
                sweep,
            } => {
                let centre = [f32::from(*cx) * scale, f32::from(*cy) * scale];
                let radius = [f32::from(*rx) * scale, f32::from(*ry) * scale];
                let start = f32::from(*start).to_radians();
                let sweep = f32::from(*sweep).to_radians();
                // Segments about a pixel and a half long, so the flattening error stays under the
                // antialiasing it is drawn with. Following the size means a title is no more faceted
                // than a caption, which a fixed step count would not manage.
                let arc_length = sweep.abs() * radius[0].max(radius[1]);
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let steps = ((arc_length / 1.5).ceil() as u16).clamp(3, 256);
                let mut previous = arc_point(centre, radius, start);
                for step in 1..=steps {
                    let angle = start + sweep * (f32::from(step) / f32::from(steps));
                    let point = arc_point(centre, radius, angle);
                    segments.push(Segment {
                        from: previous,
                        to: point,
                    });
                    previous = point;
                }
            }
        }
    }
    segments
}

fn arc_point(centre: [f32; 2], radius: [f32; 2], angle: f32) -> [f32; 2] {
    [
        centre[0] + radius[0] * angle.cos(),
        centre[1] + radius[1] * angle.sin(),
    ]
}

/// Rasterises a glyph's strokes into coverage.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap
)]
fn rasterise(glyph: Glyph, size: f32, stroke_width: f32) -> Option<Coverage> {
    if !size.is_finite() || size <= 0.0 {
        return None;
    }
    let segments = flatten(glyph, size / UNITS_PER_LINE);
    if segments.is_empty() {
        return None;
    }

    let half = stroke_width / 2.0;
    let mut min = [f32::MAX, f32::MAX];
    let mut max = [f32::MIN, f32::MIN];
    for segment in &segments {
        for point in [segment.from, segment.to] {
            for axis in 0..2 {
                min[axis] = min[axis].min(point[axis]);
                max[axis] = max[axis].max(point[axis]);
            }
        }
    }
    // One pixel of margin past the stroke's half-width, which is where coverage reaches zero. Without
    // it the outermost row of antialiasing is cut off and every glyph has one hard edge.
    let left = (min[0] - half - 1.0).floor();
    let top = (min[1] - half - 1.0).floor();
    let width = ((max[0] + half + 1.0).ceil() - left).max(1.0) as u32;
    let height = ((max[1] + half + 1.0).ceil() - top).max(1.0) as u32;

    let mut data = vec![0u8; (width as usize) * (height as usize)];
    for row in 0..height {
        for column in 0..width {
            // The pixel's centre, not its corner. Measuring from the corner shifts every glyph half a
            // pixel up and left, which a rasteriser compared against itself cannot show.
            let point = [left + column as f32 + 0.5, top + row as f32 + 0.5];
            let distance = segments
                .iter()
                .map(|segment| segment.distance(point))
                .fold(f32::MAX, f32::min);
            // Coverage falls from one to zero across the last pixel of the half-width, which is the
            // antialiasing. A stroke has no inside, so no winding rule is needed.
            let coverage = (half + 0.5 - distance).clamp(0.0, 1.0);
            data[(row as usize) * (width as usize) + column as usize] = (coverage * 255.0) as u8;
        }
    }

    Some(Coverage {
        width,
        height,
        left: left as i32,
        top: top as i32,
        data,
    })
}

/// Where one glyph sits in an atlas, and how to place it when drawing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    /// Texture coordinates of the glyph's rectangle: left, top, right, bottom.
    pub uv: [f32; 4],
    /// The glyph's size in pixels.
    pub size: [f32; 2],
    /// Where the glyph's top-left corner goes, relative to the pen and the top of the line.
    pub offset: [f32; 2],
    /// How far the pen advances afterwards, in pixels.
    pub advance: f32,
}

/// Every glyph the interface needs, rasterised once and packed into one image.
///
/// # Why sizes are declared rather than discovered
///
/// A lazily-grown atlas has to reallocate and re-upload its texture the first time a new size appears,
/// which puts a device operation in the middle of building a draw list. Declaring the sizes up front —
/// a theme has three, multiplied by the display scale — means the texture is written once and the
/// drawing path only ever reads. Rebuilding on a scale change is then an explicit step a host takes,
/// not a hidden one it discovers.
///
/// Sizes are rounded to whole pixels, because that is the resolution a rasteriser has anyway. Two
/// requests half a pixel apart would otherwise pack two indistinguishable copies of the alphabet.
#[derive(Debug, Clone, PartialEq)]
pub struct GlyphAtlas {
    width: u32,
    height: u32,
    data: Vec<u8>,
    placed: BTreeMap<(u32, char), Placed>,
}

impl GlyphAtlas {
    /// Rasterises every glyph at every requested size and packs them.
    ///
    /// Sizes that round to the same pixel size are packed once. An empty request yields a one-pixel
    /// atlas rather than a zero-sized one, because a texture of no extent cannot be created.
    #[must_use]
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    pub fn new(font: &Font, sizes: &[f32]) -> Self {
        let mut wanted: Vec<u32> = sizes
            .iter()
            .filter(|size| size.is_finite() && **size >= 1.0)
            .map(|size| size.round() as u32)
            .collect();
        wanted.sort_unstable();
        wanted.dedup();

        // Rasterise first, pack second. The packing needs every glyph's height to know how tall a shelf
        // is, and rasterising twice to find out would double the cost of building an atlas.
        let mut rasterised: Vec<(u32, char, Coverage)> = Vec::new();
        for size in &wanted {
            for character in font.characters() {
                if let Some(coverage) = font.rasterise(character, *size as f32) {
                    rasterised.push((*size, character, coverage));
                }
            }
        }

        let mut placed = BTreeMap::new();
        let mut pen = [1u32, 1u32];
        let mut shelf = 0u32;
        let mut extent = [1u32, 1u32];
        let mut cells: Vec<([u32; 2], u32, char, Coverage)> = Vec::new();
        for (size, character, coverage) in rasterised {
            // One pixel of padding around every glyph, so a bilinear sample at a glyph's edge cannot
            // reach into its neighbour. Without it, text shows a faint ghost of the next letter.
            if pen[0] + coverage.width + 1 > ATLAS_WIDTH {
                pen = [1, pen[1] + shelf + 1];
                shelf = 0;
            }
            shelf = shelf.max(coverage.height);
            let at = pen;
            pen[0] += coverage.width + 1;
            extent[0] = extent[0].max(at[0] + coverage.width + 1);
            extent[1] = extent[1].max(at[1] + coverage.height + 1);
            cells.push((at, size, character, coverage));
        }

        let (width, height) = (extent[0].max(1), extent[1].max(1));
        let mut data = vec![0u8; (width as usize) * (height as usize)];
        for (at, size, character, coverage) in cells {
            for row in 0..coverage.height {
                let source = (row as usize) * (coverage.width as usize);
                let target = ((at[1] + row) as usize) * (width as usize) + at[0] as usize;
                data[target..target + coverage.width as usize]
                    .copy_from_slice(&coverage.data[source..source + coverage.width as usize]);
            }
            let advance = f32::from(font.glyph(character).advance) * size as f32 / UNITS_PER_LINE;
            placed.insert(
                (size, character),
                Placed {
                    uv: [
                        at[0] as f32 / width as f32,
                        at[1] as f32 / height as f32,
                        (at[0] + coverage.width) as f32 / width as f32,
                        (at[1] + coverage.height) as f32 / height as f32,
                    ],
                    size: [coverage.width as f32, coverage.height as f32],
                    offset: [coverage.left as f32, coverage.top as f32],
                    advance,
                },
            );
        }

        Self {
            width,
            height,
            data,
            placed,
        }
    }

    /// The atlas image's width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// The atlas image's height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// One byte of coverage per pixel, row-major.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// How many glyphs are packed.
    #[must_use]
    pub fn len(&self) -> usize {
        self.placed.len()
    }

    /// Whether nothing is packed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.placed.is_empty()
    }

    /// Which pixel sizes are packed, in order.
    pub fn sizes(&self) -> impl Iterator<Item = u32> + '_ {
        // The map is keyed by size first, so runs of one size are contiguous and deduplicating is a
        // comparison with the previous key rather than a set.
        let mut previous = None;
        self.placed
            .keys()
            .filter(move |(size, _)| {
                let fresh = previous != Some(*size);
                previous = Some(*size);
                fresh
            })
            .map(|(size, _)| *size)
    }

    /// One glyph at a size, or nothing when it was not packed.
    ///
    /// A space is legitimately absent: it has no strokes and so no bitmap, and a shaper advances the pen
    /// past it without drawing anything.
    #[must_use]
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn glyph(&self, size: f32, character: char) -> Option<Placed> {
        if !size.is_finite() || size < 1.0 {
            return None;
        }
        self.placed.get(&(size.round() as u32, character)).copied()
    }

    /// The pixel size this atlas will actually use for a requested one.
    ///
    /// The nearest packed size, so a request the atlas was not built for draws at the closest thing it
    /// has instead of drawing nothing. A caller that wants to know it asked for a size nobody packed
    /// compares this with what it passed.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn nearest_size(&self, size: f32) -> Option<f32> {
        if !size.is_finite() {
            return None;
        }
        self.sizes()
            .min_by(|left, right| {
                let distance = |candidate: u32| (candidate as f32 - size).abs();
                distance(*left).total_cmp(&distance(*right))
            })
            .map(|nearest| nearest as f32)
    }
}

#[cfg(test)]
mod tests {
    // Every cast below is over figures a test states itself -- a specimen's cell size at a size of 40, a
    // glyph's own bitmap extent, a coverage byte -- so none can truncate, wrap, or lose a sign.
    #![allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]

    use super::{BASELINE, Coverage, Font, GLYPHS, GlyphAtlas, Stroke, UNITS_PER_LINE, rasterise};

    /// The darkest pixel in a coverage bitmap, which is one wherever a stroke's centre passed.
    fn peak(coverage: &Coverage) -> u8 {
        coverage.data.iter().copied().max().unwrap_or(0)
    }

    /// How much ink a bitmap holds, as a fraction of full coverage over its area.
    fn ink(coverage: &Coverage) -> f32 {
        let total: u32 = coverage.data.iter().copied().map(u32::from).sum();
        total as f32 / (255.0 * coverage.data.len() as f32)
    }

    #[test]
    fn the_glyph_table_is_ordered_and_holds_no_duplicates() {
        // Ordered so a reader can find a glyph and so a future bisection is valid; unique because a
        // second entry for a character would be unreachable and would look like the one being edited.
        let mut previous = None;
        for (character, _) in GLYPHS {
            if let Some(before) = previous {
                assert!(
                    before < *character,
                    "{before:?} then {character:?} is out of order or repeated"
                );
            }
            previous = Some(*character);
        }
    }

    #[test]
    fn every_printable_ascii_character_has_a_glyph_of_its_own() {
        // The set the shell draws from. Anything absent falls back to a hollow box, which is honest but
        // is not something a Latin interface should be showing.
        let font = Font::new();
        for code in 0x20u8..=0x7e {
            let character = char::from(code);
            assert!(font.has_glyph(character), "{character:?} has no glyph");
        }
    }

    #[test]
    fn every_glyph_stays_inside_its_own_advance_and_the_line() {
        // A glyph wider than its advance collides with the next letter, and one outside the line's
        // vertical extent is clipped by the row it is packed into. Both are authoring mistakes in the
        // table, so they are checked as data rather than found in a capture.
        for (character, glyph) in GLYPHS {
            let mut bounds = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
            let mut note = |x: f32, y: f32| {
                bounds[0] = bounds[0].min(x);
                bounds[1] = bounds[1].min(y);
                bounds[2] = bounds[2].max(x);
                bounds[3] = bounds[3].max(y);
            };
            for stroke in glyph.strokes {
                match stroke {
                    Stroke::Line(points) => {
                        for (x, y) in *points {
                            note(f32::from(*x), f32::from(*y));
                        }
                    }
                    Stroke::Arc {
                        cx,
                        cy,
                        rx,
                        ry,
                        start,
                        sweep,
                    } => {
                        // The extremes of an arc are not only its endpoints, so this walks it.
                        for step in 0u8..=64 {
                            let angle = (f32::from(*start)
                                + f32::from(*sweep) * f32::from(step) / 64.0)
                                .to_radians();
                            note(
                                f32::from(*cx) + f32::from(*rx) * angle.cos(),
                                f32::from(*cy) + f32::from(*ry) * angle.sin(),
                            );
                        }
                    }
                }
            }
            if glyph.strokes.is_empty() {
                continue;
            }
            let advance = f32::from(glyph.advance);
            assert!(
                bounds[2] <= advance + 0.5,
                "{character:?} reaches x {} past its advance of {advance}",
                bounds[2]
            );
            assert!(bounds[0] >= -2.5, "{character:?} starts at x {}", bounds[0]);
            assert!(bounds[1] >= -0.5, "{character:?} rises to y {}", bounds[1]);
            assert!(
                bounds[3] <= UNITS_PER_LINE + 0.5,
                "{character:?} descends to y {}",
                bounds[3]
            );
        }
    }

    #[test]
    fn a_rasterised_glyph_is_antialiased_rather_than_a_hard_mask() {
        // The whole point of measuring distance. A hard mask has only 0 and 255 in it, and it is what a
        // rasteriser that forgot to blend the last pixel of the half-width produces.
        let font = Font::new();
        let coverage = font.rasterise('O', 24.0).expect("O has strokes");
        assert_eq!(peak(&coverage), 255, "a stroke's centre must be solid");
        assert!(
            coverage.data.iter().any(|value| *value > 0 && *value < 255),
            "no partial coverage: the edges are not antialiased"
        );
        // And there is background: an O is mostly hole.
        assert!(coverage.data.contains(&0));
    }

    #[test]
    fn coverage_is_measured_from_the_pixel_centre() {
        // A single vertical stroke exactly on a pixel boundary covers the two pixels either side of it
        // equally. Measured from the pixel *corner* instead, it would land solidly on one of them --
        // which is the half-pixel error a rasteriser compared against itself cannot show.
        const STEM: &[Stroke] = &[super::line(&[(4, 4), (4, 20)])];
        let glyph = super::Glyph {
            advance: 8,
            strokes: STEM,
        };
        // A stroke one pixel wide, at a scale that puts x = 4 on a whole pixel.
        let coverage = rasterise(glyph, UNITS_PER_LINE, 1.0).expect("strokes");
        let row = coverage.height / 2;
        let at = |column: u32| coverage.data[(row * coverage.width + column) as usize];
        let centre = (4 - coverage.left) as u32;
        assert_eq!(
            at(centre - 1),
            at(centre),
            "the two pixels either side of the boundary must match"
        );
        assert!(at(centre) > 100, "and both must hold real coverage");
    }

    #[test]
    fn a_glyph_scales_with_its_size_rather_than_being_a_bitmap() {
        // The charter's resolution independence, at the level it actually has to hold: doubling the size
        // doubles the extent, and the ink stays a similar fraction of the area rather than the strokes
        // thinning away or fattening into a blob.
        let font = Font::new();
        let small = font.rasterise('A', 16.0).expect("strokes");
        let large = font.rasterise('A', 32.0).expect("strokes");
        let ratio = f32::from(u16::try_from(large.height).expect("small"))
            / f32::from(u16::try_from(small.height).expect("small"));
        assert!(
            (ratio - 2.0).abs() < 0.25,
            "doubling the size scaled the height by {ratio}"
        );
        let (light, heavy) = (ink(&small), ink(&large));
        assert!(
            (light - heavy).abs() < 0.12,
            "stroke weight drifted from {light} to {heavy} across sizes"
        );
    }

    #[test]
    fn a_space_rasterises_to_nothing_but_still_advances() {
        // A space has no strokes, so it has no bitmap to pack, and a shaper moves the pen past it.
        let font = Font::new();
        assert!(font.rasterise(' ', 16.0).is_none());
        assert!(font.advance(" ", 28.0) > 0.0);
    }

    #[test]
    fn an_absent_character_draws_a_box_rather_than_nothing() {
        // The convention, and the right answer for the reason a missing string renders as its own key: a
        // dropped character is indistinguishable from text nobody typed.
        let font = Font::new();
        assert!(!font.has_glyph('中'));
        let coverage = font.rasterise('中', 20.0).expect("a box has strokes");
        assert_eq!(peak(&coverage), 255);
        // Hollow, so it reads as a placeholder rather than as a block.
        assert!(ink(&coverage) < 0.5, "the box is filled in");
    }

    #[test]
    fn an_advance_is_summed_in_grid_units_and_scaled_once() {
        // Scaling each advance and then summing drifts by a fraction of a pixel per character, and the
        // caret ends up slightly wrong on a long string.
        let font = Font::new();
        let size = 17.0;
        let text = "Commanders in Chief";
        let expected: f32 = text
            .chars()
            .map(|character| f32::from(font.glyph(character).advance))
            .sum::<f32>()
            * size
            / UNITS_PER_LINE;
        assert!((font.advance(text, size) - expected).abs() < 1e-3);
        // And it is additive over a split, which is what makes it usable for a caret offset.
        let (head, tail) = text.split_at(9);
        assert!(
            (font.advance(head, size) + font.advance(tail, size) - font.advance(text, size)).abs()
                < 1e-3
        );
    }

    #[test]
    fn metrics_follow_the_grid_the_glyphs_are_authored_on() {
        let font = Font::new();
        assert!((font.line_height(28.0) - 28.0).abs() < f32::EPSILON);
        assert!((font.ascent(28.0) - BASELINE).abs() < f32::EPSILON);
        // Weight is proportional with a one-pixel floor, below which a stroke reads as grey.
        assert!((font.stroke_width(48.0) - 4.0).abs() < f32::EPSILON);
        assert!((font.stroke_width(6.0) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn an_atlas_packs_every_glyph_at_every_requested_size() {
        let font = Font::new();
        let atlas = GlyphAtlas::new(&font, &[13.0, 16.0, 30.0]);
        assert_eq!(atlas.sizes().collect::<Vec<_>>(), vec![13, 16, 30]);
        // Every character but the space, which has no bitmap, at each of the three sizes.
        let drawable = font
            .characters()
            .filter(|character| *character != ' ')
            .count();
        assert_eq!(atlas.len(), drawable * 3);
        assert!(!atlas.is_empty());
        assert_eq!(
            atlas.data().len(),
            (atlas.width() * atlas.height()) as usize
        );
        assert!(atlas.width() <= super::ATLAS_WIDTH);
    }

    #[test]
    fn a_packed_glyph_reports_coordinates_inside_the_atlas() {
        let font = Font::new();
        let atlas = GlyphAtlas::new(&font, &[16.0]);
        let placed = atlas.glyph(16.0, 'g').expect("g is packed");
        for coordinate in placed.uv {
            assert!(
                (0.0..=1.0).contains(&coordinate),
                "{coordinate} is outside the atlas"
            );
        }
        assert!(placed.uv[0] < placed.uv[2] && placed.uv[1] < placed.uv[3]);
        assert!(placed.size[0] > 0.0 && placed.size[1] > 0.0);
        // A descender hangs below the baseline, which is what the offset has to be able to say.
        assert!(placed.offset[1] + placed.size[1] > font.ascent(16.0));
    }

    #[test]
    fn glyphs_are_padded_so_a_sample_cannot_reach_a_neighbour() {
        // Without a gap, a bilinear sample at a glyph's edge picks up the next letter and text shows a
        // faint ghost beside every character.
        let font = Font::new();
        let atlas = GlyphAtlas::new(&font, &[20.0]);
        let mut boxes: Vec<[f32; 4]> = Vec::new();
        for character in font.characters() {
            if let Some(placed) = atlas.glyph(20.0, character) {
                boxes.push(placed.uv);
            }
        }
        let gap = 0.5 / atlas.width() as f32;
        for (index, left) in boxes.iter().enumerate() {
            for right in &boxes[index + 1..] {
                let apart = left[2] + gap <= right[0]
                    || right[2] + gap <= left[0]
                    || left[3] <= right[1]
                    || right[3] <= left[1];
                assert!(apart, "{left:?} and {right:?} touch");
            }
        }
    }

    #[test]
    fn a_size_nobody_packed_falls_back_to_the_nearest_one() {
        // Drawing at a slightly wrong size beats drawing nothing, and a caller that cares can compare
        // what it asked for with what it got.
        let font = Font::new();
        let atlas = GlyphAtlas::new(&font, &[16.0, 32.0]);
        assert_eq!(atlas.nearest_size(15.6), Some(16.0));
        assert_eq!(atlas.nearest_size(40.0), Some(32.0));
        assert_eq!(atlas.nearest_size(f32::NAN), None);
        // A rounded request finds the glyph packed for it.
        assert!(atlas.glyph(15.6, 'A').is_some());
        assert!(atlas.glyph(20.0, 'A').is_none());
    }

    #[test]
    fn a_specimen_of_every_glyph_is_written_for_review() {
        // This project's standing rule is that a green test is not verification for anything drawn --
        // every rendering bug so far passed its own assertions and was caught by opening the image. A
        // letterform is authored data, so the mistakes in it are wrong coordinates and arcs swept the
        // wrong way, and no assertion over coverage bytes will show a `g` with its tail on backwards.
        //
        // So the assertions here cover what *can* be stated -- every glyph puts ink on the page, and none
        // of them is a solid block -- and the specimen beside them is what a person looks at.
        let font = Font::new();
        let size: f32 = 40.0;
        let columns = 12usize;
        let cell = [(size * 1.5) as usize, (size * 1.4) as usize];
        let characters: Vec<char> = font
            .characters()
            .filter(|character| *character != ' ')
            .collect();
        let rows = characters.len().div_ceil(columns);
        let (width, height) = (columns * cell[0], rows * cell[1]);
        let mut image = vec![0u8; width * height];

        for (index, character) in characters.iter().enumerate() {
            let coverage = font
                .rasterise(*character, size)
                .unwrap_or_else(|| panic!("{character:?} produced no ink at all"));
            assert!(
                ink(&coverage) < 0.85,
                "{character:?} is very nearly a solid block, which no letter is"
            );
            let origin = [
                (index % columns) * cell[0] + 4,
                (index / columns) * cell[1] + 2,
            ];
            for row in 0..coverage.height as usize {
                for column in 0..coverage.width as usize {
                    let x = origin[0] + column + coverage.left.max(0) as usize;
                    let y = origin[1] + row + coverage.top.max(0) as usize;
                    if x < width && y < height {
                        let value = coverage.data[row * coverage.width as usize + column];
                        image[y * width + x] = image[y * width + x].max(value);
                    }
                }
            }
        }

        // The same place the capture tests leave their evidence. `CARGO_TARGET_TMPDIR` is set for
        // integration tests only, and this has to be a unit test because it reaches the rasteriser
        // directly, so the directory is derived from the manifest instead.
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join("tmp");
        std::fs::create_dir_all(&directory).expect("create the output directory");
        let path = directory.join("font-specimen.png");
        let file = std::fs::File::create(&path).expect("create the specimen");
        let mut encoder = png::Encoder::new(
            std::io::BufWriter::new(file),
            u32::try_from(width).expect("width fits"),
            u32::try_from(height).expect("height fits"),
        );
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .expect("write the header")
            .write_image_data(&image)
            .expect("write the specimen");
        eprintln!("font specimen written to {}", path.display());
    }

    #[test]
    fn an_empty_request_still_yields_a_creatable_image() {
        // A texture of no extent cannot be created, and a host that themed nothing should get an empty
        // atlas rather than a device error.
        let atlas = GlyphAtlas::new(&Font::new(), &[]);
        assert!(atlas.is_empty());
        assert_eq!(atlas.width(), 1);
        assert_eq!(atlas.height(), 1);
        assert_eq!(atlas.data().len(), 1);
        // A size too small to rasterise is refused rather than packed as a smear.
        assert!(GlyphAtlas::new(&Font::new(), &[0.0, -4.0, f32::NAN]).is_empty());
    }
}
