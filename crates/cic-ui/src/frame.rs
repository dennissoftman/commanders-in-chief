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
/// slider's track pieces, and each family's `W3DGadget*ImageDraw` reads its own slots. The retained
/// control knows its family; the frame carries it so the renderer does not have to reach back into
/// the layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UiControlFamily {
    /// `PUSHBUTTON`, whose art is authored as a left end, a repeating centre, and a right end.
    PushButton,
    /// `RADIOBUTTON`, three pieces at its own indices, with a selected button drawing from the
    /// hilite slot's second triple whatever its enablement.
    RadioButton,
    /// `CHECKBOX`, one square box image inset from the control's left edge.
    CheckBox,
    /// `ENTRYFIELD`, a horizontal four-piece frame: two ends, a repeating centre, and a narrower
    /// centre that fills the remaining seam.
    TextEntry,
    /// `VERTICALSLIDER`, the same four pieces stacked.
    VerticalSlider,
    /// `HORIZONTALSLIDER`, a row of repeating tick squares filled up to the current position.
    HorizontalSlider,
    /// `PROGRESSBAR`, a three-piece background with a repeating bar drawn inside it.
    ProgressBar,
    /// `TABCONTROL`, a stretched background plus one image per declared tab.
    TabControl(UiTabGeometry),
    /// Any other family — list boxes, combo boxes, static text, and plain windows — whose base
    /// visual is the slot's first image stretched across the control.
    #[default]
    Simple,
}

impl UiControlFamily {
    /// Returns a stable name for reports and diagnostics, without a tab control's geometry.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::PushButton => "PushButton",
            Self::RadioButton => "RadioButton",
            Self::CheckBox => "CheckBox",
            Self::TextEntry => "TextEntry",
            Self::VerticalSlider => "VerticalSlider",
            Self::HorizontalSlider => "HorizontalSlider",
            Self::ProgressBar => "ProgressBar",
            Self::TabControl(_) => "TabControl",
            Self::Simple => "Simple",
        }
    }
}

/// A tab control's declared tab strip, which its composition needs and no other family has.
///
/// The values come from `TABCONTROLDATA`; the derived tab origin reproduces
/// `GadgetTabControlComputeTabRegion`. No retail layout declares a tab control, so this rests on
/// source evidence alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UiTabGeometry {
    /// `TABORIENTATION`: `0` centre, `1` top left, `2` bottom right.
    pub orientation: i32,
    /// `TABEDGE`: `3` top, `4` right, `5` left, `6` bottom.
    pub edge: i32,
    /// One tab's width in creation-resolution pixels.
    pub width: i32,
    /// One tab's height in creation-resolution pixels.
    pub height: i32,
    /// How many tabs exist, at most [`UI_MAX_TABS`].
    pub count: usize,
    /// The inset the strip and its panes sit inside.
    pub pane_border: i32,
    /// Which tab is active, and so draws hilited.
    pub active: usize,
    /// Which tabs are disabled, indexed the same as `count`.
    pub disabled: [bool; UI_MAX_TABS],
}

/// How many tabs a `TABCONTROL` may declare.
///
/// `Gadget.h` sizes the pane array at `NUM_TAB_PANES = 8`.
pub const UI_MAX_TABS: usize = 8;

/// Live control state the per-family compositions read.
///
/// The source's draw procedures branch on the window's `WIN_STATUS_ENABLED` bit and the instance's
/// `WIN_STATE_HILITED`/`WIN_STATE_SELECTED` bits, then read family-specific user data. This carries
/// exactly that much, so a composition never needs the retained tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UiDrawState {
    /// Whether the control and every ancestor are enabled.
    pub enabled: bool,
    /// Whether the control draws hilited, which is hover or press on an enabled control.
    pub hilited: bool,
    /// The source's `WIN_STATE_SELECTED`: held down for a push button, checked for a check box,
    /// the group's choice for a radio button.
    pub selected: bool,
    /// A slider's position or a progress bar's percentage.
    pub value: i32,
    /// A slider's inclusive lower bound.
    pub minimum: i32,
    /// A slider's inclusive upper bound.
    pub maximum: i32,
}

