// Commanders in Chief
// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only
//
// Provenance: the hit-testing layer order, the child descent rule, the inclusive edge test, the
// focus refusal and parent walk, and the wraparound tab cycle are derived from Electronic Arts'
// GPL-3.0 source release, GeneralsGameCode revision 9f7abb866f5afd446db14149979e744c7216baaf,
// specifically `GeneralsMD/Code/GameEngine/Source/GameClient/GUI/GameWindowManager.cpp`
// (`GameWindowManager::getWindowUnderCursor`, `winSetFocus`, `winNextTab`, `winPrevTab`) and
// `Core/GameEngine/Source/GameClient/GUI/GameWindow.cpp` (`GameWindow::winPointInChild`,
// `winPointInWindow`). Typed events and the text-editing model are project design.

use crate::retained::{UiControl, UiControlId, UiControlKind, UiLayout, UiPoint, UiStatus};

/// A mouse button the UI distinguishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiMouseButton {
    /// Primary button.
    Left,
    /// Secondary button. Only controls declaring `RIGHT_CLICK` react to it.
    Right,
}

/// A key the retained runtime interprets.
///
/// Text insertion is a separate operation, so this covers only keys with structural meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiKey {
    /// Move focus to the next tab stop.
    Tab,
    /// Move focus to the previous tab stop.
    ShiftTab,
    /// Delete the character before the caret.
    Backspace,
    /// Delete the character at the caret.
    Delete,
    /// Move the caret one character left.
    Left,
    /// Move the caret one character right.
    Right,
    /// Move the caret to the start.
    Home,
    /// Move the caret to the end.
    End,
    /// Move a list or combo selection up one, or a slider down one step.
    Up,
    /// Move a list or combo selection down one, or a slider up one step.
    Down,
    /// Activate the focused control.
    Enter,
    /// Dismiss an open drop-down.
    Escape,
}

/// A typed UI state change.
///
/// Events carry the source callback name as data. Routing an event to an application action is the
/// caller's allowlisted decision; nothing here dispatches a name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiEvent {
    /// The pointer entered a control.
    HoverEntered {
        /// The control.
        control: UiControlId,
    },
    /// The pointer left a control.
    HoverLeft {
        /// The control.
        control: UiControlId,
    },
    /// A control was pressed.
    Pressed {
        /// The control.
        control: UiControlId,
        /// Which button.
        button: UiMouseButton,
    },
    /// A press completed over the same control that received it.
    Activated {
        /// The control.
        control: UiControlId,
        /// Which button.
        button: UiMouseButton,
        /// The control's retained system callback name, when it declares one.
        callback: Option<String>,
    },
    /// A press was released away from the control that received it.
    PressCancelled {
        /// The control.
        control: UiControlId,
    },
    /// Keyboard focus moved.
    FocusChanged {
        /// The control that lost focus.
        from: Option<UiControlId>,
        /// The control that gained focus.
        to: Option<UiControlId>,
    },
    /// A check box or radio button changed state.
    ToggleChanged {
        /// The control.
        control: UiControlId,
        /// The new state.
        selected: bool,
    },
    /// A slider moved.
    ValueChanged {
        /// The control.
        control: UiControlId,
        /// The applied value.
        value: i32,
    },
    /// A list or combo selection changed.
    SelectionChanged {
        /// The control.
        control: UiControlId,
        /// The selected index, absent when the selection was cleared.
        index: Option<usize>,
    },
    /// A text entry's contents changed.
    TextChanged {
        /// The control.
        control: UiControlId,
    },
}

