//! What the interface remembers between frames, and what input does to it.
//!
//! # Why state is keyed by id and not by position
//!
//! A layout is re-solved whenever the window changes size, so every rectangle and every index in a solved
//! frame is new. State keyed by either would be lost on a resize — a half-typed name cleared by dragging
//! the window edge. Keyed by the id the layout author wrote, it survives, and that is why the format
//! *requires* an id on any widget that holds state or takes focus rather than treating it as optional.
//!
//! # Why values are not in the layout file
//!
//! A slider's range is in the layout because it describes the control. Its value is not, because it
//! describes whatever the screen is editing — and a layout file stating one would be a second source of
//! truth for a setting the host already owns. So the host seeds values in, and reads them back out.
//!
//! # Why a press arms rather than activates
//!
//! Pressing a button and releasing it somewhere else must not activate it: that is how a user cancels
//! after realising they aimed at the wrong control, and it is universal enough that its absence reads as a
//! bug. So a press records what was armed and a release fires only if it lands on the same node.
//!
//! # Why this returns at most one action
//!
//! One event is one intent. A release that both toggles a checkbox and reports its action has done one
//! thing the host cares about, and returning a list would invite a caller to handle actions in an order
//! this module never defined.

use std::collections::BTreeMap;

use crate::input::{Adjust, Edit, FocusMove, UiEvent};
use crate::layout::Widget;
use crate::solve::{Solved, SolvedNode};
use crate::{Action, Rect};

/// What one control remembers.
///
/// Deliberately one enum rather than a struct of options, so a checkbox cannot be asked for its scroll
/// offset. A mismatch between the stored kind and the widget's kind means the host seeded the wrong
/// thing, and the readers below report that as absence rather than coercing it.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// A checkbox's state.
    Toggle(bool),
    /// A slider's position, in the units of its own range.
    Slide(f32),
    /// A text entry's contents and cursor.
    Text(TextField),
    /// A scrollable container's offset, in physical pixels.
    Scroll(f32),
    /// A list's or a tab strip's chosen entry.
    Select(usize),
}

/// Editable text, where the cursor sits in it, and which part is not committed yet.
///
/// The cursor is a **character** index, not a byte offset. Byte offsets are what `String` indexes by and
/// what a naive implementation reaches for, and they put the cursor inside a multi-byte character the
/// first time somebody types one — which panics rather than merely looking wrong.
///
/// # The composition
///
/// Under an input method, text passes through an uncommitted stage: the user types keys that produce a
/// *composition* which changes as they go and becomes real only when committed. That composition is held
/// here, inside `text`, as a character range — so a renderer draws one string and underlines a span of it,
/// rather than stitching two strings together and getting the caret wrong at the join.
///
/// Two readers, deliberately: [`Self::text`] is what to *draw*, composition included, and
/// [`Self::committed`] is the field's *value*, which is what a host reading a setting wants. Conflating
/// them means a half-composed word is saved as if the user had finished typing it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextField {
    text: String,
    cursor: usize,
    /// Character range within `text` that an input method has not committed.
    composing: Option<std::ops::Range<usize>>,
}

impl TextField {
    /// Wraps text with the cursor at its end, which is where typing continues from.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let cursor = text.chars().count();
        Self {
            text,
            cursor,
            composing: None,
        }
    }

    /// Everything to draw, including any uncommitted composition.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The field's value: the text without any uncommitted composition.
    ///
    /// What a host reads when it saves a setting. Borrowed when nothing is composing, so the common case
    /// allocates nothing.
    #[must_use]
    pub fn committed(&self) -> std::borrow::Cow<'_, str> {
        match &self.composing {
            None => std::borrow::Cow::Borrowed(&self.text),
            Some(range) => {
                let from = self.offset(range.start);
                let to = self.offset(range.end);
                let mut committed = String::with_capacity(self.text.len() - (to - from));
                committed.push_str(&self.text[..from]);
                committed.push_str(&self.text[to..]);
                std::borrow::Cow::Owned(committed)
            }
        }
    }

    /// The character range an input method has not committed, for a renderer to mark.
    #[must_use]
    pub fn composition(&self) -> Option<std::ops::Range<usize>> {
        self.composing.clone()
    }

    /// Whether an input method is mid-composition.
    #[must_use]
    pub const fn is_composing(&self) -> bool {
        self.composing.is_some()
    }

    /// The cursor's character index.
    #[must_use]
    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    /// How many characters there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.text.chars().count()
    }

    /// Whether there is no text.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// The byte offset of a character index, saturating at the end.
    fn offset(&self, characters: usize) -> usize {
        self.text
            .char_indices()
            .nth(characters)
            .map_or(self.text.len(), |(offset, _)| offset)
    }

    /// Replaces the current composition with `preedit`, placing the caret where the method asks.
    ///
    /// Bounded like typed text is: the composition is truncated to whatever room the *committed* text
    /// leaves, so a runaway input method cannot grow the field past its limit.
    fn compose(&mut self, preedit: &str, cursor: Option<usize>, limit: usize) {
        let start = self.replace_composition("");
        let committed = self.text.chars().count();
        let room = limit.saturating_sub(committed);
        let accepted: String = preedit
            .chars()
            .filter(|character| !character.is_control())
            .take(room)
            .collect();
        if accepted.is_empty() {
            // An empty composition is a withdrawn one, not a zero-length span to keep marking.
            self.composing = None;
            self.cursor = start.min(self.text.chars().count());
            return;
        }
        let length = accepted.chars().count();
        let at = self.offset(start);
        self.text.insert_str(at, &accepted);
        self.composing = Some(start..start + length);
        // The caret sits inside the composition where the method put it, which is how a user sees where
        // in the half-formed word they are.
        self.cursor = start + cursor.unwrap_or(length).min(length);
    }

    /// Turns committed text into real text, replacing any composition in progress.
    fn commit(&mut self, text: &str, limit: usize) {
        let start = self.replace_composition("");
        self.cursor = start;
        for character in text.chars() {
            self.apply(Edit::Insert(character), limit);
        }
    }

    /// Drops any composition, leaving the committed text untouched.
    fn cancel_composition(&mut self) {
        let start = self.replace_composition("");
        self.cursor = start.min(self.text.chars().count());
    }

    /// Swaps whatever is composing for `replacement`, returning where the composition began.
    ///
    /// Returns the cursor when nothing was composing, so a caller can treat "replace the composition"
    /// and "insert at the cursor" as one operation.
    fn replace_composition(&mut self, replacement: &str) -> usize {
        let Some(range) = self.composing.take() else {
            return self.cursor;
        };
        let from = self.offset(range.start);
        let to = self.offset(range.end);
        self.text.replace_range(from..to, replacement);
        range.start
    }

    /// Applies one edit, bounded by `limit` characters.
    fn apply(&mut self, edit: Edit, limit: usize) {
        // A plain edit arriving mid-composition means the platform routed a key to us rather than to the
        // input method. Accepting what was composed is the least destructive reading: dropping it would
        // discard characters the user has already seen on screen.
        if self.composing.is_some() {
            self.composing = None;
        }
        match edit {
            Edit::Insert(character) => {
                // Control characters arrive as named edits, so anything that is one here came from a
                // caller passing raw key input through the text channel. Dropping it keeps a stray
                // `'\n'` out of a single-line field.
                if character.is_control() || self.len() >= limit {
                    return;
                }
                let at = self.offset(self.cursor);
                self.text.insert(at, character);
                self.cursor += 1;
            }
            Edit::Backspace => {
                if self.cursor > 0 {
                    let from = self.offset(self.cursor - 1);
                    let to = self.offset(self.cursor);
                    self.text.replace_range(from..to, "");
                    self.cursor -= 1;
                }
            }
            Edit::Delete => {
                if self.cursor < self.len() {
                    let from = self.offset(self.cursor);
                    let to = self.offset(self.cursor + 1);
                    self.text.replace_range(from..to, "");
                }
            }
            Edit::Left => self.cursor = self.cursor.saturating_sub(1),
            Edit::Right => self.cursor = (self.cursor + 1).min(self.len()),
            Edit::Home => self.cursor = 0,
            Edit::End => self.cursor = self.len(),
        }
    }
}