/// Every mapped-image name a control declares, addressed the way the source addresses it.
///
/// A draw procedure picks both a slot and an entry index — a selected radio button reads the hilite
/// slot even while enabled, and a horizontal slider reads the disabled slot in every state — so a
/// frame that carried only the current state's slot could not express those. All three slots travel
/// together and the composition selects, exactly as `W3DGadget*ImageDraw` does.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UiSlotImages {
    enabled: Vec<Option<String>>,
    disabled: Vec<Option<String>>,
    hilite: Vec<Option<String>>,
}

impl UiSlotImages {
    /// Creates a table from one control's three slots, each in entry order.
    #[must_use]
    pub const fn new(
        enabled: Vec<Option<String>>,
        disabled: Vec<Option<String>>,
        hilite: Vec<Option<String>>,
    ) -> Self {
        Self {
            enabled,
            disabled,
            hilite,
        }
    }

    /// Returns one slot's image name at an entry index, absent when the entry declares none.
    #[must_use]
    pub fn image(&self, slot: WndDrawDataSlot, index: usize) -> Option<&str> {
        self.slot(slot).get(index)?.as_deref()
    }

    /// Returns one slot's image names in entry order.
    #[must_use]
    pub fn slot(&self, slot: WndDrawDataSlot) -> &[Option<String>] {
        match slot {
            WndDrawDataSlot::Disabled => &self.disabled,
            WndDrawDataSlot::Hilite => &self.hilite,
            _ => &self.enabled,
        }
    }

    /// Returns whether any slot declares any image.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        [&self.enabled, &self.disabled, &self.hilite]
            .into_iter()
            .all(|slot| slot.iter().all(Option::is_none))
    }

    /// Returns every declared image name, enabled slot first then disabled then hilite, each in
    /// entry order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        [&self.enabled, &self.disabled, &self.hilite]
            .into_iter()
            .flatten()
            .filter_map(Option::as_deref)
    }
}

/// How a text run is positioned inside its rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UiTextAlign {
    /// Placed at the rectangle's top left.
    #[default]
    TopLeft,
    /// Centred on both axes, which is what `drawButtonText` does for a push button.
    Centered,
    /// Vertically centred and indented from the left by the control's own height, which is where
    /// `drawCheckBoxText` puts a check box's label so it clears the box image.
    CenteredBesideBox,
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
        ///
        /// A composition still reads other slots where its source procedure does; this is the slot
        /// the plain state selection lands on, and the one the colours below come from.
        slot: WndDrawDataSlot,
        /// The slot's fill colour, absent when the control declares no draw data for the slot.
        color: Option<WndColor>,
        /// The slot's border colour, absent when the control declares no draw data for the slot.
        ///
        /// It applies only on the colour path: `W3DGameWinDefaultDraw` and every gadget's colour
        /// draw outline the control before filling it, while the matching `...ImageDraw` never
        /// does. `WIN_STATUS_BORDER` exists in `GameWindow.h` but no draw procedure reads it, so
        /// `image_draw` alone decides whether this colour is honoured.
        border_color: Option<WndColor>,
        /// Every image name the control declares, by slot and entry index, so a family's
        /// composition can read the slots its own draw procedure reads.
        images: UiSlotImages,
        /// The control's `IMAGEOFFSET`, which the source adds to every piece's position.
        image_offset: (i32, i32),
        /// Which family's composition rules apply.
        family: UiControlFamily,
        /// The live state that family's composition branches on.
        state: UiDrawState,
        /// Whether the control takes an image-drawing path at all.
        ///
        /// The discriminator is the retained draw-callback name — an `...ImageDraw` variant against
        /// a plain `...Draw` — because that name is what the original binds as the draw procedure.
        /// A control naming no draw callback falls back to its `IMAGE` status bit.
        image_draw: bool,
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
                color: entry.map(WndDrawEntry::color),
                border_color: entry.map(WndDrawEntry::border_color),
                images: control.slot_images(),
                image_offset: control.image_offset().unwrap_or((0, 0)),
                family: control.family(),
                state: UiDrawState {
                    enabled: self.is_effectively_enabled(id),
                    hilited: slot == WndDrawDataSlot::Hilite,
                    selected: control.is_selected(),
                    ..control.draw_bounds()
                },
                image_draw: control.is_image_draw(),
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