impl UiLayout {
    /// Returns the control the pointer is over, reproducing the original's layered search.
    ///
    /// The original searches top-level windows in three passes — `ABOVE` first, then windows with
    /// none of `ABOVE`/`BELOW`/`HIDDEN`, then `BELOW` — and descends into the first matching child,
    /// skipping a hidden or disabled child and continuing with the next in list order. A control
    /// declaring `NO_INPUT` discards the result entirely. While a control holds the mouse, the search
    /// is confined to it. Every edge test is inclusive.
    ///
    /// Both searches run in reverse source order, because that is the order the original's window
    /// manager holds. `winCreate` links every new window at the head of its list — the top-level list
    /// through `linkWindow`, a child list through `addWindowToParent`, whose append-at-end variant is
    /// commented out — so a layout's list is the reverse of its file order, and `winBringToTop` pulls
    /// from the layout tail specifically to preserve that. `getWindowUnderCursor` and
    /// `winPointInChild` then walk from the head, so the last window in the file is tested first.
    /// This is also the front-most window: `winRepaint` draws from the tail backwards, which puts the
    /// last window in the file on top.
    #[must_use]
    pub fn hit_test(&self, point: UiPoint) -> Option<UiControlId> {
        if let Some(capture) = self.capture() {
            return self.filter_input(self.descend(capture, point));
        }
        let passes: [fn(UiStatus) -> bool; 3] = [
            |status| status.contains(UiStatus::ABOVE),
            |status| !status.intersects(UiStatus::ABOVE) && !status.intersects(UiStatus::BELOW),
            |status| status.contains(UiStatus::BELOW),
        ];
        for accepts in passes {
            if let Some(control) = self.hit_test_pass(point, accepts) {
                return Some(control);
            }
        }
        None
    }

    /// Runs one pass of the layered search over this layout's top-level controls.
    ///
    /// The shell owns the three-pass loop when more than one layout is up, because the original's
    /// passes run over one global window list spanning every layout rather than per layout.
    pub(crate) fn hit_test_pass(
        &self,
        point: UiPoint,
        accepts: fn(UiStatus) -> bool,
    ) -> Option<UiControlId> {
        for root in self.roots().iter().rev() {
            let control = self.control(*root);
            if control.is_hidden() || !accepts(control.status()) {
                continue;
            }
            if !self.screen_rect(*root).contains(point) || !control.is_enabled() {
                continue;
            }
            return self.filter_input(self.descend(*root, point));
        }
        None
    }

    /// Descends into the first visible, enabled child containing the point, or returns `parent`.
    fn descend(&self, parent: UiControlId, point: UiPoint) -> UiControlId {
        for child in self.control(parent).children().iter().rev() {
            let control = self.control(*child);
            if control.is_hidden() || !self.screen_rect(*child).contains(point) {
                continue;
            }
            if control.is_enabled() {
                return self.descend(*child, point);
            }
            // A disabled child that contains the point is skipped, and iteration continues with the
            // next child in list order, exactly as the source does.
        }
        parent
    }

    fn filter_input(&self, id: UiControlId) -> Option<UiControlId> {
        let control = self.control(id);
        if control.status().contains(UiStatus::NO_INPUT) {
            return None;
        }
        Some(id)
    }

    /// Moves the pointer, updating hover state and emitting the resulting transitions.
    pub fn pointer_moved(&mut self, point: UiPoint) -> Vec<UiEvent> {
        let target = self.hit_test(point);
        let mut events = Vec::new();
        let previously: Vec<UiControlId> = self
            .controls()
            .iter()
            .filter(|control| control.is_hovered())
            .map(UiControl::id)
            .collect();
        for control in previously {
            if Some(control) != target {
                self.control_mut(control).set_hovered(false);
                events.push(UiEvent::HoverLeft { control });
            }
        }
        if let Some(control) = target
            && !self.control(control).is_hovered()
        {
            self.control_mut(control).set_hovered(true);
            events.push(UiEvent::HoverEntered { control });
        }
        events
    }