/// The interface's live state: what is focused, what the pointer is over, and what every control holds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Interface {
    focus: Option<String>,
    hover: Option<String>,
    armed: Option<String>,
    values: BTreeMap<String, Value>,
}

impl Interface {
    /// A fresh interface with nothing focused and nothing remembered.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The focused control's id.
    #[must_use]
    pub fn focus(&self) -> Option<&str> {
        self.focus.as_deref()
    }

    /// The hovered control's id.
    #[must_use]
    pub fn hover(&self) -> Option<&str> {
        self.hover.as_deref()
    }

    /// The armed control's id — pressed, not yet released.
    #[must_use]
    pub fn armed(&self) -> Option<&str> {
        self.armed.as_deref()
    }

    /// Focuses a control by id, or clears focus with `None`.
    pub fn set_focus(&mut self, id: Option<&str>) {
        self.focus = id.map(str::to_owned);
    }

    /// Stores a value, replacing whatever was there.
    pub fn set(&mut self, id: impl Into<String>, value: Value) {
        self.values.insert(id.into(), value);
    }

    /// Seeds a checkbox.
    pub fn set_toggle(&mut self, id: impl Into<String>, on: bool) {
        self.set(id, Value::Toggle(on));
    }

    /// Seeds a slider.
    pub fn set_slide(&mut self, id: impl Into<String>, value: f32) {
        self.set(id, Value::Slide(value));
    }

    /// Seeds a text entry, cursor at the end.
    pub fn set_text(&mut self, id: impl Into<String>, text: impl Into<String>) {
        self.set(id, Value::Text(TextField::new(text)));
    }

    /// Seeds a list's or tab strip's selection.
    pub fn set_selection(&mut self, id: impl Into<String>, index: usize) {
        self.set(id, Value::Select(index));
    }

