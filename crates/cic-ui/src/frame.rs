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

/// Which established gadget family a quad belongs to.
///
/// A presentation layer needs this because the source composes a draw-data record's nine entries
/// per family: a push button's ends and repeating centre come from different indices than a
/// slider's track pieces. The retained control knows its family; the frame carries it so the
/// renderer does not have to reach back into the layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UiControlFamily {
    /// `PUSHBUTTON`, whose art is authored as a left end, a repeating centre, and a right end.
    PushButton,
    /// Any other family, whose base visual is one image stretched across the control.
    #[default]
    Simple,
}

/// How a text run is positioned inside its rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UiTextAlign {
    /// Placed at the rectangle's top left.
    #[default]
    TopLeft,
    /// Centred on both axes, which is what `drawButtonText` does for a push button.
    Centered,
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
    /// Where the run sits inside its rectangle.
    pub align: UiTextAlign,
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
        /// Whether the control declares `BORDER`, which is what makes the original draw its border
        /// and corners at all. A colour alone does not.
        border: bool,
        /// Every image name in the selected slot, in entry order, so a presentation layer can
        /// compose the pieces its family declares. Entry 0 is the whole-control background.
        entries: Vec<Option<String>>,
        /// The control's `IMAGEOFFSET`, which the source adds to every piece's position.
        image_offset: (i32, i32),
        /// Which family's composition rules apply.
        family: UiControlFamily,
        /// Whether the control is currently held down, which selects the pushed art.
        selected: bool,
        /// Whether the control declares `IMAGE`, which is what makes the original take an
        /// image-drawing path at all instead of filling with the slot's colour.
        image_status: bool,
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
                border: control.status().contains(UiStatus::BORDER),
                entries: control.draw_entry_images(slot),
                image_offset: control.image_offset().unwrap_or((0, 0)),
                family: control.family(),
                selected: control.is_pressed(),
                image_status: control.status().contains(UiStatus::IMAGE),
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
            align: control.text_align(),
        })
    }
}

impl UiFrame {
    /// Returns a copy of this frame with every text run's label replaced by `resolve`'s result.
    ///
    /// A `TEXT` record holds either a localization label or a literal string, and the retained model
    /// deliberately does not decide which. This lets a caller that owns a string table substitute
    /// localized text without the runtime depending on localization: returning `None` keeps the
    /// original value, which is the correct behavior for both a literal and an unresolved label.
    #[must_use]
    pub fn with_resolved_text(&self, resolve: &dyn Fn(&str) -> Option<String>) -> Self {
        Self {
            items: self
                .items
                .iter()
                .map(|item| match item {
                    UiFrameItem::Text(run) => UiFrameItem::Text(UiTextRun {
                        label: resolve(&run.label).unwrap_or_else(|| run.label.clone()),
                        ..run.clone()
                    }),
                    other => other.clone(),
                })
                .collect(),
        }
    }
}
