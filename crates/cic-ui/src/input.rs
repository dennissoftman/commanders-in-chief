//! What the interface accepts: intent, not device input.
//!
//! # Why there are no key codes here
//!
//! The same separation the camera uses, for the same reasons. A caller translates its own devices into
//! these events, so nothing in this crate knows about `winit`, scan codes, or which key a given keyboard
//! layout puts where — and the interaction logic becomes testable by constructing an event rather than by
//! synthesising a platform message.
//!
//! It also decouples the two things that get remapped independently. A key binding screen changes which
//! key produces [`UiEvent::Activate`]; it does not change what activation *means*. Had widgets read key
//! codes, every widget would have needed to know about rebinding.
//!
//! # Why editing is its own event and not a stream of characters
//!
//! [`Edit::Insert`] carries a character, but backspace, delete, and the cursor moves are named rather
//! than encoded as control characters. A platform reports those as key presses, not as text, and passing
//! `'\u{8}'` through a text channel would mean every consumer re-deriving the distinction the platform
//! had already made.
//!
//! # Why composition is separate from insertion
//!
//! A single character per keystroke is the *Latin* case, and assuming it is the only one is how an engine
//! ends up unable to accept Chinese, Japanese, or Korean text without being rebuilt. Under an input
//! method a user types several keys that produce no text at all, an **uncommitted composition** appears
//! and changes as they continue, candidates are chosen, and only then is text committed — possibly several
//! characters at once, possibly none.
//!
//! That cannot be expressed as a sequence of [`Edit::Insert`]s, because the composition is *replaced*
//! rather than appended to, and because it must be drawn differently — conventionally underlined — so the
//! user can see what is not yet real. So [`UiEvent::Compose`] carries the whole current composition each
//! time it changes, [`UiEvent::Commit`] carries finished text, and [`UiEvent::ComposeCancelled`] withdraws
//! a composition that was abandoned.
//!
//! This is the reason it is here in the first slice rather than added later: retrofitting it means
//! changing this vocabulary, the field's representation, *and* every renderer that had assumed one string
//! with one cursor.
//!
//! A caller on `winit` maps `Ime::Preedit` to `Compose`, `Ime::Commit` to `Commit`, and `Ime::Disabled`
//! to `ComposeCancelled`; it should also call `set_ime_allowed` and `set_ime_cursor_area` from
//! [`Interface::ime_wanted`](crate::Interface::ime_wanted) and
//! [`Interface::ime_cursor_area`](crate::Interface::ime_cursor_area), because an input method cannot place
//! its candidate window without being told where the text is.

/// One thing the user did, expressed as what it means rather than as what was pressed.
///
/// Not `Copy`, because composition carries owned text. Cloning an event is cheap for every other variant
/// and the alternative — a lifetime on the event type — would spread a borrow of the platform's buffer
/// through every widget.
#[derive(Debug, Clone, PartialEq)]
pub enum UiEvent {
    /// The pointer moved to a physical position.
    PointerMoved {
        /// Physical X.
        x: f32,
        /// Physical Y.
        y: f32,
    },
    /// The primary button went down at a physical position.
    PointerPressed {
        /// Physical X.
        x: f32,
        /// Physical Y.
        y: f32,
    },
    /// The primary button came up at a physical position.
    PointerReleased {
        /// Physical X.
        x: f32,
        /// Physical Y.
        y: f32,
    },
    /// The pointer left the surface entirely.
    PointerLeft,
    /// The wheel turned over a physical position.
    Scrolled {
        /// Physical X.
        x: f32,
        /// Physical Y.
        y: f32,
        /// How far, in logical units. Positive scrolls toward the content's end.
        amount: f32,
    },
    /// Move focus.
    Focus(FocusMove),
    /// Activate whatever holds focus.
    Activate,
    /// Nudge whatever holds focus along its own axis of adjustment.
    Adjust(Adjust),
    /// Edit the text of whatever holds focus.
    Edit(Edit),
    /// An input method's composition changed. Carries the whole current composition, not a delta.
    ///
    /// Replaces any composition already in progress. An empty `text` withdraws it, which is how some
    /// platforms report a composition being cleared without ending the session.
    Compose {
        /// The uncommitted text as it currently stands.
        text: String,
        /// Where the input method wants the caret, as a character offset into `text`.
        ///
        /// Absent when the platform does not say, in which case the end is the sensible place.
        cursor: Option<usize>,
    },
    /// An input method committed text, which may be several characters or none.
    Commit(String),
    /// A composition was abandoned without committing.
    ComposeCancelled,
    /// Back out of the current screen.
    Cancel,
}

/// Which way keyboard navigation goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusMove {
    /// The next focusable control in reading order, wrapping at the end.
    Next,
    /// The previous one, wrapping at the start.
    Previous,
}

/// Which way an adjustable control moves.
///
/// One pair for every kind that adjusts, rather than a separate event per widget: a slider, a list, and a
/// tab strip all answer "less" and "more", and what that means is the widget's business.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Adjust {
    /// Toward the minimum, or the previous entry.
    Decrease,
    /// Toward the maximum, or the next entry.
    Increase,
}

/// One text edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edit {
    /// Insert a character at the cursor.
    Insert(char),
    /// Delete the character before the cursor.
    Backspace,
    /// Delete the character after the cursor.
    Delete,
    /// Move the cursor one character toward the start.
    Left,
    /// Move the cursor one character toward the end.
    Right,
    /// Move the cursor to the start.
    Home,
    /// Move the cursor to the end.
    End,
}