    /// Reads any stored value.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Value> {
        self.values.get(id)
    }

    /// A checkbox's state, or nothing if that id holds no toggle.
    #[must_use]
    pub fn toggle(&self, id: &str) -> Option<bool> {
        match self.values.get(id) {
            Some(Value::Toggle(on)) => Some(*on),
            _ => None,
        }
    }

    /// A slider's value.
    #[must_use]
    pub fn slide(&self, id: &str) -> Option<f32> {
        match self.values.get(id) {
            Some(Value::Slide(value)) => Some(*value),
            _ => None,
        }
    }

    /// A text entry's field.
    #[must_use]
    pub fn text(&self, id: &str) -> Option<&TextField> {
        match self.values.get(id) {
            Some(Value::Text(field)) => Some(field),
            _ => None,
        }
    }

    /// A list's or tab strip's selection.
    #[must_use]
    pub fn selection(&self, id: &str) -> Option<usize> {
        match self.values.get(id) {
            Some(Value::Select(index)) => Some(*index),
            _ => None,
        }
    }

    /// A scrollable container's offset, which is zero until it has been scrolled.
    #[must_use]
    pub fn scroll(&self, id: &str) -> f32 {
        match self.values.get(id) {
            Some(Value::Scroll(offset)) => *offset,
            _ => 0.0,
        }
    }

    /// Applies one event against a solved layout, reporting the action it triggered, if any.
    pub fn handle(&mut self, solved: &Solved, event: UiEvent) -> Option<Action> {
        match event {
            UiEvent::PointerMoved { x, y } => {
                self.hover = solved
                    .hit_focusable(x, y)
                    .and_then(|index| solved.get(index))
                    .and_then(|node| node.id.clone());
                // A slider tracks the pointer while armed, which is what makes it a slider rather than a
                // pair of buttons. Only the armed one: dragging must not capture a control the press
                // never touched.
                self.drag(solved, x);
                None
            }
            UiEvent::PointerPressed { x, y } => {
                let hit = solved
                    .hit_focusable(x, y)
                    .and_then(|index| solved.get(index));
                self.armed = hit.and_then(|node| node.id.clone());
                if self.armed.is_some() {
                    self.focus = self.armed.clone();
                }
                // A press on a slider jumps to where it landed, before any drag.
                self.drag(solved, x);
                None
            }
            UiEvent::PointerReleased { x, y } => {
                let armed = self.armed.take()?;
                let landed = solved
                    .hit_focusable(x, y)
                    .and_then(|index| solved.get(index))
                    .and_then(|node| node.id.as_deref());
                // Released somewhere else: the user aimed away deliberately, so nothing fires.
                if landed != Some(armed.as_str()) {
                    return None;
                }
                let index = solved.index_of(&armed)?;
                self.activate(solved.get(index)?)
            }
            UiEvent::PointerLeft => {
                self.hover = None;
                None
            }
            UiEvent::Scrolled { x, y, amount } => {
                self.scroll_at(solved, x, y, amount);
                None
            }
            UiEvent::Focus(direction) => {
                self.move_focus(solved, direction);
                None
            }
            UiEvent::Activate => {
                let focused = self.focus.clone()?;
                let index = solved.index_of(&focused)?;
                self.activate(solved.get(index)?)
            }
            UiEvent::Adjust(direction) => {
                self.adjust(solved, direction);
                None
            }
            UiEvent::Edit(edit) => {
                self.with_text_field(solved, |field, limit| field.apply(edit, limit));
                None
            }
            UiEvent::Compose { text, cursor } => {
                self.with_text_field(solved, |field, limit| field.compose(&text, cursor, limit));
                None
            }
            UiEvent::Commit(text) => {
                self.with_text_field(solved, |field, limit| field.commit(&text, limit));
                None
            }
            UiEvent::ComposeCancelled => {
                self.with_text_field(solved, |field, _| field.cancel_composition());
                None
            }
            // Escape means "leave this screen", which the screen stack interprets. Returning it as an
            // action rather than popping anything here keeps this module free of navigation.
            UiEvent::Cancel => Some(Action::Back),
        }
    }

    /// Activates one node: toggles what toggles, then reports whatever action it carries.
    fn activate(&mut self, node: &SolvedNode) -> Option<Action> {
        if let Some(id) = node
            .id
            .as_deref()
            .filter(|_| node.widget == Widget::Checkbox)
        {
            let flipped = !self.toggle(id).unwrap_or(false);
            self.set(id.to_owned(), Value::Toggle(flipped));
        }
        node.action
    }

    /// Moves an armed slider to wherever the pointer is along its track.
    fn drag(&mut self, solved: &Solved, x: f32) {
        let Some(armed) = self.armed.clone() else {
            return;
        };
        let Some(node) = solved.index_of(&armed).and_then(|index| solved.get(index)) else {
            return;
        };
        if node.widget != Widget::Slider {
            return;
        }
        let Some(range) = node.range else {
            return;
        };
        self.set(armed, Value::Slide(range.at(fraction_along(node.rect, x))));
    }

    /// Scrolls the nearest enclosing scrollable container under a point.
    ///
    /// Nearest enclosing rather than whatever is directly under the pointer, because the thing under it
    /// is a row and the thing that scrolls is the container the row is in.
    fn scroll_at(&mut self, solved: &Solved, x: f32, y: f32, amount: f32) {
        let Some(hit) = solved
            .nodes()
            .iter()
            .rposition(|node| node.rect.contains(x, y))
        else {
            return;
        };
        let Some(index) = solved.enclosing(hit, Widget::Scroll) else {
            return;
        };
        let Some(id) = solved.get(index).and_then(|node| node.id.clone()) else {
            return;
        };
        let limit = solved.scroll_limit(index);
        let offset = (self.scroll(&id) + amount).clamp(0.0, limit);
        self.set(id, Value::Scroll(offset));
    }

    /// Moves focus through the focusable controls in reading order, wrapping at both ends.
    fn move_focus(&mut self, solved: &Solved, direction: FocusMove) {
        let order = solved.focus_order();
        if order.is_empty() {
            self.focus = None;
            return;
        }
        let current = self
            .focus
            .as_deref()
            .and_then(|id| order.iter().position(|entry| *entry == id));
        let next = match (current, direction) {
            // Nothing focused yet: the first control going forward, the last going back, so a single
            // press from a fresh screen lands somewhere useful either way.
            (None, FocusMove::Next) => 0,
            (None, FocusMove::Previous) => order.len() - 1,
            (Some(at), FocusMove::Next) => (at + 1) % order.len(),
            (Some(at), FocusMove::Previous) => (at + order.len() - 1) % order.len(),
        };
        self.focus = Some(order[next].to_owned());
    }

    /// Nudges the focused control along its own axis.
    fn adjust(&mut self, solved: &Solved, direction: Adjust) {
        let Some(focused) = self.focus.clone() else {
            return;
        };
        let Some(node) = solved
            .index_of(&focused)
            .and_then(|index| solved.get(index))
        else {
            return;
        };
        match node.widget {
            Widget::Slider => {
                let Some(range) = node.range else { return };
                let current = self.slide(&focused).unwrap_or(range.min);
                let step = match direction {
                    Adjust::Decrease => -range.step,
                    Adjust::Increase => range.step,
                };
                self.set(focused, Value::Slide(range.clamp(current + step)));
            }
            Widget::List | Widget::Tabs => {
                // Bounded by the children the layout actually has, so a selection can never name an
                // entry that is not there. Clamped rather than wrapped: a list that jumps from the last
                // row to the first on one key press reads as a lost keystroke.
                if node.children == 0 {
                    return;
                }
                let current = self.selection(&focused).unwrap_or(0);
                let next = match direction {
                    Adjust::Decrease => current.saturating_sub(1),
                    Adjust::Increase => (current + 1).min(node.children - 1),
                };
                self.set(focused, Value::Select(next));
            }
            _ => {}
        }
    }

    /// Runs an operation on the focused text entry's field, within the limit the layout set.
    ///
    /// One place that resolves "which field, and how long may it be", so typing, composing, committing and
    /// cancelling cannot disagree about either.
    fn with_text_field(&mut self, solved: &Solved, operation: impl FnOnce(&mut TextField, usize)) {
        let Some(focused) = self.focus.clone() else {
            return;
        };
        let Some(node) = solved
            .index_of(&focused)
            .and_then(|index| solved.get(index))
        else {
            return;
        };
        if node.widget != Widget::TextEntry {
            return;
        }
        let mut field = match self.values.remove(&focused) {
            Some(Value::Text(field)) => field,
            // Anything else means the host seeded the wrong kind, or nothing at all. Starting from empty
            // beats refusing to type.
            _ => TextField::default(),
        };
        operation(&mut field, node.text_limit());
        self.set(focused, Value::Text(field));
    }

    /// Whether an input method should be enabled, which is true exactly while a text entry has focus.
    ///
    /// A caller drives `set_ime_allowed` from this. Leaving an input method on everywhere means its
    /// candidate window can appear over a menu, and leaving it off means CJK text cannot be typed at all.
    #[must_use]
    pub fn ime_wanted(&self, solved: &Solved) -> bool {
        self.focused_node(solved)
            .is_some_and(|node| node.widget == Widget::TextEntry)
    }

    /// Where an input method should put its candidate window, in physical pixels.
    ///
    /// The focused entry's own rectangle. A caret-tight rectangle would be better and needs text metrics
    /// this crate deliberately does not have — measurement arrives through a trait — so the honest answer
    /// is the field, and a renderer that knows its font can narrow it.
    #[must_use]
    pub fn ime_cursor_area(&self, solved: &Solved) -> Option<Rect> {
        self.focused_node(solved)
            .filter(|node| node.widget == Widget::TextEntry)
            .map(|node| node.rect)
    }

    /// The focused node, if it is still in the current layout.
    fn focused_node<'a>(&self, solved: &'a Solved) -> Option<&'a SolvedNode> {
        let focused = self.focus.as_deref()?;
        solved.index_of(focused).and_then(|index| solved.get(index))
    }
}

