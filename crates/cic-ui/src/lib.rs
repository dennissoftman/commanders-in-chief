//! The retained interface layer: layout, and the vocabulary a shell is authored in.
//!
//! Deliberately free of any window, GPU, or font dependency, for the reason the camera crate is: the
//! same interface model has to serve the game, a map editor, and any debug tool, and none of them should
//! inherit a graphics stack by depending on a layout solver. The two facts this crate cannot derive —
//! how large a piece of text is, and what the display scale is — arrive through [`solve::Measure`] and
//! [`Viewport`] respectively.
//!
//! # What is here
//!
//! - [`layout`] — the authored tree and its JSON format, bounded and validated.
//! - [`solve`] — the two-pass solver that turns that tree into positioned physical rectangles.
//! - [`geometry`] — rectangles, insets, and the single place logical units become physical pixels.
//! - [`strings`] — display text, behind a key rather than written into a layout.
//! - [`action`] — the closed set of effects a layout may attach to a control.
//!
//! # The three rules this crate exists to enforce
//!
//! **A layout is authored in logical units.** Physical pixels appear only on the way out of [`solve`].
//! A fixed-pixel layout is correct on one monitor, which is the charter's own way of putting it.
//!
//! **Data may not name an action the engine does not define.** [`Action`] is a closed enum, so an
//! unrecognised effect is a load error rather than a control that silently never fires.
//!
//! **Display text lives in a string table, not in a layout file.** Translating is content work for
//! later; making it possible is structural work that cannot be retrofitted cheaply.
//!
//! # Example
//!
//! ```
//! use cic_ui::{Layout, NoContent, Viewport, solve};
//!
//! let layout = Layout::from_json(
//!     br#"{
//!       "format_version": 1,
//!       "root": {
//!         "direction": "row",
//!         "width": { "fill": 1 },
//!         "height": { "fill": 1 },
//!         "children": [
//!           { "width": { "fill": 1 } },
//!           { "width": { "fill": 3 } }
//!         ]
//!       }
//!     }"#,
//! )?;
//!
//! let viewport = Viewport::new(800, 600, 1.0)?;
//! let solved = solve(&layout, viewport, &NoContent);
//!
//! // The row splits 1:3, so the second child is three times the first.
//! assert_eq!(solved.nodes()[1].rect.width, 200.0);
//! assert_eq!(solved.nodes()[2].rect.width, 600.0);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod action;
pub mod geometry;
pub mod layout;
pub mod solve;
pub mod strings;

pub use action::Action;
pub use geometry::{Insets, Rect, Viewport, ViewportError};
pub use layout::{
    Align, Direction, FORMAT_VERSION, Justify, Layout, LayoutError, Node, Sizing, Widget,
};
pub use solve::{Measure, NoContent, Solved, SolvedNode, solve};
pub use strings::StringTable;
