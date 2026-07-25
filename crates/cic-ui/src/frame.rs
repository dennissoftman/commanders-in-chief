// Commanders in Chief
// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only
//
// Provenance: the draw layering (`BELOW`, then unlayered, then `ABOVE`), the parent-before-child
// submission order, and the enabled/disabled/hilite state selection follow Electronic Arts' GPL-3.0
// source release, GeneralsGameCode revision 9f7abb866f5afd446db14149979e744c7216baaf, specifically
// `GeneralsMD/Code/GameEngine/Source/GameClient/GUI/GameWindowManager.cpp`
// (`getWindowUnderCursor`'s layer order, which the draw order inverts) and the per-gadget draw-data
// slots in `Core/GameEngine/Include/GameClient/Gadget.h`. The frame representation, the clip policy,
// and the renderer-neutral item vocabulary are project design.

use cic_formats::{WndColor, WndDrawDataSlot, WndDrawEntry};

use crate::retained::{UiControlId, UiLayout, UiRect, UiStatus};

/// Whether a frame confines a control's children to its own rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UiClipPolicy {
    /// No clipping, which is how the original renders: a child may overflow its parent, and retail
    /// layouts contain children that do.
    #[default]
    None,
    /// Clip each control's subtree to that control's rectangle. Project design, for callers that
    /// want a scrolling or masked region.
    ClipToParent,
}

/// One shaped text run a frame asks the renderer to draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiTextRun {
    /// The control the run belongs to.
    pub control: UiControlId,
    /// The rectangle the run is laid out in, in viewport pixels.
    pub rect: UiRect,
    /// The `TEXT` record's value, which the caller resolves as a label or literal.
    pub label: String,
    /// The declared font family, point size, and weight, when the control declares one.
    pub font: Option<(String, i32, bool)>,
    /// The state colour for the run, absent when the control declares no `TEXTCOLOR` record.
    pub color: Option<WndColor>,
    /// Whether the run renders masked, for a secret text entry.
    pub masked: bool,
}

/// One renderer-neutral draw instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiFrameItem {
    /// Begin confining subsequent items to a rectangle.
    PushClip {
        /// The clip rectangle in viewport pixels.
        rect: UiRect,
    },
    /// End the innermost clip.
    PopClip,
    /// A control's background quad.
    Quad {
        /// The control.
        control: UiControlId,
        /// The absolute rectangle.
        rect: UiRect,
        /// Which draw-data slot the control's current state selected.
        slot: WndDrawDataSlot,
        /// The mapped-image name for the slot's first entry, absent when it declares `NoImage`
        /// or the control declares no draw data for the slot.
        image: Option<String>,
        /// The slot's fill colour, absent when the control declares no draw data for the slot.
        color: Option<WndColor>,
        /// The slot's border colour, absent when the control declares no draw data for the slot.
        border_color: Option<WndColor>,
    },
    /// A control's text.
    Text(UiTextRun),
}

/// One frame of renderer-neutral UI draw instructions, in stable submission order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UiFrame {
    items: Vec<UiFrameItem>,
}

impl UiFrame {
    /// Returns the instructions in submission order.
    #[must_use]
    pub fn items(&self) -> &[UiFrameItem] {
        &self.items
    }

    /// Returns how many instructions the frame holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Returns whether the frame is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl UiLayout {
    /// Builds one renderer-neutral frame from current retained state.
    ///
    /// Submission order is the inverse of the hit-test layering — `BELOW` roots first, then
    /// unlayered roots, then `ABOVE` roots — with each subtree emitted parent before children in
    /// source order, so a child draws over its parent and a later sibling draws over an earlier one.
    /// A hidden control and its whole subtree are skipped; a control declaring `SEE_THRU` emits no
    /// quad of its own but still emits its children, which is what that flag means.
    #[must_use]
    pub fn frame(&self, clip: UiClipPolicy) -> UiFrame {
        let mut frame = UiFrame::default();
        let layers: [fn(UiStatus) -> bool; 3] = [
            |status| status.contains(UiStatus::BELOW),
            |status| !status.intersects(UiStatus::ABOVE) && !status.intersects(UiStatus::BELOW),
            |status| status.contains(UiStatus::ABOVE),
        ];
        for accepts in layers {
            for root in self.roots() {
                if accepts(self.control(*root).status()) {
                    self.emit(*root, clip, &mut frame);
                }
            }
        }
        frame
    }

    fn emit(&self, id: UiControlId, clip: UiClipPolicy, frame: &mut UiFrame) {
        let control = self.control(id);
        if control.is_hidden() {
            return;
        }
        let rect = self.screen_rect(id);
        if !control.status().contains(UiStatus::SEE_THRU) {
            let slot = self.state_slot(id);
            let entry = control.draw_entry(slot);
            frame.items.push(UiFrameItem::Quad {
                control: id,
                rect,
                slot,
                image: entry.and_then(|entry| entry.image()).map(str::to_owned),
                color: entry.map(WndDrawEntry::color),
                border_color: entry.map(WndDrawEntry::border_color),
            });
            if let Some(run) = self.text_run(id, rect) {
                frame.items.push(UiFrameItem::Text(run));
            }
        }
        if control.children().is_empty() {
            return;
        }
        let clipped = clip == UiClipPolicy::ClipToParent;
        if clipped {
            frame.items.push(UiFrameItem::PushClip { rect });
        }
        for child in control.children() {
            self.emit(*child, clip, frame);
        }
        if clipped {
            frame.items.push(UiFrameItem::PopClip);
        }
    }

    /// Returns the draw-data slot a control's current state selects.
    ///
    /// Disabled wins over hover, and a pressed or hovered enabled control draws hilited, which is
    /// the three-state model every gadget's draw data declares.
    #[must_use]
    pub fn state_slot(&self, id: UiControlId) -> WndDrawDataSlot {
        let control = self.control(id);
        if !self.is_effectively_enabled(id) {
            WndDrawDataSlot::Disabled
        } else if control.is_pressed() || control.is_hovered() {
            WndDrawDataSlot::Hilite
        } else {
            WndDrawDataSlot::Enabled
        }
    }

    fn text_run(&self, id: UiControlId, rect: UiRect) -> Option<UiTextRun> {
        let control = self.control(id);
        let masked = control.is_secret_text();
        let label = control
            .displayed_text()
            .filter(|text| !text.is_empty())?
            .to_owned();
        Some(UiTextRun {
            control: id,
            rect,
            label,
            font: control
                .font()
                .map(|(name, size, bold)| (name.to_owned(), size, bold)),
            color: control.text_color(self.state_slot(id)),
            masked,
        })
    }
}