/// How far along a rectangle's width a physical X sits, from zero to one.
fn fraction_along(rect: Rect, x: f32) -> f32 {
    if rect.width <= 0.0 {
        return 0.0;
    }
    ((x - rect.x) / rect.width).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    // Slider values below are exact multiples of a step over a range chosen to be representable, and the
    // band coordinates are small integers, so exact comparison is the assertion and the conversion in
    // `in_band` cannot lose anything.
    #![allow(clippy::cast_precision_loss, clippy::float_cmp)]

    use super::{Interface, TextField, Value};
    use crate::input::{Adjust, Edit, FocusMove, UiEvent};
    use crate::layout::{Layout, Node, Range, Sizing, Widget};
    use crate::solve::{NoContent, Solved, solve};
    use crate::{Action, Viewport};

    fn band(id: &str, widget: Widget, height: f32) -> Node {
        Node {
            id: Some(id.to_owned()),
            widget,
            width: Sizing::Fixed(100.0),
            height: Sizing::Fixed(height),
            ..Node::default()
        }
    }

    fn row(height: f32) -> Node {
        Node {
            width: Sizing::Fixed(100.0),
            height: Sizing::Fixed(height),
            ..Node::default()
        }
    }

    /// A settings-shaped screen: one of everything that behaves, stacked in bands 20 tall.
    ///
    /// Laid out as a column of fixed heights so every control occupies a known strip and a test can name
    /// a point inside one without solving anything by hand.
    fn screen() -> Layout {
        Layout {
            format_version: crate::layout::FORMAT_VERSION,
            root: Node {
                width: Sizing::Fill(1),
                height: Sizing::Fill(1),
                children: vec![
                    Node {
                        action: Some(Action::ApplySettings),
                        ..band("check", Widget::Checkbox, 20.0)
                    },
                    Node {
                        range: Some(Range {
                            min: 0.0,
                            max: 10.0,
                            step: 1.0,
                        }),
                        ..band("volume", Widget::Slider, 20.0)
                    },
                    Node {
                        max_length: Some(5),
                        ..band("name", Widget::TextEntry, 20.0)
                    },
                    Node {
                        children: vec![row(20.0), row(20.0), row(20.0)],
                        ..band("tabs", Widget::Tabs, 20.0)
                    },
                    Node {
                        action: Some(Action::ConfirmSettings),
                        ..band("ok", Widget::Button, 20.0)
                    },
                    Node {
                        children: vec![row(40.0), row(40.0), row(40.0)],
                        ..band("scroller", Widget::Scroll, 50.0)
                    },
                ],
                ..Node::default()
            },
        }
    }

    fn solved_at(width: u32, height: u32, scale: f32) -> Solved {
        let layout = screen();
        layout.validate().expect("the fixture must be valid");
        let viewport = Viewport::new(width, height, scale).expect("valid viewport");
        solve(&layout, viewport, &NoContent)
    }

    fn solved() -> Solved {
        solved_at(200, 400, 1.0)
    }

    /// Bands are 20 tall in declaration order, so this is the middle of the nth.
    const fn in_band(index: usize) -> (f32, f32) {
        (50.0, 20.0 * index as f32 + 10.0)
    }

    #[test]
    fn hover_follows_the_pointer_over_controls_and_clears_when_it_leaves() {
        let solved = solved();
        let mut ui = Interface::new();
        let (x, y) = in_band(0);
        ui.handle(&solved, UiEvent::PointerMoved { x, y });
        assert_eq!(ui.hover(), Some("check"));
        let (x, y) = in_band(4);
        ui.handle(&solved, UiEvent::PointerMoved { x, y });
        assert_eq!(ui.hover(), Some("ok"));
        // Off every control but still on the root, which is a panel and so not hoverable.
        ui.handle(&solved, UiEvent::PointerMoved { x: 180.0, y: 300.0 });
        assert_eq!(ui.hover(), None);
        let (x, y) = in_band(0);
        ui.handle(&solved, UiEvent::PointerMoved { x, y });
        ui.handle(&solved, UiEvent::PointerLeft);
        assert_eq!(ui.hover(), None);
    }

    #[test]
    fn a_press_arms_and_focuses_but_does_not_activate() {
        let solved = solved();
        let mut ui = Interface::new();
        let (x, y) = in_band(4);
        assert_eq!(ui.handle(&solved, UiEvent::PointerPressed { x, y }), None);
        assert_eq!(ui.armed(), Some("ok"));
        assert_eq!(ui.focus(), Some("ok"));
    }

    #[test]
    fn a_release_on_the_armed_control_fires_its_action() {
        let solved = solved();
        let mut ui = Interface::new();
        let (x, y) = in_band(4);
        ui.handle(&solved, UiEvent::PointerPressed { x, y });
        assert_eq!(
            ui.handle(&solved, UiEvent::PointerReleased { x, y }),
            Some(Action::ConfirmSettings)
        );
        assert_eq!(ui.armed(), None, "releasing disarms");
    }

    #[test]
    fn pressing_a_control_and_releasing_elsewhere_activates_nothing() {
        // How a user cancels after realising they aimed wrong. Its absence reads as a bug, which is why
        // a press arms rather than activating.
        let solved = solved();
        let mut ui = Interface::new();
        let (x, y) = in_band(4);
        ui.handle(&solved, UiEvent::PointerPressed { x, y });
        // Released over a different control.
        let (other_x, other_y) = in_band(0);
        assert_eq!(
            ui.handle(
                &solved,
                UiEvent::PointerReleased {
                    x: other_x,
                    y: other_y
                }
            ),
            None
        );
        assert_eq!(ui.armed(), None);
        // And the checkbox it was released over must not have toggled either.
        assert_eq!(ui.toggle("check"), None);
        // Released over nothing at all.
        ui.handle(&solved, UiEvent::PointerPressed { x, y });
        assert_eq!(
            ui.handle(&solved, UiEvent::PointerReleased { x: 180.0, y: 300.0 }),
            None
        );
    }

    #[test]
    fn a_release_with_nothing_armed_does_nothing() {
        let solved = solved();
        let mut ui = Interface::new();
        let (x, y) = in_band(4);
        assert_eq!(ui.handle(&solved, UiEvent::PointerReleased { x, y }), None);
    }

    #[test]
    fn a_checkbox_toggles_on_each_activation_and_still_reports_its_action() {
        let solved = solved();
        let mut ui = Interface::new();
        let (x, y) = in_band(0);
        ui.handle(&solved, UiEvent::PointerPressed { x, y });
        assert_eq!(
            ui.handle(&solved, UiEvent::PointerReleased { x, y }),
            Some(Action::ApplySettings)
        );
        assert_eq!(ui.toggle("check"), Some(true));
        ui.handle(&solved, UiEvent::PointerPressed { x, y });
        ui.handle(&solved, UiEvent::PointerReleased { x, y });
        assert_eq!(ui.toggle("check"), Some(false), "and back again");
    }

    #[test]
    fn a_slider_jumps_to_a_press_and_then_tracks_the_drag() {
        // The track spans x 0..100 and the range 0..10, so the arithmetic is readable.
        let solved = solved();
        let mut ui = Interface::new();
        let (_, y) = in_band(1);
        ui.handle(&solved, UiEvent::PointerPressed { x: 50.0, y });
        assert_eq!(ui.slide("volume"), Some(5.0));
        ui.handle(&solved, UiEvent::PointerMoved { x: 80.0, y });
        assert_eq!(ui.slide("volume"), Some(8.0));
        // Past either end clamps rather than extrapolating.
        ui.handle(&solved, UiEvent::PointerMoved { x: -40.0, y });
        assert_eq!(ui.slide("volume"), Some(0.0));
        ui.handle(&solved, UiEvent::PointerMoved { x: 400.0, y });
        assert_eq!(ui.slide("volume"), Some(10.0));
        // Releasing ends the drag, so later movement leaves it alone.
        ui.handle(&solved, UiEvent::PointerReleased { x: 400.0, y });
        ui.handle(&solved, UiEvent::PointerMoved { x: 10.0, y });
        assert_eq!(ui.slide("volume"), Some(10.0));
    }

    #[test]
    fn a_drag_only_moves_the_slider_the_press_landed_on() {
        // Otherwise moving the pointer after pressing a button would drive a slider it never touched.
        let solved = solved();
        let mut ui = Interface::new();
        ui.set_slide("volume", 3.0);
        let (x, y) = in_band(4);
        ui.handle(&solved, UiEvent::PointerPressed { x, y });
        let (_, slider_y) = in_band(1);
        ui.handle(
            &solved,
            UiEvent::PointerMoved {
                x: 90.0,
                y: slider_y,
            },
        );
        assert_eq!(ui.slide("volume"), Some(3.0), "untouched");
    }

    #[test]
    fn a_slider_steps_by_its_own_step_and_stops_at_its_own_ends() {
        let solved = solved();
        let mut ui = Interface::new();
        ui.set_focus(Some("volume"));
        ui.set_slide("volume", 9.0);
        ui.handle(&solved, UiEvent::Adjust(Adjust::Increase));
        assert_eq!(ui.slide("volume"), Some(10.0));
        ui.handle(&solved, UiEvent::Adjust(Adjust::Increase));
        assert_eq!(ui.slide("volume"), Some(10.0), "clamped at the maximum");
        for _ in 0..12 {
            ui.handle(&solved, UiEvent::Adjust(Adjust::Decrease));
        }
        assert_eq!(ui.slide("volume"), Some(0.0), "clamped at the minimum");
    }

    #[test]
    fn focus_moves_in_reading_order_and_wraps_at_both_ends() {
        let solved = solved();
        assert_eq!(
            solved.focus_order(),
            vec!["check", "volume", "name", "tabs", "ok"],
            "the scroll container is deliberately not a focus stop"
        );
        let mut ui = Interface::new();
        // From nothing, forward lands on the first and backward on the last.
        ui.handle(&solved, UiEvent::Focus(FocusMove::Next));
        assert_eq!(ui.focus(), Some("check"));
        for expected in ["volume", "name", "tabs", "ok", "check"] {
            ui.handle(&solved, UiEvent::Focus(FocusMove::Next));
            assert_eq!(ui.focus(), Some(expected));
        }
        ui.handle(&solved, UiEvent::Focus(FocusMove::Previous));
        assert_eq!(ui.focus(), Some("ok"), "wrapping backward past the start");
        let mut fresh = Interface::new();
        fresh.handle(&solved, UiEvent::Focus(FocusMove::Previous));
        assert_eq!(fresh.focus(), Some("ok"));
    }

    #[test]
    fn activating_the_focused_control_needs_no_pointer_at_all() {
        // The keyboard path has to reach the same activation the pointer does, or a keyboard user cannot
        // press a button.
        let solved = solved();
        let mut ui = Interface::new();
        ui.set_focus(Some("ok"));
        assert_eq!(
            ui.handle(&solved, UiEvent::Activate),
            Some(Action::ConfirmSettings)
        );
        ui.set_focus(Some("check"));
        assert_eq!(
            ui.handle(&solved, UiEvent::Activate),
            Some(Action::ApplySettings)
        );
        assert_eq!(ui.toggle("check"), Some(true));
        // Nothing focused, nothing happens.
        let mut idle = Interface::new();
        assert_eq!(idle.handle(&solved, UiEvent::Activate), None);
    }

    #[test]
    fn a_tab_strip_selects_within_the_children_it_actually_has() {
        // Bounded by the layout, so a selection can never name an entry that is not there. Clamped rather
        // than wrapped: jumping from the last tab to the first on one press reads as a lost keystroke.
        let solved = solved();
        let mut ui = Interface::new();
        ui.set_focus(Some("tabs"));
        assert_eq!(ui.selection("tabs"), None);
        ui.handle(&solved, UiEvent::Adjust(Adjust::Increase));
        assert_eq!(ui.selection("tabs"), Some(1));
        ui.handle(&solved, UiEvent::Adjust(Adjust::Increase));
        assert_eq!(ui.selection("tabs"), Some(2));
        ui.handle(&solved, UiEvent::Adjust(Adjust::Increase));
        assert_eq!(
            ui.selection("tabs"),
            Some(2),
            "three children, so 2 is last"
        );
        for _ in 0..5 {
            ui.handle(&solved, UiEvent::Adjust(Adjust::Decrease));
        }
        assert_eq!(ui.selection("tabs"), Some(0));
    }

    #[test]
    fn the_wheel_scrolls_the_container_a_row_is_inside() {
        // Nearest enclosing, because what is under the pointer is a row and what scrolls is the box the
        // row is in. Content is three 40-tall rows in a 50-tall box, so the limit is 70.
        let solved = solved();
        let scroller = solved.index_of("scroller").expect("the fixture has one");
        assert_eq!(solved.scroll_limit(scroller), 70.0);
        let mut ui = Interface::new();
        assert_eq!(ui.scroll("scroller"), 0.0);
        ui.handle(
            &solved,
            UiEvent::Scrolled {
                x: 50.0,
                y: 120.0,
                amount: 30.0,
            },
        );
        assert_eq!(ui.scroll("scroller"), 30.0);
        ui.handle(
            &solved,
            UiEvent::Scrolled {
                x: 50.0,
                y: 120.0,
                amount: 500.0,
            },
        );
        assert_eq!(ui.scroll("scroller"), 70.0, "clamped to the content");
        ui.handle(
            &solved,
            UiEvent::Scrolled {
                x: 50.0,
                y: 120.0,
                amount: -500.0,
            },
        );
        assert_eq!(ui.scroll("scroller"), 0.0, "and never past the start");
    }

    #[test]
    fn the_wheel_over_nothing_scrollable_changes_nothing() {
        let solved = solved();
        let mut ui = Interface::new();
        let (x, y) = in_band(0);
        ui.handle(&solved, UiEvent::Scrolled { x, y, amount: 30.0 });
        assert_eq!(ui.scroll("scroller"), 0.0);
        // Outside the layout entirely.
        ui.handle(
            &solved,
            UiEvent::Scrolled {
                x: 900.0,
                y: 900.0,
                amount: 30.0,
            },
        );
        assert_eq!(ui.scroll("scroller"), 0.0);
    }

    #[test]
    fn typing_inserts_at_the_cursor_and_the_cursor_moves() {
        let solved = solved();
        let mut ui = Interface::new();
        ui.set_focus(Some("name"));
        for character in ['a', 'b', 'c'] {
            ui.handle(&solved, UiEvent::Edit(Edit::Insert(character)));
        }
        let field = ui.text("name").expect("typed text");
        assert_eq!(field.text(), "abc");
        assert_eq!(field.cursor(), 3);
        ui.handle(&solved, UiEvent::Edit(Edit::Left));
        ui.handle(&solved, UiEvent::Edit(Edit::Insert('X')));
        assert_eq!(ui.text("name").map(TextField::text), Some("abXc"));
        ui.handle(&solved, UiEvent::Edit(Edit::Home));
        ui.handle(&solved, UiEvent::Edit(Edit::Insert('-')));
        assert_eq!(ui.text("name").map(TextField::text), Some("-abXc"));
    }

    #[test]
    fn backspace_and_delete_remove_on_either_side_of_the_cursor() {
        let solved = solved();
        let mut ui = Interface::new();
        ui.set_focus(Some("name"));
        ui.set_text("name", "abcd");
        ui.handle(&solved, UiEvent::Edit(Edit::Backspace));
        assert_eq!(ui.text("name").map(TextField::text), Some("abc"));
        ui.handle(&solved, UiEvent::Edit(Edit::Home));
        ui.handle(&solved, UiEvent::Edit(Edit::Delete));
        assert_eq!(ui.text("name").map(TextField::text), Some("bc"));
        // At the ends, neither does anything rather than underflowing.
        ui.handle(&solved, UiEvent::Edit(Edit::Backspace));
        assert_eq!(ui.text("name").map(TextField::text), Some("bc"));
        ui.handle(&solved, UiEvent::Edit(Edit::End));
        ui.handle(&solved, UiEvent::Edit(Edit::Delete));
        assert_eq!(ui.text("name").map(TextField::text), Some("bc"));
    }

    #[test]
    fn the_cursor_is_a_character_index_so_multi_byte_text_does_not_panic() {
        // The bug this is built to prevent: `String` indexes by bytes, so a cursor held as a byte offset
        // lands inside a multi-byte character and the next edit panics on a boundary. Every character
        // here is more than one byte.
        let solved = solved();
        let mut ui = Interface::new();
        ui.set_focus(Some("name"));
        for character in ['é', 'ü', '中'] {
            ui.handle(&solved, UiEvent::Edit(Edit::Insert(character)));
        }
        assert_eq!(ui.text("name").map(TextField::text), Some("éü中"));
        assert_eq!(ui.text("name").map(TextField::cursor), Some(3));
        ui.handle(&solved, UiEvent::Edit(Edit::Left));
        ui.handle(&solved, UiEvent::Edit(Edit::Insert('🙂')));
        assert_eq!(ui.text("name").map(TextField::text), Some("éü🙂中"));
        ui.handle(&solved, UiEvent::Edit(Edit::Backspace));
        assert_eq!(ui.text("name").map(TextField::text), Some("éü中"));
        ui.handle(&solved, UiEvent::Edit(Edit::Home));
        ui.handle(&solved, UiEvent::Edit(Edit::Delete));
        assert_eq!(ui.text("name").map(TextField::text), Some("ü中"));
    }

    #[test]
    fn typed_text_stops_at_the_limit_the_layout_set() {
        // Typed input is input, and nothing here grows without a bound somebody chose. The fixture sets
        // five.
        let solved = solved();
        let mut ui = Interface::new();
        ui.set_focus(Some("name"));
        for character in "abcdefghij".chars() {
            ui.handle(&solved, UiEvent::Edit(Edit::Insert(character)));
        }
        let field = ui.text("name").expect("typed text");
        assert_eq!(field.text(), "abcde");
        assert_eq!(field.len(), 5);
    }

    #[test]
    fn a_control_character_arriving_as_text_is_dropped() {
        // Backspace and the cursor moves are named events. Anything control-shaped reaching the text
        // channel came from a caller passing raw key input through it, and a stray newline in a
        // single-line field is worse than a dropped keystroke.
        let solved = solved();
        let mut ui = Interface::new();
        ui.set_focus(Some("name"));
        ui.handle(&solved, UiEvent::Edit(Edit::Insert('a')));
        for character in ['\n', '\t', '\u{8}'] {
            ui.handle(&solved, UiEvent::Edit(Edit::Insert(character)));
        }
        assert_eq!(ui.text("name").map(TextField::text), Some("a"));
    }

    #[test]
    fn editing_reaches_nothing_unless_a_text_entry_holds_focus() {
        let solved = solved();
        let mut ui = Interface::new();
        ui.handle(&solved, UiEvent::Edit(Edit::Insert('a')));
        assert_eq!(ui.text("name"), None, "nothing focused");
        ui.set_focus(Some("ok"));
        ui.handle(&solved, UiEvent::Edit(Edit::Insert('a')));
        assert_eq!(ui.text("name"), None, "a button is not a text entry");
        // Adjusting a text entry does nothing either.
        ui.set_focus(Some("name"));
        ui.handle(&solved, UiEvent::Adjust(Adjust::Increase));
        assert_eq!(ui.text("name"), None);
    }

    #[test]
    fn a_value_of_the_wrong_kind_reads_as_absent_rather_than_being_coerced() {
        let mut ui = Interface::new();
        ui.set_toggle("check", true);
        assert_eq!(ui.toggle("check"), Some(true));
        assert_eq!(ui.slide("check"), None);
        assert_eq!(ui.selection("check"), None);
        assert_eq!(ui.text("check"), None);
        assert_eq!(ui.scroll("check"), 0.0);
        assert_eq!(ui.get("check"), Some(&Value::Toggle(true)));
        assert_eq!(ui.get("absent"), None);
    }

    #[test]
    fn retained_state_survives_being_solved_against_a_different_viewport() {
        // The property the whole keying scheme exists for. A resize re-solves everything, so state keyed
        // by rectangle or by index would be lost — a half-typed name cleared by dragging a window edge.
        let small = solved_at(200, 400, 1.0);
        let mut ui = Interface::new();
        ui.set_focus(Some("name"));
        // Within the fixture's five-character limit, so the insert at the end of this test is testing
        // relayout rather than the bound.
        ui.set_text("name", "Alp");
        ui.set_slide("volume", 7.0);
        ui.set_toggle("check", true);
        ui.handle(
            &small,
            UiEvent::Scrolled {
                x: 50.0,
                y: 120.0,
                amount: 25.0,
            },
        );

        // A different size *and* a different display scale, so every rectangle moved.
        let large = solved_at(800, 600, 2.0);
        assert_ne!(
            small.by_id("name").map(|node| node.rect),
            large.by_id("name").map(|node| node.rect),
            "the fixture must actually relayout, or this proves nothing"
        );
        assert_eq!(ui.text("name").map(TextField::text), Some("Alp"));
        assert_eq!(ui.slide("volume"), Some(7.0));
        assert_eq!(ui.toggle("check"), Some(true));
        assert_eq!(ui.scroll("scroller"), 25.0);
        assert_eq!(ui.focus(), Some("name"));
        // And typing continues into the same field against the new layout.
        ui.handle(&large, UiEvent::Edit(Edit::Insert('!')));
        assert_eq!(ui.text("name").map(TextField::text), Some("Alp!"));
    }

    #[test]
    fn seeding_past_the_limit_blocks_further_typing_rather_than_truncating() {
        // A host may seed a value longer than the layout's limit — a name loaded from a config the
        // limit later tightened. Truncating would destroy the host's data silently, so the value is kept
        // and only *growth* is refused. Found by a test that seeded six characters into a field of five.
        let solved = solved();
        let mut ui = Interface::new();
        ui.set_focus(Some("name"));
        ui.set_text("name", "Alpine Assault");
        ui.handle(&solved, UiEvent::Edit(Edit::Insert('!')));
        assert_eq!(
            ui.text("name").map(TextField::text),
            Some("Alpine Assault"),
            "kept whole, and not grown"
        );
        // Deleting still works, and once under the limit typing resumes.
        ui.set_text("name", "abcd");
        ui.handle(&solved, UiEvent::Edit(Edit::Insert('e')));
        assert_eq!(ui.text("name").map(TextField::text), Some("abcde"));
        ui.handle(&solved, UiEvent::Edit(Edit::Insert('f')));
        assert_eq!(ui.text("name").map(TextField::text), Some("abcde"));
    }

    /// A composition event, as a platform's input method would report it.
    fn compose(text: &str, cursor: Option<usize>) -> UiEvent {
        UiEvent::Compose {
            text: text.to_owned(),
            cursor,
        }
    }

    #[test]
    fn a_composition_replaces_itself_as_it_grows_rather_than_accumulating() {
        // The whole reason composition cannot be a sequence of inserts: each report carries the *current*
        // composition, so treating them as appends would spell "nnihnihanihao".
        let solved = solved();
        let mut ui = Interface::new();
        ui.set_focus(Some("name"));
        for stage in ["n", "ni", "nih", "niha", "nihao"] {
            ui.handle(&solved, compose(stage, None));
            let field = ui.text("name").expect("composing");
            assert_eq!(field.text(), stage);
            assert!(field.is_composing());
            // None of it is real yet, which is what a host reading the value must see.
            assert_eq!(field.committed(), "");
        }
        assert_eq!(
            ui.text("name").and_then(TextField::composition),
            Some(0..5),
            "the renderer needs the span to underline"
        );
    }

    #[test]
    fn committing_turns_a_composition_into_real_text() {
        let solved = solved();
        let mut ui = Interface::new();
        ui.set_focus(Some("name"));
        ui.handle(&solved, compose("nihao", None));
        // One commit can deliver several characters at once, which an insert-per-keystroke model cannot.
        ui.handle(&solved, UiEvent::Commit("你好".to_owned()));
        let field = ui.text("name").expect("committed");
        assert_eq!(field.text(), "你好");
        assert_eq!(field.committed(), "你好");
        assert!(!field.is_composing(), "nothing is pending any more");
        assert_eq!(field.composition(), None);
        assert_eq!(field.cursor(), 2, "the caret follows the committed text");
    }

    #[test]
    fn an_abandoned_composition_leaves_the_committed_text_alone() {
        let solved = solved();
        let mut ui = Interface::new();
        ui.set_focus(Some("name"));
        ui.set_text("name", "ab");
        ui.handle(&solved, compose("xyz", None));
        assert_eq!(ui.text("name").map(TextField::text), Some("abxyz"));
        ui.handle(&solved, UiEvent::ComposeCancelled);
        let field = ui.text("name").expect("still there");
        assert_eq!(field.text(), "ab", "only the composition went");
        assert_eq!(field.cursor(), 2);
        assert!(!field.is_composing());
    }

    #[test]
    fn a_composition_lands_at_the_cursor_and_not_at_the_end() {
        // A user moves the caret into the middle of a name and then starts composing there.
        let solved = solved();
        let mut ui = Interface::new();
        ui.set_focus(Some("name"));
        ui.set_text("name", "ad");
        ui.handle(&solved, UiEvent::Edit(Edit::Left));
        ui.handle(&solved, compose("bc", None));
        assert_eq!(ui.text("name").map(TextField::text), Some("abcd"));
        assert_eq!(ui.text("name").and_then(TextField::composition), Some(1..3));
        assert_eq!(ui.text("name").map(TextField::committed), Some("ad".into()));
        ui.handle(&solved, UiEvent::Commit("BC".to_owned()));
        assert_eq!(ui.text("name").map(TextField::text), Some("aBCd"));
    }

    #[test]
    fn the_caret_sits_where_the_input_method_asks_inside_the_composition() {
        // So the user can see where in the half-formed word they are.
        let solved = solved();
        let mut ui = Interface::new();
        ui.set_focus(Some("name"));
        ui.handle(&solved, compose("abcd", Some(2)));
        assert_eq!(ui.text("name").map(TextField::cursor), Some(2));
        // Absent means the end, which is the sensible default.
        ui.handle(&solved, compose("abcd", None));
        assert_eq!(ui.text("name").map(TextField::cursor), Some(4));
        // A cursor past the composition clamps into it rather than escaping.
        ui.handle(&solved, compose("ab", Some(99)));
        assert_eq!(ui.text("name").map(TextField::cursor), Some(2));
    }

    #[test]
    fn an_empty_composition_withdraws_the_marked_span() {
        // Some platforms clear a composition without ending the session, and a zero-length span left
        // marked would have a renderer underlining nothing at a stale position.
        let solved = solved();
        let mut ui = Interface::new();
        ui.set_focus(Some("name"));
        ui.handle(&solved, compose("ab", None));
        ui.handle(&solved, compose("", None));
        let field = ui.text("name").expect("still there");
        assert_eq!(field.text(), "");
        assert!(!field.is_composing());
        assert_eq!(field.composition(), None);
    }

    #[test]
    fn a_composition_cannot_grow_the_field_past_its_limit() {
        // Bounded like typed input, because an input method is input. The fixture allows five.
        let solved = solved();
        let mut ui = Interface::new();
        ui.set_focus(Some("name"));
        ui.set_text("name", "abc");
        ui.handle(&solved, compose("xyzzy", None));
        let field = ui.text("name").expect("composing");
        assert_eq!(field.text(), "abcxy", "truncated to the room left");
        assert_eq!(field.composition(), Some(3..5));
        // A commit is bounded the same way.
        ui.handle(&solved, UiEvent::Commit("XYZZY".to_owned()));
        assert_eq!(ui.text("name").map(TextField::text), Some("abcXY"));
    }

    #[test]
    fn a_keystroke_arriving_mid_composition_keeps_what_was_composed() {
        // Means the platform routed a key to us rather than to the input method. Dropping the composition
        // would discard characters the user has already seen on screen.
        let solved = solved();
        let mut ui = Interface::new();
        ui.set_focus(Some("name"));
        ui.handle(&solved, compose("ab", None));
        ui.handle(&solved, UiEvent::Edit(Edit::Insert('c')));
        let field = ui.text("name").expect("still there");
        assert_eq!(field.text(), "abc");
        assert!(!field.is_composing());
        assert_eq!(field.committed(), "abc", "all of it is real now");
    }

    #[test]
    fn composition_reaches_nothing_unless_a_text_entry_holds_focus() {
        let solved = solved();
        let mut ui = Interface::new();
        ui.handle(&solved, compose("ab", None));
        assert_eq!(ui.text("name"), None);
        ui.set_focus(Some("ok"));
        ui.handle(&solved, UiEvent::Commit("ab".to_owned()));
        assert_eq!(ui.text("name"), None);
    }

    #[test]
    fn an_input_method_is_wanted_only_while_a_text_entry_has_focus() {
        // Drives `set_ime_allowed`. Left on everywhere, a candidate window can appear over a menu; left
        // off, CJK text cannot be typed at all.
        let solved = solved();
        let mut ui = Interface::new();
        assert!(!ui.ime_wanted(&solved), "nothing focused");
        assert_eq!(ui.ime_cursor_area(&solved), None);
        ui.set_focus(Some("ok"));
        assert!(!ui.ime_wanted(&solved), "a button is not a text entry");
        assert_eq!(ui.ime_cursor_area(&solved), None);
        ui.set_focus(Some("name"));
        assert!(ui.ime_wanted(&solved));
        // The area is the field itself, which is where a candidate window belongs.
        let expected = solved.by_id("name").map(|node| node.rect);
        assert_eq!(ui.ime_cursor_area(&solved), expected);
        // A focus naming a control this layout does not have reports nothing rather than panicking.
        ui.set_focus(Some("absent"));
        assert!(!ui.ime_wanted(&solved));
        assert_eq!(ui.ime_cursor_area(&solved), None);
    }

    #[test]
    fn a_committed_reader_borrows_when_nothing_is_composing() {
        // The common case must not allocate, since a renderer reads this every frame.
        let field = TextField::new("abc");
        assert!(matches!(
            field.committed(),
            std::borrow::Cow::Borrowed("abc")
        ));
    }

    #[test]
    fn cancel_asks_to_leave_the_screen() {
        // Interpreted by the screen stack. Returning it as an action rather than popping anything keeps
        // this module free of navigation.
        let solved = solved();
        let mut ui = Interface::new();
        assert_eq!(ui.handle(&solved, UiEvent::Cancel), Some(Action::Back));
    }

    #[test]
    fn a_text_field_reports_its_own_shape() {
        let empty = TextField::default();
        assert!(empty.is_empty());
        assert_eq!(empty.len(), 0);
        assert_eq!(empty.cursor(), 0);
        // Cursor at the end, because that is where typing continues from.
        let seeded = TextField::new("hi");
        assert!(!seeded.is_empty());
        assert_eq!(seeded.cursor(), 2);
    }
}
