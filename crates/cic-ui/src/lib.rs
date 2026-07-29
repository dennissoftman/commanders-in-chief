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
//! - [`input`] — what the user did, as intent rather than as key codes.
//! - [`state`] — what the interface remembers between frames, and what input does to it.
//! - [`strings`] — display text, behind a key rather than written into a layout.
//! - [`action`] — the closed set of effects a layout may attach to a control.
//! - [`paint`] — what to draw, as rectangles and text runs, and the theme that decides how.
//! - [`screen`] — which screens are open, in what order, and what each remembers.
//! - [`settings`] — an apply that must be confirmed, and a revert that need not be reached.
//! - [`shell`] — the navigable whole, and the routing between the two above.
//!
//! # A settings apply is undone by a machine, not by a user
//!
//! The other rule worth stating at the top. A display change can leave the person who made it unable
//! to see the screen well enough to undo it, so an undo that depends on them clicking is not an undo.
//! [`settings::Transaction`] inverts it: a change is applied, a window opens, and the *absence* of a
//! confirmation is what brings the previous settings back. See that module for the three values a
//! setting has at once and why a second apply inside the window keeps the first restore point.
//!
//! # Text input is not one character per keystroke
//!
//! Worth stating at the top because assuming otherwise is how an engine ends up unable to accept Chinese,
//! Japanese, or Korean text without being rebuilt. Under an input method, text passes through an
//! uncommitted **composition** that is replaced as the user types and committed only at the end — so
//! [`state::TextField`] holds that composition as a range inside itself, [`UiEvent`] carries it as its own
//! events, and a field has two readers: the text to *draw* and the field's *value*. See [`input`] for the
//! rest, and [`Interface::ime_wanted`] and [`Interface::ime_cursor_area`] for what a host must drive.
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
pub mod input;
pub mod layout;
pub mod paint;
pub mod screen;
pub mod settings;
pub mod shell;
pub mod solve;
pub mod state;
pub mod strings;

pub use action::Action;
pub use geometry::{Insets, Rect, Viewport, ViewportError};
pub use input::{Adjust, Edit, FocusMove, UiEvent};
pub use layout::{
    Align, DEFAULT_MAX_LENGTH, Direction, FORMAT_VERSION, Justify, Layout, LayoutError, Node,
    Range, Sizing, Style, Widget,
};
pub use paint::{Colour, Content, Painter, Primitive, TextAlign, Theme};
pub use screen::{Screen, ScreenStack, Screens, Transition};
pub use settings::{Probation, REVERT_WINDOW, Transaction};
pub use shell::{Outcome, Request, Shell, ShellError};
pub use solve::{Measure, NoContent, Solved, SolvedNode, solve};
pub use state::{Interface, TextField, Value};
pub use strings::StringTable;