    /// Presses at a point, taking mouse capture and moving focus where the control accepts it.
    ///
    /// A control declaring `ON_MOUSE_DOWN` activates immediately on press, which is how the
    /// original distinguishes those push buttons; every other control activates on release.
    pub fn pointer_pressed(&mut self, point: UiPoint, button: UiMouseButton) -> Vec<UiEvent> {
        let mut events = Vec::new();
        let Some(target) = self.hit_test(point) else {
            return events;
        };
        if button == UiMouseButton::Right
            && !self
                .control(target)
                .status()
                .contains(UiStatus::RIGHT_CLICK)
        {
            return events;
        }
        self.control_mut(target).set_pressed(true);
        self.set_capture(Some(target));
        events.push(UiEvent::Pressed {
            control: target,
            button,
        });
        events.extend(self.set_focus(Some(target)));
        if self
            .control(target)
            .status()
            .contains(UiStatus::ON_MOUSE_DOWN)
        {
            events.extend(self.activate(target, button));
        }
        events
    }

    /// Releases at a point, activating the pressed control only when the release lands on it.
    ///
    /// While a control holds the mouse the original's cursor search is confined to that control and
    /// returns it for any point, so the release test is the control's own rectangle rather than a
    /// fresh search — which is also what each gadget's input handler checks before acting.
    pub fn pointer_released(&mut self, point: UiPoint, button: UiMouseButton) -> Vec<UiEvent> {
        let mut events = Vec::new();
        let Some(pressed) = self.capture() else {
            return events;
        };
        let released_inside = self.screen_rect(pressed).contains(point);
        self.control_mut(pressed).set_pressed(false);
        self.set_capture(None);
        if released_inside {
            if self
                .control(pressed)
                .status()
                .contains(UiStatus::ON_MOUSE_DOWN)
            {
                // Already activated on press; releasing must not activate twice.
                return events;
            }
            events.extend(self.activate(pressed, button));
        } else {
            events.push(UiEvent::PressCancelled { control: pressed });
        }
        events
    }

    /// Applies a control's activation invariants and returns the resulting events.
    fn activate(&mut self, control: UiControlId, button: UiMouseButton) -> Vec<UiEvent> {
        let mut events = Vec::new();
        // The kind is inspected first and released, so the invariant methods below can borrow the
        // layout mutably.
        let activation = match self.control(control).kind() {
            UiControlKind::CheckBox { .. } => Activation::Toggle,
            UiControlKind::RadioButton { .. } => Activation::Radio,
            UiControlKind::ComboBox { open, .. } => Activation::Combo { open: *open },
            _ => Activation::None,
        };
        match activation {
            Activation::Toggle => {
                if let Some(checked) = self.toggle_check(control) {
                    events.push(UiEvent::ToggleChanged {
                        control,
                        selected: checked,
                    });
                }
            }
            Activation::Radio => {
                self.select_radio(control);
                events.push(UiEvent::ToggleChanged {
                    control,
                    selected: true,
                });
            }
            Activation::Combo { open } => {
                self.set_combo_open(control, !open);
            }
            Activation::None => {}
        }
        let callback = self.control(control).system_callback().map(str::to_owned);
        events.push(UiEvent::Activated {
            control,
            button,
            callback,
        });
        events
    }

    /// Sets keyboard focus, honoring the original's refusal rules.
    ///
    /// A control declaring `NOFOCUS` refuses focus outright. A control that cannot hold focus —
    /// anything that is not a text entry, list, combo, slider, or button family — passes the request
    /// to its parent, and focus becomes `None` when no ancestor accepts it, which is the shape of
    /// the original's `GWM_INPUT_FOCUS` walk.
    pub fn set_focus(&mut self, requested: Option<UiControlId>) -> Vec<UiEvent> {
        let previous = self.focus();
        let mut candidate = requested;
        let mut accepted = None;
        while let Some(control) = candidate {
            if self.control(control).status().contains(UiStatus::NO_FOCUS) {
                return Vec::new();
            }
            if self.accepts_focus(control) && self.is_effectively_enabled(control) {
                accepted = Some(control);
                break;
            }
            candidate = self.control(control).parent();
        }
        if accepted == previous {
            return Vec::new();
        }
        self.set_focus_field(accepted);
        vec![UiEvent::FocusChanged {
            from: previous,
            to: accepted,
        }]
    }

    fn accepts_focus(&self, control: UiControlId) -> bool {
        matches!(
            self.control(control).kind(),
            UiControlKind::TextEntry { .. }
                | UiControlKind::ListBox { .. }
                | UiControlKind::ComboBox { .. }
                | UiControlKind::Slider { .. }
                | UiControlKind::PushButton
                | UiControlKind::RadioButton { .. }
                | UiControlKind::CheckBox { .. }
        )
    }

    /// Moves focus to the next tab stop, wrapping around.
    pub fn focus_next(&mut self) -> Vec<UiEvent> {
        self.focus_step(true)
    }

    /// Moves focus to the previous tab stop, wrapping around.
    pub fn focus_previous(&mut self) -> Vec<UiEvent> {
        self.focus_step(false)
    }

    fn focus_step(&mut self, forward: bool) -> Vec<UiEvent> {
        let order: Vec<UiControlId> = self
            .tab_order()
            .iter()
            .copied()
            .filter(|control| self.is_effectively_enabled(*control))
            .collect();
        if order.is_empty() {
            return Vec::new();
        }
        let current = self
            .focus()
            .and_then(|focus| order.iter().position(|control| *control == focus));
        let next = match (current, forward) {
            (Some(index), true) => (index + 1) % order.len(),
            (Some(index), false) => (index + order.len() - 1) % order.len(),
            (None, true) => 0,
            (None, false) => order.len() - 1,
        };
        self.set_focus(Some(order[next]))
    }

    /// Inserts text into the focused control, bounded by its declared maximum length.
    ///
    /// Characters are counted, not bytes, so a Unicode entry field holds the number of characters
    /// its definition declares.
    pub fn insert_text(&mut self, text: &str) -> Vec<UiEvent> {
        let Some(control) = self.focus() else {
            return Vec::new();
        };
        let limit = self.limits().max_text_length;
        let UiControlKind::TextEntry {
            text: contents,
            caret,
            max_length,
            ..
        } = self.control_mut(control).kind_mut()
        else {
            return Vec::new();
        };
        let effective = if *max_length == 0 { limit } else { *max_length };
        let mut inserted = false;
        for character in text.chars() {
            if contents.chars().count() >= effective {
                break;
            }
            let byte = byte_offset(contents, *caret);
            contents.insert(byte, character);
            *caret += 1;
            inserted = true;
        }
        if inserted {
            vec![UiEvent::TextChanged { control }]
        } else {
            Vec::new()
        }
    }

    /// Applies one structural key to the focused control.
    pub fn press_key(&mut self, key: UiKey) -> Vec<UiEvent> {
        match key {
            UiKey::Tab => return self.focus_next(),
            UiKey::ShiftTab => return self.focus_previous(),
            _ => {}
        }
        let Some(control) = self.focus() else {
            return Vec::new();
        };
        match self.control(control).kind() {
            UiControlKind::TextEntry { .. } => self.text_entry_key(control, key),
            UiControlKind::Slider { .. } => self.slider_key(control, key),
            UiControlKind::ListBox { .. } => self.list_key(control, key),
            UiControlKind::ComboBox { .. } => self.combo_key(control, key),
            _ => self.button_key(control, key),
        }
    }

    fn text_entry_key(&mut self, control: UiControlId, key: UiKey) -> Vec<UiEvent> {
        let UiControlKind::TextEntry { text, caret, .. } = self.control_mut(control).kind_mut()
        else {
            return Vec::new();
        };
        let length = text.chars().count();
        let mut changed = false;
        match key {
            UiKey::Backspace if *caret > 0 => {
                let byte = byte_offset(text, *caret - 1);
                text.remove(byte);
                *caret -= 1;
                changed = true;
            }
            UiKey::Delete if *caret < length => {
                let byte = byte_offset(text, *caret);
                text.remove(byte);
                changed = true;
            }
            UiKey::Left => *caret = caret.saturating_sub(1),
            UiKey::Right => *caret = (*caret + 1).min(length),
            UiKey::Home => *caret = 0,
            UiKey::End => *caret = length,
            _ => {}
        }
        if changed {
            vec![UiEvent::TextChanged { control }]
        } else {
            Vec::new()
        }
    }

    fn slider_key(&mut self, control: UiControlId, key: UiKey) -> Vec<UiEvent> {
        let Some(previous) = self.slider_value(control) else {
            return Vec::new();
        };
        let requested = match key {
            UiKey::Up | UiKey::Right => previous + 1,
            UiKey::Down | UiKey::Left => previous - 1,
            _ => return Vec::new(),
        };
        match self.set_slider_value(control, requested) {
            Some(applied) if applied != previous => vec![UiEvent::ValueChanged {
                control,
                value: applied,
            }],
            _ => Vec::new(),
        }
    }

    fn slider_value(&self, control: UiControlId) -> Option<i32> {
        match self.control(control).kind() {
            UiControlKind::Slider { value, .. } => Some(*value),
            _ => None,
        }
    }

    fn list_key(&mut self, control: UiControlId, key: UiKey) -> Vec<UiEvent> {
        let Some((count, current)) = self.list_state(control) else {
            return Vec::new();
        };
        if count == 0 {
            return Vec::new();
        }
        let target = match (key, current) {
            (UiKey::Down, Some(index)) => (index + 1).min(count - 1),
            (UiKey::Down, None) => 0,
            (UiKey::Up, Some(index)) => index.saturating_sub(1),
            (UiKey::Up, None) => count - 1,
            _ => return Vec::new(),
        };
        if current == Some(target) {
            return Vec::new();
        }
        if self.select_list_row(control, target, false) {
            vec![UiEvent::SelectionChanged {
                control,
                index: Some(target),
            }]
        } else {
            Vec::new()
        }
    }

    fn combo_key(&mut self, control: UiControlId, key: UiKey) -> Vec<UiEvent> {
        let Some((count, current)) = self.combo_state(control) else {
            return Vec::new();
        };
        if key == UiKey::Escape {
            self.set_combo_open(control, false);
            return Vec::new();
        }
        if count == 0 {
            return Vec::new();
        }
        let target = match (key, current) {
            (UiKey::Down, Some(index)) => (index + 1).min(count - 1),
            (UiKey::Down, None) => 0,
            (UiKey::Up, Some(index)) => index.saturating_sub(1),
            (UiKey::Up, None) => count - 1,
            _ => return Vec::new(),
        };
        if current == Some(target) {
            return Vec::new();
        }
        if self.select_combo_entry(control, target) {
            vec![UiEvent::SelectionChanged {
                control,
                index: Some(target),
            }]
        } else {
            Vec::new()
        }
    }

    fn button_key(&mut self, control: UiControlId, key: UiKey) -> Vec<UiEvent> {
        if key == UiKey::Enter {
            self.activate(control, UiMouseButton::Left)
        } else {
            Vec::new()
        }
    }
}

/// What a control does when activated, extracted before the layout is borrowed mutably.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Activation {
    Toggle,
    Radio,
    Combo { open: bool },
    None,
}

impl UiLayout {
    fn list_state(&self, control: UiControlId) -> Option<(usize, Option<usize>)> {
        match self.control(control).kind() {
            UiControlKind::ListBox { rows, selected, .. } => {
                Some((rows.len(), selected.last().copied()))
            }
            _ => None,
        }
    }

    fn combo_state(&self, control: UiControlId) -> Option<(usize, Option<usize>)> {
        match self.control(control).kind() {
            UiControlKind::ComboBox {
                entries, selected, ..
            } => Some((entries.len(), *selected)),
            _ => None,
        }
    }
}

/// Returns the byte offset of a character index, clamped to the string's length.
fn byte_offset(text: &str, character: usize) -> usize {
    text.char_indices()
        .nth(character)
        .map_or(text.len(), |(offset, _)| offset)
}
