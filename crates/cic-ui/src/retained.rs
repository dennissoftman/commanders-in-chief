// Commanders in Chief
// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only
//
// Provenance: the retained control model, the creation-resolution scaling, the parent-relative
// coordinate rule, the status vocabulary, and the gadget invariants are derived from Electronic
// Arts' GPL-3.0 source release, GeneralsGameCode revision
// 9f7abb866f5afd446db14149979e744c7216baaf, specifically
// `GeneralsMD/Code/GameEngine/Source/GameClient/GUI/GameWindowManagerScript.cpp` (`parseScreenRect`,
// `winCreateFromScript`), `Core/GameEngine/Source/GameClient/GUI/GameWindow.cpp`
// (`GameWindow::winGetScreenPosition`, `winPointInWindow`, `winPointInChild`, `winSetEnabled`,
// `winHide`), `Core/GameEngine/Include/GameClient/GameWindow.h` (the `WIN_STATUS_*` vocabulary), and
// `Core/GameEngine/Source/GameClient/GUI/Gadget/GadgetRadioButton.cpp` (group exclusivity).
// The arena representation, typed events, diagnostics, limits, and the Modern scale policy are
// project design.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use cic_formats::{
    WND_DRAW_DATA_ENTRIES, WndColor, WndComboBoxData, WndDocument, WndDrawData, WndDrawDataSlot,
    WndDrawEntry, WndGadgetData, WndListBoxData, WndSliderData, WndTabControlData, WndTextColors,
    WndTextEntryData, WndWindow,
};

use crate::frame::{
    UI_MAX_TABS, UiControlFamily, UiDrawState, UiSlotImages, UiTabGeometry, UiTextAlign,
};

/// Explicit bounds for instantiating a retained layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiLimits {
    /// Maximum retained controls in one layout.
    pub max_controls: usize,
    /// Maximum nesting depth.
    pub max_depth: usize,
    /// Maximum characters a text-entry control may hold when its definition declares no limit.
    pub max_text_length: usize,
    /// Maximum rows a list box may hold.
    pub max_list_rows: usize,
}

impl Default for UiLimits {
    fn default() -> Self {
        Self {
            max_controls: 4_096,
            max_depth: 64,
            max_text_length: 1_024,
            max_list_rows: 4_096,
        }
    }
}

/// What a synthesised gadget child is, for a control the definition never declared.
///
/// The original's gadget-creation functions build these while creating the gadget itself, so they
/// are as real as any declared control: they hit test, they take focus, and they draw from the
/// parent's child-specific draw-data records. Nothing here is invented — see
/// [`UiLayout::synthesise_children`] for the source of each one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UiGadgetRole {
    /// A slider's draggable thumb, from `gogoGadgetSlider`.
    SliderThumb,
    /// A scroll list box's up button, from `GadgetListboxCreateScrollbar`.
    ListBoxUpButton,
    /// A scroll list box's down button, from `GadgetListboxCreateScrollbar`.
    ListBoxDownButton,
    /// A scroll list box's vertical slider, from `GadgetListboxCreateScrollbar`.
    ListBoxSlider,
    /// A combo box's drop-down button, from `gogoGadgetComboBox`.
    ComboBoxDropDownButton,
    /// A combo box's edit field, from `gogoGadgetComboBox`.
    ComboBoxEditBox,
    /// A combo box's drop-down list, from `gogoGadgetComboBox`.
    ComboBoxListBox,
}

impl UiGadgetRole {
    /// Returns the stable report name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::SliderThumb => "slider_thumb",
            Self::ListBoxUpButton => "list_box_up_button",
            Self::ListBoxDownButton => "list_box_down_button",
            Self::ListBoxSlider => "list_box_slider",
            Self::ComboBoxDropDownButton => "combo_box_drop_down_button",
            Self::ComboBoxEditBox => "combo_box_edit_box",
            Self::ComboBoxListBox => "combo_box_list_box",
        }
    }

    /// Returns the suffix appended to the parent's name to name this child.
    ///
    /// The original leaves these windows unnamed and reaches them through `ListboxData` and
    /// `ComboBoxData` pointers. A name is this project's own addition, so a report can identify one
    /// and a patch overlay cannot silently collide with a declared control.
    const fn name_suffix(self) -> &'static str {
        match self {
            Self::SliderThumb => "<thumb>",
            Self::ListBoxUpButton => "<upbutton>",
            Self::ListBoxDownButton => "<downbutton>",
            Self::ListBoxSlider => "<slider>",
            Self::ComboBoxDropDownButton => "<dropdownbutton>",
            Self::ComboBoxEditBox => "<editbox>",
            Self::ComboBoxListBox => "<listbox>",
        }
    }
}

/// The button width `GadgetListboxCreateScrollbar` and `gogoGadgetComboBox` both hardcode.
///
/// Every size in this group is a literal applied to an already-scaled parent rectangle, so a scroll
/// button stays 21 pixels wide at every resolution while the gadget around it grows. That is source
/// behaviour, not an oversight in this port.
const GADGET_BUTTON_WIDTH: i32 = 21;

/// The button height `GadgetListboxCreateScrollbar` hardcodes beside [`GADGET_BUTTON_WIDTH`].
///
/// `gogoGadgetComboBox` declares the same local and then never uses it, giving its drop-down button
/// the combo box's full height instead.
const GADGET_BUTTON_HEIGHT: i32 = 22;

/// `Gadget.h`'s `HORIZONTAL_SLIDER_THUMB_WIDTH`.
const HORIZONTAL_SLIDER_THUMB_WIDTH: i32 = 13;

/// `Gadget.h`'s `HORIZONTAL_SLIDER_THUMB_HEIGHT`.
const HORIZONTAL_SLIDER_THUMB_HEIGHT: i32 = 16;

/// `GadgetSlider.h`'s `HORIZONTAL_SLIDER_THUMB_POSITION`, which is two thirds of the thumb height
/// under integer division.
const HORIZONTAL_SLIDER_THUMB_POSITION: i32 = HORIZONTAL_SLIDER_THUMB_HEIGHT * 2 / 3;

/// The presentation viewport a layout is instantiated for, in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiViewport {
    width: i32,
    height: i32,
}

impl UiViewport {
    /// Creates a viewport, rejecting a non-positive extent.
    ///
    /// # Errors
    ///
    /// Returns [`UiLayoutError::InvalidViewport`] when either extent is not positive.
    pub const fn new(width: i32, height: i32) -> Result<Self, UiLayoutError> {
        if width <= 0 || height <= 0 {
            return Err(UiLayoutError::InvalidViewport { width, height });
        }
        Ok(Self { width, height })
    }

    /// Returns the viewport width in pixels.
    #[must_use]
    pub const fn width(self) -> i32 {
        self.width
    }

    /// Returns the viewport height in pixels.
    #[must_use]
    pub const fn height(self) -> i32 {
        self.height
    }
}

/// How stored creation-resolution coordinates map onto the presentation viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UiScalePolicy {
    /// The original policy, reproduced exactly.
    ///
    /// `parseScreenRect` scales each stored corner independently by
    /// `viewport / creation_resolution` per axis and truncates toward zero, then derives size from
    /// the scaled corners. Non-uniform aspect ratios therefore stretch, and rounding is not
    /// symmetric — both are visible in the original at any non-4:3 resolution.
    #[default]
    Classic,
    /// A uniform-scale, centered policy that preserves the authored aspect ratio.
    ///
    /// The smaller axis ratio is applied to both axes and the scaled layout is centered in the
    /// viewport, so a widescreen viewport letterboxes the authored composition instead of
    /// stretching it. This is project design, not source behavior; it exists because the original
    /// stretch is the single most visible artifact of running a 4:3 layout on a modern display.
    Modern,
}

/// The explicit presentation inputs a retained layout is built against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiPresentation {
    /// The target viewport.
    pub viewport: UiViewport,
    /// How stored coordinates map onto it.
    pub scale: UiScalePolicy,
}

impl UiPresentation {
    /// Creates a presentation for one viewport and policy.
    #[must_use]
    pub const fn new(viewport: UiViewport, scale: UiScalePolicy) -> Self {
        Self { viewport, scale }
    }
}

/// A point in viewport pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiPoint {
    /// Horizontal position.
    pub x: i32,
    /// Vertical position.
    pub y: i32,
}

impl UiPoint {
    /// Creates a point.
    #[must_use]
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// A rectangle in viewport pixels. `x` and `y` are parent-relative for a child control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiRect {
    /// Left edge, relative to the parent's left edge.
    pub x: i32,
    /// Top edge, relative to the parent's top edge.
    pub y: i32,
    /// Width in pixels.
    pub width: i32,
    /// Height in pixels.
    pub height: i32,
}

impl UiRect {
    /// Returns whether a point lies inside, with both edges inclusive.
    ///
    /// `GameWindow::winPointInWindow` tests `x >= left && x <= left + width`, so a point exactly on
    /// the right or bottom edge is inside. Reproduced because a one-pixel hit-test difference
    /// changes which control a click reaches on adjacent controls.
    #[must_use]
    pub const fn contains(self, point: UiPoint) -> bool {
        point.x >= self.x
            && point.x <= self.x + self.width
            && point.y >= self.y
            && point.y <= self.y + self.height
    }
}

/// The established window status vocabulary, as a bit set.
///
/// Values match `WIN_STATUS_*` in `GameWindow.h` so a decoded flag list maps one to one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UiStatus(u32);

impl UiStatus {
    /// `WIN_STATUS_ACTIVE`.
    pub const ACTIVE: Self = Self(0x0000_0001);
    /// `WIN_STATUS_TOGGLE`.
    pub const TOGGLE: Self = Self(0x0000_0002);
    /// `WIN_STATUS_DRAGGABLE`.
    pub const DRAGGABLE: Self = Self(0x0000_0004);
    /// `WIN_STATUS_ENABLED`.
    pub const ENABLED: Self = Self(0x0000_0008);
    /// `WIN_STATUS_HIDDEN`.
    pub const HIDDEN: Self = Self(0x0000_0010);
    /// `WIN_STATUS_ABOVE`.
    pub const ABOVE: Self = Self(0x0000_0020);
    /// `WIN_STATUS_BELOW`.
    pub const BELOW: Self = Self(0x0000_0040);
    /// `WIN_STATUS_IMAGE`.
    pub const IMAGE: Self = Self(0x0000_0080);
    /// `WIN_STATUS_TAB_STOP`.
    pub const TAB_STOP: Self = Self(0x0000_0100);
    /// `WIN_STATUS_NO_INPUT`.
    pub const NO_INPUT: Self = Self(0x0000_0200);
    /// `WIN_STATUS_NO_FOCUS`.
    pub const NO_FOCUS: Self = Self(0x0000_0400);
    /// `WIN_STATUS_BORDER`.
    pub const BORDER: Self = Self(0x0000_1000);
    /// `WIN_STATUS_SMOOTH_TEXT`.
    pub const SMOOTH_TEXT: Self = Self(0x0000_2000);
    /// `WIN_STATUS_ONE_LINE`.
    pub const ONE_LINE: Self = Self(0x0000_4000);
    /// `WIN_STATUS_SEE_THRU`.
    pub const SEE_THRU: Self = Self(0x0001_0000);
    /// `WIN_STATUS_RIGHT_CLICK`.
    pub const RIGHT_CLICK: Self = Self(0x0002_0000);
    /// `WIN_STATUS_WRAP_CENTERED`.
    pub const WRAP_CENTERED: Self = Self(0x0004_0000);
    /// `WIN_STATUS_CHECK_LIKE`.
    pub const CHECK_LIKE: Self = Self(0x0008_0000);
    /// `WIN_STATUS_DESTROYED`.
    pub const DESTROYED: Self = Self(0x0000_0800);
    /// `WIN_STATUS_NO_FLUSH`.
    pub const NO_FLUSH: Self = Self(0x0000_8000);
    /// `WIN_STATUS_HOTKEY_TEXT`.
    pub const HOTKEY_TEXT: Self = Self(0x0010_0000);
    /// `WIN_STATUS_USE_OVERLAY_STATES`.
    pub const USE_OVERLAY_STATES: Self = Self(0x0020_0000);
    /// `WIN_STATUS_NOT_READY`.
    pub const NOT_READY: Self = Self(0x0040_0000);
    /// `WIN_STATUS_FLASHING`.
    pub const FLASHING: Self = Self(0x0080_0000);
    /// `WIN_STATUS_ALWAYS_COLOR`.
    pub const ALWAYS_COLOR: Self = Self(0x0100_0000);
    /// `WIN_STATUS_ON_MOUSE_DOWN`.
    pub const ON_MOUSE_DOWN: Self = Self(0x0200_0000);

    /// The complete `WindowStatusNames` vocabulary, paired with its `WIN_STATUS_*` bit.
    ///
    /// Every name either edition can write is mapped, so a retail or modded layout produces no
    /// unmapped-status diagnostic. Bits beyond visibility, enablement, layering, input, focus, and
    /// tab stops are retained as the definition's request without a retained-state effect of their
    /// own; the presentation gates consume them.
    const NAMES: [(&'static str, Self); 26] = [
        ("ACTIVE", Self::ACTIVE),
        ("TOGGLE", Self::TOGGLE),
        ("DRAGABLE", Self::DRAGGABLE),
        ("ENABLED", Self::ENABLED),
        ("HIDDEN", Self::HIDDEN),
        ("ABOVE", Self::ABOVE),
        ("BELOW", Self::BELOW),
        ("IMAGE", Self::IMAGE),
        ("TABSTOP", Self::TAB_STOP),
        ("NOINPUT", Self::NO_INPUT),
        ("NOFOCUS", Self::NO_FOCUS),
        ("BORDER", Self::BORDER),
        ("SMOOTH_TEXT", Self::SMOOTH_TEXT),
        ("ONE_LINE", Self::ONE_LINE),
        ("SEE_THRU", Self::SEE_THRU),
        ("RIGHT_CLICK", Self::RIGHT_CLICK),
        ("WRAP_CENTERED", Self::WRAP_CENTERED),
        ("CHECK_LIKE", Self::CHECK_LIKE),
        ("DESTROYED", Self::DESTROYED),
        ("NO_FLUSH", Self::NO_FLUSH),
        ("HOTKEY_TEXT", Self::HOTKEY_TEXT),
        ("USE_OVERLAY_STATES", Self::USE_OVERLAY_STATES),
        ("NOT_READY", Self::NOT_READY),
        ("FLASHING", Self::FLASHING),
        ("ALWAYS_COLOR", Self::ALWAYS_COLOR),
        ("ON_MOUSE_DOWN", Self::ON_MOUSE_DOWN),
    ];

    /// Returns the bit a status name selects, or `None` for a name outside this vocabulary.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::NAMES
            .into_iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            .map(|(_, bit)| bit)
    }

    /// Returns whether every bit in `other` is set.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Returns whether any bit in `other` is set.
    #[must_use]
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    /// Returns the raw bits.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    const fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }
}

/// How a synthesised gadget child's status differs from the gadget's own.
///
/// The source builds each part by masking bits out of the parent's status and setting others, which
/// is why a part of a hidden or bordered gadget is neither.
#[derive(Debug, Clone, Copy)]
struct GadgetChildStatus {
    /// Bits forced on.
    set: UiStatus,
    /// Bits forced off.
    clear: UiStatus,
}

/// A stable, source-order control identity within one layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UiControlId(usize);

impl UiControlId {
    /// Returns the zero-based source-order index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

/// Per-family retained state for one control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiControlKind {
    /// `PUSHBUTTON`.
    PushButton,
    /// `RADIOBUTTON`, exclusive within its declared group.
    RadioButton {
        /// The group this button belongs to.
        group: i32,
        /// Whether it is the group's selection.
        selected: bool,
    },
    /// `CHECKBOX`.
    CheckBox {
        /// Whether the box is checked.
        checked: bool,
    },
    /// `VERTICALSLIDER` or `HORIZONTALSLIDER`.
    Slider {
        /// Inclusive lower bound.
        minimum: i32,
        /// Inclusive upper bound.
        maximum: i32,
        /// Current position, always within bounds.
        value: i32,
    },
    /// `SCROLLLISTBOX`.
    ListBox {
        /// Row labels or literals, in insertion order.
        rows: Vec<String>,
        /// Selected rows, ascending. At most one unless the definition allows multi-select.
        selected: Vec<usize>,
        /// The first visible row.
        scroll_top: usize,
        /// How many rows the definition displays at once.
        visible_rows: usize,
        /// Whether more than one row may be selected.
        multi_select: bool,
    },
    /// `COMBOBOX`.
    ComboBox {
        /// Entry labels or literals, in insertion order.
        entries: Vec<String>,
        /// The selected entry, absent when nothing is selected.
        selected: Option<usize>,
        /// Whether the drop-down is showing.
        open: bool,
        /// How many entries the definition displays at once.
        max_display: usize,
        /// Whether the edit field accepts typing.
        editable: bool,
        /// `MAXCHARS`, which the synthesised edit field is created with.
        max_chars: usize,
    },
    /// `ENTRYFIELD`.
    TextEntry {
        /// Current contents.
        text: String,
        /// Caret position, in characters.
        caret: usize,
        /// Maximum characters accepted.
        max_length: usize,
        /// Whether the contents render masked.
        secret: bool,
    },
    /// `STATICTEXT`.
    StaticText {
        /// Whether `STATICTEXTDATA` declares the text centred.
        centered: bool,
    },
    /// `PROGRESSBAR`.
    ProgressBar {
        /// Completion percentage, clamped to `0..=100`.
        progress: i32,
    },
    /// `TABCONTROL`.
    TabControl {
        /// The active pane.
        active_pane: usize,
        /// How many panes the definition declares.
        panes: usize,
        /// The declared tab strip, which the presentation layer composes from.
        geometry: UiTabGeometry,
    },
    /// Any other established window type, retained without family-specific state.
    Generic,
}

/// One retained control.
///
/// Live interaction state is a set of independent booleans because the original tracks each as its
/// own status or state bit, and collapsing them into one enum would lose combinations the original
/// allows — a control can be hovered and pressed and disabled at once.
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag mirrors an independent source state bit"
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiControl {
    id: UiControlId,
    parent: Option<UiControlId>,
    children: Vec<UiControlId>,
    depth: usize,
    name: Option<String>,
    window_type: String,
    rect: UiRect,
    image_offset: Option<(i32, i32)>,
    status: UiStatus,
    hidden: bool,
    enabled: bool,
    hovered: bool,
    pressed: bool,
    text_label: Option<String>,
    tooltip_label: Option<String>,
    font: Option<(String, i32, bool)>,
    header_template: Option<String>,
    system_callback: Option<String>,
    input_callback: Option<String>,
    tooltip_callback: Option<String>,
    draw_callback: Option<String>,
    draw_data: Vec<(WndDrawDataSlot, WndDrawData)>,
    text_colors: Option<WndTextColors>,
    kind: UiControlKind,
    role: Option<UiGadgetRole>,
    /// `LISTBOXDATA`'s `SCROLLBAR`, which decides whether the list box builds a scroll bar. It is
    /// creation-time input rather than retained state, so it stays out of [`UiControlKind`].
    list_box_scroll_bar: bool,
    /// The `MOUSETRACK` style bit, which is what makes hovering change a control's appearance.
    mouse_track: bool,
}

impl UiControl {
    /// Returns the stable source-order identity.
    #[must_use]
    pub const fn id(&self) -> UiControlId {
        self.id
    }

    /// Returns the parent control, absent for a top-level control.
    #[must_use]
    pub const fn parent(&self) -> Option<UiControlId> {
        self.parent
    }

    /// Returns child controls in source order.
    #[must_use]
    pub fn children(&self) -> &[UiControlId] {
        &self.children
    }

    /// Returns which gadget part this control is, absent for a control the definition declared.
    ///
    /// A synthesised child is built by the original's gadget-creation code rather than written in
    /// the layout, so nothing in the file names it and a report needs this to explain where it came
    /// from.
    #[must_use]
    pub const fn gadget_role(&self) -> Option<UiGadgetRole> {
        self.role
    }

    /// Returns the nesting depth, zero for a top-level control.
    #[must_use]
    pub const fn depth(&self) -> usize {
        self.depth
    }

    /// Returns the full decorated name from the definition's `NAME` record, which is what a patch
    /// overlay targets.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Returns the undecorated control part of the name, absent when the definition declared only a
    /// layout prefix.
    ///
    /// Retail layouts give some windows only the prefix (`"OptionsMenu.wnd:"`); those are unnamed
    /// controls rather than controls sharing a name, so identity comparison uses this part.
    #[must_use]
    pub fn control_name(&self) -> Option<&str> {
        let name = self.name.as_deref()?;
        let (_, control) = name.split_once(':')?;
        (!control.is_empty()).then_some(control)
    }

    /// Returns the declared window type.
    #[must_use]
    pub fn window_type(&self) -> &str {
        &self.window_type
    }

    /// Returns the parent-relative rectangle for the instantiated viewport.
    #[must_use]
    pub const fn rect(&self) -> UiRect {
        self.rect
    }

    /// Returns the declared status bits, which are the definition's request rather than live state.
    #[must_use]
    pub const fn status(&self) -> UiStatus {
        self.status
    }

    /// Returns whether the control is currently hidden.
    #[must_use]
    pub const fn is_hidden(&self) -> bool {
        self.hidden
    }

    /// Returns whether the control currently accepts input.
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Returns whether the pointer is currently over the control.
    #[must_use]
    pub const fn is_hovered(&self) -> bool {
        self.hovered
    }

    /// Returns whether the control is currently held down.
    #[must_use]
    pub const fn is_pressed(&self) -> bool {
        self.pressed
    }

    /// Returns whether the control declares `MOUSETRACK`.
    ///
    /// This is `GWS_MOUSE_TRACK`, and it is the gate on hover changing anything an eye can see: every
    /// gadget's input handler sets `WIN_STATE_HILITED` on `GWM_MOUSE_ENTERING` only when the bit is
    /// set, and the window manager itself never sets that state. A control without it is drawn the
    /// same whether the pointer is over it or not.
    #[must_use]
    pub const fn is_mouse_track(&self) -> bool {
        self.mouse_track
    }

    /// Returns the `TEXT` record's value, which callers resolve as a label or literal.
    #[must_use]
    pub fn text_label(&self) -> Option<&str> {
        self.text_label.as_deref()
    }

    /// Returns the `TOOLTIPTEXT` record's value.
    #[must_use]
    pub fn tooltip_label(&self) -> Option<&str> {
        self.tooltip_label.as_deref()
    }

    /// Returns the declared font as family, point size, and weight.
    #[must_use]
    pub fn font(&self) -> Option<(&str, i32, bool)> {
        self.font
            .as_ref()
            .map(|(name, size, bold)| (name.as_str(), *size, *bold))
    }

    /// Returns the declared header template name.
    #[must_use]
    pub fn header_template(&self) -> Option<&str> {
        self.header_template.as_deref()
    }

    /// Returns the retained system callback name. It is data: nothing here dispatches it.
    #[must_use]
    pub fn system_callback(&self) -> Option<&str> {
        self.system_callback.as_deref()
    }

    /// Returns the retained input callback name. It is data: nothing here dispatches it.
    #[must_use]
    pub fn input_callback(&self) -> Option<&str> {
        self.input_callback.as_deref()
    }

    /// Returns the retained tooltip callback name. It is data: nothing here dispatches it.
    #[must_use]
    pub fn tooltip_callback(&self) -> Option<&str> {
        self.tooltip_callback.as_deref()
    }

    /// Returns the family-specific retained state.
    #[must_use]
    pub const fn kind(&self) -> &UiControlKind {
        &self.kind
    }

    /// Returns every image name in one draw-data slot, in entry order.
    ///
    /// A slot always holds [`WND_DRAW_DATA_ENTRIES`] entries; a control declaring no draw data for
    /// the slot yields that many absent names, so an index is always in range for a caller composing
    /// pieces by index.
    #[must_use]
    pub fn draw_entry_images(&self, slot: WndDrawDataSlot) -> Vec<Option<String>> {
        self.draw_data
            .iter()
            .find(|(candidate, _)| *candidate == slot)
            .map_or_else(
                || vec![None; WND_DRAW_DATA_ENTRIES],
                |(_, data)| {
                    data.entries()
                        .iter()
                        .map(|entry| entry.image().map(str::to_owned))
                        .collect()
                },
            )
    }

    /// Returns the control's `IMAGEOFFSET`, which the source adds to every drawn piece's position.
    #[must_use]
    pub const fn image_offset(&self) -> Option<(i32, i32)> {
        self.image_offset
    }

    /// Returns the retained draw callback name. It is data: nothing here dispatches it.
    #[must_use]
    pub fn draw_callback(&self) -> Option<&str> {
        self.draw_callback.as_deref()
    }

    /// Returns whether this control draws through an image path rather than a colour-only one.
    ///
    /// The source resolves this in two steps. Creating a gadget assigns a default procedure from the
    /// `IMAGE` status bit — `getPushButtonImageDrawFunc` against `getPushButtonDrawFunc` — and a
    /// `DRAWCALLBACK` the function lexicon resolves then replaces it. So a name that reads as a
    /// bound draw procedure decides, and anything else, including the overwhelmingly common
    /// `"[None]"` and any name the lexicon would not resolve, leaves the status bit deciding.
    #[must_use]
    pub fn is_image_draw(&self) -> bool {
        match self.draw_callback.as_deref() {
            Some(callback) if callback.ends_with("ImageDraw") => true,
            Some(callback) if callback.ends_with("Draw") => false,
            _ => self.status.contains(UiStatus::IMAGE),
        }
    }

    /// Returns which family's draw-data composition rules apply.
    ///
    /// A slider's orientation is not in its retained state — both orientations decode one
    /// `SLIDERDATA` — so the declared window type distinguishes them, as it does in the source's
    /// own `winCreateFromScript` dispatch.
    #[must_use]
    pub fn family(&self) -> UiControlFamily {
        match self.kind {
            UiControlKind::PushButton => UiControlFamily::PushButton,
            UiControlKind::RadioButton { .. } => UiControlFamily::RadioButton,
            UiControlKind::CheckBox { .. } => UiControlFamily::CheckBox,
            UiControlKind::TextEntry { .. } => UiControlFamily::TextEntry,
            UiControlKind::ProgressBar { .. } => UiControlFamily::ProgressBar,
            UiControlKind::TabControl { geometry, .. } => UiControlFamily::TabControl(geometry),
            UiControlKind::Slider { .. } if self.window_type.eq_ignore_ascii_case("HORZSLIDER") => {
                UiControlFamily::HorizontalSlider
            }
            UiControlKind::Slider { .. } => UiControlFamily::VerticalSlider,
            _ => UiControlFamily::Simple,
        }
    }

    /// Returns the source's `WIN_STATE_SELECTED` for this control.
    ///
    /// The bit means something different per family — a push button is selected while held down, a
    /// check box while checked, a radio button while it is its group's choice — and each family's
    /// draw procedure reads it to pick its selected art.
    #[must_use]
    pub const fn is_selected(&self) -> bool {
        match self.kind {
            UiControlKind::RadioButton { selected, .. } => selected,
            UiControlKind::CheckBox { checked } => checked,
            _ => self.pressed,
        }
    }

    /// Returns the scalar draw inputs a family's composition reads: a slider's bounds and position,
    /// or a progress bar's percentage.
    #[must_use]
    pub const fn draw_bounds(&self) -> UiDrawState {
        let (value, minimum, maximum) = match self.kind {
            UiControlKind::Slider {
                minimum,
                maximum,
                value,
            } => (value, minimum, maximum),
            UiControlKind::ProgressBar { progress } => (progress, 0, 100),
            _ => (0, 0, 0),
        };
        UiDrawState {
            enabled: false,
            hilited: false,
            selected: false,
            value,
            minimum,
            maximum,
        }
    }

    /// Returns every image name this control declares, by slot and entry index.
    ///
    /// A slot the control does not declare yields [`WND_DRAW_DATA_ENTRIES`] absent names, so an
    /// index a family composes from is always in range.
    #[must_use]
    pub fn slot_images(&self) -> UiSlotImages {
        UiSlotImages::new(
            self.draw_entry_images(WndDrawDataSlot::Enabled),
            self.draw_entry_images(WndDrawDataSlot::Disabled),
            self.draw_entry_images(WndDrawDataSlot::Hilite),
        )
    }

    /// Returns where this control's text sits inside its rectangle.
    ///
    /// `drawButtonText` centres a push button's text on both axes unless the control declares
    /// `SHORTCUT_BUTTON`, which the source itself calls a hack for drawing at the top left.
    /// `drawRadioButtonText` centres the same way. `drawCheckBoxText` does not: it centres
    /// vertically but indents the label by the control's own height, clearing the box image.
    /// Static text centres only when its own `CENTERED` flag is set.
    #[must_use]
    pub const fn text_align(&self) -> UiTextAlign {
        match self.kind {
            UiControlKind::PushButton
            | UiControlKind::RadioButton { .. }
            | UiControlKind::StaticText { centered: true } => UiTextAlign::Centered,
            UiControlKind::CheckBox { .. } => UiTextAlign::CenteredBesideBox,
            _ => UiTextAlign::TopLeft,
        }
    }

    /// Returns one draw-data slot's first state entry, which is the entry the base visual uses.
    #[must_use]
    pub fn draw_entry(&self, slot: WndDrawDataSlot) -> Option<&WndDrawEntry> {
        self.draw_data
            .iter()
            .find(|(candidate, _)| *candidate == slot)
            .and_then(|(_, data)| data.entries().first())
    }

    /// Returns the text colour a draw-data slot selects, absent when the control declares no
    /// `TEXTCOLOR` record.
    #[must_use]
    pub fn text_color(&self, slot: WndDrawDataSlot) -> Option<WndColor> {
        let colors = self.text_colors?;
        Some(match slot {
            WndDrawDataSlot::Disabled => colors.disabled(),
            WndDrawDataSlot::Hilite => colors.hilite(),
            _ => colors.enabled(),
        })
    }

    /// Returns the text to display: a text entry's live contents, otherwise the `TEXT` record.
    #[must_use]
    pub fn displayed_text(&self) -> Option<&str> {
        match &self.kind {
            UiControlKind::TextEntry { text, .. } => Some(text.as_str()),
            _ => self.text_label.as_deref(),
        }
    }

    /// Returns whether the control's text renders masked.
    #[must_use]
    pub const fn is_secret_text(&self) -> bool {
        matches!(self.kind, UiControlKind::TextEntry { secret: true, .. })
    }

    pub(crate) const fn set_hovered(&mut self, hovered: bool) {
        self.hovered = hovered;
    }

    pub(crate) const fn set_pressed(&mut self, pressed: bool) {
        self.pressed = pressed;
    }

    pub(crate) const fn kind_mut(&mut self) -> &mut UiControlKind {
        &mut self.kind
    }
}

/// Why instantiation failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiLayoutError {
    /// A viewport extent was not positive.
    InvalidViewport {
        /// The rejected width.
        width: i32,
        /// The rejected height.
        height: i32,
    },
    /// The definition declares more controls than [`UiLimits::max_controls`].
    TooManyControls {
        /// The configured limit.
        limit: usize,
    },
    /// The definition nests deeper than [`UiLimits::max_depth`].
    TooDeep {
        /// The configured limit.
        limit: usize,
    },
    /// A control declared a creation resolution with a zero or negative extent, which cannot be
    /// scaled.
    InvalidCreationResolution {
        /// The control's source-order index.
        control: usize,
        /// The declared width.
        width: i32,
        /// The declared height.
        height: i32,
    },
}

impl Display for UiLayoutError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidViewport { width, height } => {
                write!(formatter, "viewport {width}x{height} is not positive")
            }
            Self::TooManyControls { limit } => {
                write!(formatter, "layout exceeds the {limit}-control limit")
            }
            Self::TooDeep { limit } => write!(formatter, "layout exceeds the {limit}-depth limit"),
            Self::InvalidCreationResolution {
                control,
                width,
                height,
            } => write!(
                formatter,
                "control {control} declares creation resolution {width}x{height}, which cannot scale"
            ),
        }
    }
}

impl Error for UiLayoutError {}

/// A non-fatal observation from instantiation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiDiagnosticKind {
    /// A status name outside the vocabulary this crate maps. The flag is retained in the definition
    /// but has no retained-state effect.
    UnmappedStatus {
        /// The name exactly as spelled.
        name: Box<str>,
    },
    /// A slider declared `maximum` below `minimum`; the bounds were ordered before use.
    InvertedSliderBounds {
        /// The declared lower bound.
        minimum: i32,
        /// The declared upper bound.
        maximum: i32,
    },
    /// A list box declared more visible rows than [`UiLimits::max_list_rows`]; the limit applies.
    ListRowsClamped {
        /// The declared count.
        declared: i32,
        /// The applied count.
        applied: usize,
    },
    /// A text entry declared a length above [`UiLimits::max_text_length`]; the limit applies.
    TextLengthClamped {
        /// The declared length.
        declared: i32,
        /// The applied length.
        applied: usize,
    },
    /// A scroll list box declared text, so `GadgetListboxCreateScrollbar` would inset its scroll bar
    /// by the title's font height. This crate holds no font metrics and laid the bar out as though
    /// the box were untitled. No retail layout reaches this.
    UntitledScrollBarAssumed,
}

/// One non-fatal instantiation observation, attributed to a control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiDiagnostic {
    control: UiControlId,
    kind: UiDiagnosticKind,
}

impl UiDiagnostic {
    /// Returns the control the observation applies to.
    #[must_use]
    pub const fn control(&self) -> UiControlId {
        self.control
    }

    /// Returns the observation detail.
    #[must_use]
    pub const fn kind(&self) -> &UiDiagnosticKind {
        &self.kind
    }
}

/// One instantiated layout: the retained control tree plus its focus and capture state.
#[derive(Debug, Clone)]
pub struct UiLayout {
    controls: Vec<UiControl>,
    roots: Vec<UiControlId>,
    tab_order: Vec<UiControlId>,
    presentation: UiPresentation,
    limits: UiLimits,
    focus: Option<UiControlId>,
    capture: Option<UiControlId>,
    diagnostics: Vec<UiDiagnostic>,
    layout_init: Option<String>,
    layout_update: Option<String>,
    layout_shutdown: Option<String>,
    hidden: bool,
}

impl UiLayout {
    /// Instantiates an immutable definition for one viewport and scale policy.
    ///
    /// Rectangles are resolved exactly as the original reader resolves them: each stored corner is
    /// scaled by the viewport-to-creation-resolution ratio and truncated, size is derived from the
    /// scaled corners, and a child's position is made relative to its parent's already-scaled
    /// screen position. Declared `HIDDEN` and `ENABLED` bits seed live state; every other bit is
    /// retained as the definition's request.
    ///
    /// # Errors
    ///
    /// Returns a structured error when the control count or nesting depth exceeds its limit, or
    /// when a control declares a creation resolution that cannot be scaled.
    pub fn instantiate(
        document: &WndDocument,
        presentation: UiPresentation,
        limits: UiLimits,
    ) -> Result<Self, UiLayoutError> {
        let block = document.layout();
        let mut layout = Self {
            controls: Vec::new(),
            roots: Vec::new(),
            tab_order: Vec::new(),
            presentation,
            limits,
            focus: None,
            capture: None,
            diagnostics: Vec::new(),
            layout_init: block.and_then(|block| block.init()).map(str::to_owned),
            layout_update: block.and_then(|block| block.update()).map(str::to_owned),
            layout_shutdown: block.and_then(|block| block.shutdown()).map(str::to_owned),
            // `WindowLayout`'s constructor leaves a layout visible; its windows carry their own
            // declared `HIDDEN` bits independently.
            hidden: false,
        };
        for window in document.windows() {
            let id = layout.add(window, None, 0)?;
            layout.roots.push(id);
        }
        layout.tab_order = layout
            .controls
            .iter()
            .filter(|control| control.status.contains(UiStatus::TAB_STOP))
            .map(|control| control.id)
            .collect();
        Ok(layout)
    }

    fn add(
        &mut self,
        window: &WndWindow,
        parent: Option<UiControlId>,
        depth: usize,
    ) -> Result<UiControlId, UiLayoutError> {
        if depth >= self.limits.max_depth {
            return Err(UiLayoutError::TooDeep {
                limit: self.limits.max_depth,
            });
        }
        if self.controls.len() >= self.limits.max_controls {
            return Err(UiLayoutError::TooManyControls {
                limit: self.limits.max_controls,
            });
        }
        let id = UiControlId(self.controls.len());
        let mut status = UiStatus::default();
        let mut diagnostics = Vec::new();
        for flag in window.status() {
            match UiStatus::from_name(flag.name()) {
                Some(bit) => status = status.with(bit),
                None => diagnostics.push(UiDiagnostic {
                    control: id,
                    kind: UiDiagnosticKind::UnmappedStatus {
                        name: flag.name().to_owned().into_boxed_str(),
                    },
                }),
            }
        }
        let parent_origin = parent.map_or(UiPoint::new(0, 0), |parent| self.screen_origin(parent));
        let rect = self.scaled_rect(window, id, parent_origin)?;
        let (kind, mut kind_diagnostics) = self.control_kind(window, id);
        diagnostics.append(&mut kind_diagnostics);
        self.diagnostics.append(&mut diagnostics);

        self.controls.push(UiControl {
            id,
            parent,
            children: Vec::new(),
            depth,
            name: window.name().map(str::to_owned),
            window_type: window.window_type().to_owned(),
            rect,
            image_offset: window.image_offset(),
            status,
            hidden: status.contains(UiStatus::HIDDEN),
            enabled: status.contains(UiStatus::ENABLED),
            hovered: false,
            pressed: false,
            // `MOUSETRACK` is matched case-insensitively because a WND flag list is authored freely
            // and the source's own lookup upper-cases before comparing.
            mouse_track: window
                .style()
                .iter()
                .any(|flag| flag.name().eq_ignore_ascii_case("MOUSETRACK")),
            text_label: window.text().map(str::to_owned),
            tooltip_label: window.tooltip_text().map(str::to_owned),
            font: window
                .font()
                .map(|font| (font.name().to_owned(), font.size(), font.bold())),
            header_template: window.header_template().map(str::to_owned),
            system_callback: window
                .callbacks()
                .get(cic_formats::WndCallbackKind::System)
                .map(str::to_owned),
            input_callback: window
                .callbacks()
                .get(cic_formats::WndCallbackKind::Input)
                .map(str::to_owned),
            tooltip_callback: window
                .callbacks()
                .get(cic_formats::WndCallbackKind::Tooltip)
                .map(str::to_owned),
            draw_callback: window
                .callbacks()
                .get(cic_formats::WndCallbackKind::Draw)
                .map(str::to_owned),
            draw_data: window.draw_data().to_vec(),
            text_colors: window.text_colors().copied(),
            kind,
            role: None,
            list_box_scroll_bar: matches!(
                window.gadget_data(),
                Some(WndGadgetData::ListBox(data)) if data.scroll_bar()
            ),
        });
        if let Some(parent) = parent {
            self.controls[parent.0].children.push(id);
        }
        // The original's gadget-creation functions run inside `winCreate`, before the script reads
        // the gadget's own `CHILD` list, so a synthesised part precedes every declared child.
        self.synthesise_children(id, depth)?;
        for child in window.children() {
            self.add(child, Some(id), depth + 1)?;
        }
        Ok(id)
    }

    /// Builds the child windows the original's gadget-creation code creates for a control.
    ///
    /// A slider gets a thumb from `gogoGadgetSlider`; a scroll list box that asks for a scroll bar
    /// gets an up button, a down button, and a vertical slider from `GadgetListboxCreateScrollbar`;
    /// a combo box gets a drop-down button, an edit field, and a hidden drop-down list from
    /// `gogoGadgetComboBox`. Each part draws from the parent's own child-specific draw-data records,
    /// which is what `winCreateFromScript` copies into them once the parts exist — and it is the
    /// reason a layout carries `COMBOBOXEDITBOX*`, `LISTBOX*SLIDER*`, and `SLIDERTHUMB*` arrays at
    /// all.
    fn synthesise_children(&mut self, id: UiControlId, depth: usize) -> Result<(), UiLayoutError> {
        let control = &self.controls[id.0];
        let UiRect { width, height, .. } = control.rect;
        match control.kind {
            UiControlKind::Slider { .. } => self.add_slider_thumb(id, depth)?,
            UiControlKind::ListBox { .. } if control.list_box_scroll_bar => {
                self.add_scroll_bar(id, depth, width, height)?;
            }
            UiControlKind::ComboBox { .. } => self.add_combo_box_parts(id, depth, width, height)?,
            _ => {}
        }
        Ok(())
    }

    /// Reproduces the thumb `gogoGadgetSlider` gives every slider it creates.
    fn add_slider_thumb(&mut self, id: UiControlId, depth: usize) -> Result<(), UiLayoutError> {
        // The same function sets `WIN_STATUS_TAB_STOP` on the slider itself, whatever the layout
        // declared, and the thumb inherits that status alongside `DRAGGABLE`.
        self.controls[id.0].status = self.controls[id.0].status.with(UiStatus::TAB_STOP);
        let width = self.controls[id.0].rect.width;
        let rect = if self.controls[id.0].family() == UiControlFamily::HorizontalSlider {
            UiRect {
                x: 0,
                y: HORIZONTAL_SLIDER_THUMB_POSITION,
                width: HORIZONTAL_SLIDER_THUMB_WIDTH,
                height: HORIZONTAL_SLIDER_THUMB_HEIGHT,
            }
        } else {
            // The vertical thumb is one pixel taller than it is wide, from the source's
            // `width, width + 1` argument pair.
            UiRect {
                x: 0,
                y: 0,
                width,
                height: width + 1,
            }
        };
        self.add_gadget_child(
            id,
            depth,
            UiGadgetRole::SliderThumb,
            rect,
            UiControlKind::PushButton,
            "PUSHBUTTON",
            [
                WndDrawDataSlot::SliderThumbEnabled,
                WndDrawDataSlot::SliderThumbDisabled,
                WndDrawDataSlot::SliderThumbHilite,
            ],
            GadgetChildStatus {
                set: UiStatus::ENABLED.with(UiStatus::DRAGGABLE),
                clear: UiStatus::HIDDEN,
            },
            None,
        )?;
        Ok(())
    }

    /// Reproduces the three parts `gogoGadgetComboBox` builds, and the scroll bar inside its list.
    fn add_combo_box_parts(
        &mut self,
        id: UiControlId,
        depth: usize,
        width: i32,
        height: i32,
    ) -> Result<(), UiLayoutError> {
        let editable = matches!(
            self.controls[id.0].kind,
            UiControlKind::ComboBox { editable: true, .. }
        );
        {
            // Shared by all three parts: the parts are never borders and never start hidden,
            // however the combo box itself was declared.
            let strip = UiStatus::BORDER.with(UiStatus::HIDDEN);
            self.add_gadget_child(
                id,
                depth,
                UiGadgetRole::ComboBoxDropDownButton,
                UiRect {
                    x: width - GADGET_BUTTON_WIDTH,
                    y: 0,
                    width: GADGET_BUTTON_WIDTH,
                    height,
                },
                UiControlKind::PushButton,
                "PUSHBUTTON",
                [
                    WndDrawDataSlot::ComboBoxDropDownButtonEnabled,
                    WndDrawDataSlot::ComboBoxDropDownButtonDisabled,
                    WndDrawDataSlot::ComboBoxDropDownButtonHilite,
                ],
                GadgetChildStatus {
                    set: UiStatus::ACTIVE.with(UiStatus::ENABLED),
                    clear: strip,
                },
                None,
            )?;
            self.add_gadget_child(
                id,
                depth,
                UiGadgetRole::ComboBoxEditBox,
                UiRect {
                    x: 0,
                    y: 0,
                    width: width - GADGET_BUTTON_WIDTH,
                    height,
                },
                UiControlKind::TextEntry {
                    text: String::new(),
                    caret: 0,
                    max_length: self.combo_box_max_chars(id),
                    secret: false,
                },
                "ENTRYFIELD",
                [
                    WndDrawDataSlot::ComboBoxEditBoxEnabled,
                    WndDrawDataSlot::ComboBoxEditBoxDisabled,
                    WndDrawDataSlot::ComboBoxEditBoxHilite,
                ],
                GadgetChildStatus {
                    // A non-editable combo box's field refuses input; the source leaves its
                    // `NO_FOCUS` companion commented out, so focus still reaches it.
                    set: if editable {
                        UiStatus::default()
                    } else {
                        UiStatus::NO_INPUT
                    },
                    clear: strip,
                },
                // `winInstData.m_textLabelString = "Entry"`, a literal rather than a CSF label.
                Some("Entry".to_owned()),
            )?;
            self.add_combo_box_list(id, depth, width, height, strip)?;
        }
        Ok(())
    }

    /// Builds the drop-down list `gogoGadgetComboBox` creates last, and the scroll bar inside it.
    fn add_combo_box_list(
        &mut self,
        id: UiControlId,
        depth: usize,
        width: i32,
        height: i32,
        strip: UiStatus,
    ) -> Result<(), UiLayoutError> {
        let list = self.add_gadget_child(
            id,
            depth,
            UiGadgetRole::ComboBoxListBox,
            // The drop-down hangs directly below the closed box and repeats its height.
            UiRect {
                x: 0,
                y: height,
                width,
                height,
            },
            UiControlKind::ListBox {
                rows: Vec::new(),
                selected: Vec::new(),
                scroll_top: 0,
                // `cData->listboxData->listLength = 10` in `winCreateFromScript`.
                visible_rows: 10,
                multi_select: false,
            },
            "SCROLLLISTBOX",
            [
                WndDrawDataSlot::ComboBoxListBoxEnabled,
                WndDrawDataSlot::ComboBoxListBoxDisabled,
                WndDrawDataSlot::ComboBoxListBoxHilite,
            ],
            GadgetChildStatus {
                set: UiStatus::ABOVE.with(UiStatus::ONE_LINE),
                // The list is created without the combo box's `IMAGE` bit and is hidden immediately
                // afterwards by `winHide( TRUE )`.
                clear: strip.with(UiStatus::IMAGE),
            },
            None,
        )?;
        // `winHide( TRUE )` immediately after creation: the list is built visible and then hidden,
        // which is why its status carries no `HIDDEN` bit while its live state does.
        self.set_hidden(list, true);
        // That list is created with `scrollBar = 1`, so it builds a scroll bar of its own. Its parts
        // read the *combo box's* `LISTBOX*` and `SLIDERTHUMB*` records, so the list has to carry
        // them down for its own gadget creation to find.
        self.controls[list.0].list_box_scroll_bar = true;
        self.inherit_slots(
            list,
            id,
            &[
                WndDrawDataSlot::ListBoxEnabledUpButton,
                WndDrawDataSlot::ListBoxDisabledUpButton,
                WndDrawDataSlot::ListBoxHiliteUpButton,
                WndDrawDataSlot::ListBoxEnabledDownButton,
                WndDrawDataSlot::ListBoxDisabledDownButton,
                WndDrawDataSlot::ListBoxHiliteDownButton,
                WndDrawDataSlot::ListBoxEnabledSlider,
                WndDrawDataSlot::ListBoxDisabledSlider,
                WndDrawDataSlot::ListBoxHiliteSlider,
                WndDrawDataSlot::SliderThumbEnabled,
                WndDrawDataSlot::SliderThumbDisabled,
                WndDrawDataSlot::SliderThumbHilite,
            ],
        );
        let list_rect = self.controls[list.0].rect;
        self.add_scroll_bar(list, depth + 1, list_rect.width, list_rect.height)
    }

    /// Copies draw-data records from one control onto another under the same slot names.
    ///
    /// `winCreateFromScript` reads every child-part array of the window it is creating into file
    /// statics, then hands each one to whichever descendant draws it — so a scroll bar's thumb takes
    /// its art from the list box two levels above it, not from the slider that owns it. Carrying the
    /// records down keeps that reach without a global.
    fn inherit_slots(&mut self, child: UiControlId, owner: UiControlId, slots: &[WndDrawDataSlot]) {
        let inherited: Vec<_> = self.controls[owner.0]
            .draw_data
            .iter()
            .filter(|(slot, _)| slots.contains(slot))
            .cloned()
            .collect();
        self.controls[child.0].draw_data.extend(inherited);
    }

    /// Reproduces `GadgetListboxCreateScrollbar` for one list box.
    ///
    /// `top` and `bottom` in the source allow for a list title, but no retail layout gives a scroll
    /// list box any text and a combo box's internal list is created with none, so the title branch
    /// is unreachable here. It is the only part of this function that would need font metrics, and
    /// [`UiDiagnosticKind::UntitledScrollBarAssumed`] reports a layout that reaches it.
    fn add_scroll_bar(
        &mut self,
        id: UiControlId,
        depth: usize,
        width: i32,
        height: i32,
    ) -> Result<(), UiLayoutError> {
        if self.controls[id.0]
            .text_label
            .as_ref()
            .is_some_and(|text| !text.is_empty())
        {
            self.diagnostics.push(UiDiagnostic {
                control: id,
                kind: UiDiagnosticKind::UntitledScrollBarAssumed,
            });
        }
        let (top, bottom) = (0, height);
        // The parts are always image-drawn, never bordered, hidden, or input-refusing, whatever the
        // list box declared.
        let status = GadgetChildStatus {
            set: UiStatus::IMAGE
                .with(UiStatus::ACTIVE)
                .with(UiStatus::ENABLED),
            clear: UiStatus::BORDER
                .with(UiStatus::HIDDEN)
                .with(UiStatus::NO_INPUT),
        };
        self.add_gadget_child(
            id,
            depth,
            UiGadgetRole::ListBoxUpButton,
            UiRect {
                x: width - GADGET_BUTTON_WIDTH - 2,
                y: top + 2,
                width: GADGET_BUTTON_WIDTH,
                height: GADGET_BUTTON_HEIGHT,
            },
            UiControlKind::PushButton,
            "PUSHBUTTON",
            [
                WndDrawDataSlot::ListBoxEnabledUpButton,
                WndDrawDataSlot::ListBoxDisabledUpButton,
                WndDrawDataSlot::ListBoxHiliteUpButton,
            ],
            status,
            None,
        )?;
        self.add_gadget_child(
            id,
            depth,
            UiGadgetRole::ListBoxDownButton,
            UiRect {
                x: width - GADGET_BUTTON_WIDTH - 2,
                y: top + bottom - GADGET_BUTTON_HEIGHT - 2,
                width: GADGET_BUTTON_WIDTH,
                height: GADGET_BUTTON_HEIGHT,
            },
            UiControlKind::PushButton,
            "PUSHBUTTON",
            [
                WndDrawDataSlot::ListBoxEnabledDownButton,
                WndDrawDataSlot::ListBoxDisabledDownButton,
                WndDrawDataSlot::ListBoxHiliteDownButton,
            ],
            status,
            None,
        )?;
        let slider = self.add_gadget_child(
            id,
            depth,
            UiGadgetRole::ListBoxSlider,
            UiRect {
                x: width - GADGET_BUTTON_WIDTH - 2,
                y: top + GADGET_BUTTON_HEIGHT + 3,
                width: GADGET_BUTTON_WIDTH,
                height: bottom - (2 * GADGET_BUTTON_HEIGHT) - 6,
            },
            UiControlKind::Slider {
                minimum: 0,
                // `gogoGadgetSlider` widens an empty range rather than dividing by zero, and the
                // scroll bar hands it a zeroed `SliderData`.
                maximum: 1,
                value: 0,
            },
            "VERTSLIDER",
            [
                WndDrawDataSlot::ListBoxEnabledSlider,
                WndDrawDataSlot::ListBoxDisabledSlider,
                WndDrawDataSlot::ListBoxHiliteSlider,
            ],
            status,
            None,
        )?;
        // The slider is a slider, so it builds a thumb — but the `SLIDERTHUMB*` records belong to
        // the list box, so they have to reach the slider before it creates one.
        self.inherit_slots(
            slider,
            id,
            &[
                WndDrawDataSlot::SliderThumbEnabled,
                WndDrawDataSlot::SliderThumbDisabled,
                WndDrawDataSlot::SliderThumbHilite,
            ],
        );
        self.synthesise_children(slider, depth + 1)
    }

    /// Returns a combo box's declared `MAXCHARS`, which its edit field is created with.
    fn combo_box_max_chars(&self, id: UiControlId) -> usize {
        match self.controls[id.0].kind {
            UiControlKind::ComboBox { max_chars, .. } => max_chars,
            _ => 0,
        }
    }

    /// Pushes one synthesised gadget child and returns its id.
    #[expect(
        clippy::too_many_arguments,
        reason = "one child's complete construction; the caller reads as the source function it                   reproduces"
    )]
    fn add_gadget_child(
        &mut self,
        parent: UiControlId,
        depth: usize,
        role: UiGadgetRole,
        rect: UiRect,
        kind: UiControlKind,
        window_type: &str,
        slots: [WndDrawDataSlot; 3],
        status: GadgetChildStatus,
        text_label: Option<String>,
    ) -> Result<UiControlId, UiLayoutError> {
        if depth + 1 >= self.limits.max_depth {
            return Err(UiLayoutError::TooDeep {
                limit: self.limits.max_depth,
            });
        }
        if self.controls.len() >= self.limits.max_controls {
            return Err(UiLayoutError::TooManyControls {
                limit: self.limits.max_controls,
            });
        }
        let id = UiControlId(self.controls.len());
        let owner = &self.controls[parent.0];
        let child_status = owner.status.without(status.clear).with(status.set);
        // Each part's own enabled/disabled/hilite arrays are the parent's records for that part,
        // copied across by `winCreateFromScript` once the part exists.
        let draw_data = [
            WndDrawDataSlot::Enabled,
            WndDrawDataSlot::Disabled,
            WndDrawDataSlot::Hilite,
        ]
        .into_iter()
        .zip(slots)
        .filter_map(|(own, from)| {
            owner
                .draw_data
                .iter()
                .find(|(slot, _)| *slot == from)
                .map(|(_, data)| (own, data.clone()))
        })
        .collect();
        let name = owner
            .name
            .as_ref()
            .map(|name| format!("{name}{}", role.name_suffix()));
        // The parts inherit the gadget's font and text colours, which is what `gogoGadgetComboBox`
        // copies explicitly and what `winCreate` gives the rest by default.
        let font = owner.font.clone();
        let text_colors = owner.text_colors;
        self.controls.push(UiControl {
            id,
            parent: Some(parent),
            children: Vec::new(),
            depth: depth + 1,
            name,
            window_type: window_type.to_owned(),
            rect,
            image_offset: None,
            status: child_status,
            hidden: child_status.contains(UiStatus::HIDDEN),
            enabled: child_status.contains(UiStatus::ENABLED),
            hovered: false,
            pressed: false,
            // A created part tracks the mouse when its owner does: `gogoGadgetSlider` and
            // `gogoGadgetComboBox` each copy `GWS_MOUSE_TRACK` across only if the owner declares it.
            // The combo box's drop-down list is the exception, built with the bit unconditionally.
            mouse_track: owner.mouse_track || role == UiGadgetRole::ComboBoxListBox,
            text_label,
            tooltip_label: None,
            font,
            header_template: None,
            system_callback: None,
            input_callback: None,
            tooltip_callback: None,
            draw_callback: None,
            draw_data,
            text_colors,
            kind,
            role: Some(role),
            list_box_scroll_bar: false,
        });
        self.controls[parent.0].children.push(id);
        Ok(id)
    }

    fn scaled_rect(
        &self,
        window: &WndWindow,
        id: UiControlId,
        parent_origin: UiPoint,
    ) -> Result<UiRect, UiLayoutError> {
        let stored = window.rect();
        let (creation_width, creation_height) = stored.creation_resolution();
        if creation_width <= 0 || creation_height <= 0 {
            return Err(UiLayoutError::InvalidCreationResolution {
                control: id.0,
                width: creation_width,
                height: creation_height,
            });
        }
        let (left, top) = stored.upper_left();
        let (right, bottom) = stored.bottom_right();
        let (x_scale, y_scale, x_offset, y_offset) = match self.presentation.scale {
            UiScalePolicy::Classic => (
                ratio(self.presentation.viewport.width, creation_width),
                ratio(self.presentation.viewport.height, creation_height),
                0,
                0,
            ),
            UiScalePolicy::Modern => {
                let x_scale = ratio(self.presentation.viewport.width, creation_width);
                let y_scale = ratio(self.presentation.viewport.height, creation_height);
                let uniform = x_scale.min(y_scale);
                let x_offset =
                    (self.presentation.viewport.width - truncate(creation_width, uniform)) / 2;
                let y_offset =
                    (self.presentation.viewport.height - truncate(creation_height, uniform)) / 2;
                (uniform, uniform, x_offset, y_offset)
            }
        };
        let scaled_left = truncate(left, x_scale) + x_offset;
        let scaled_top = truncate(top, y_scale) + y_offset;
        let scaled_right = truncate(right, x_scale) + x_offset;
        let scaled_bottom = truncate(bottom, y_scale) + y_offset;
        Ok(UiRect {
            x: scaled_left - parent_origin.x,
            y: scaled_top - parent_origin.y,
            width: scaled_right - scaled_left,
            height: scaled_bottom - scaled_top,
        })
    }

    fn control_kind(
        &self,
        window: &WndWindow,
        id: UiControlId,
    ) -> (UiControlKind, Vec<UiDiagnostic>) {
        let mut diagnostics = Vec::new();
        let kind = match window.gadget_data() {
            Some(WndGadgetData::Slider(data)) => Self::slider_kind(*data, id, &mut diagnostics),
            Some(WndGadgetData::RadioButtonGroup(group)) => UiControlKind::RadioButton {
                group: *group,
                selected: false,
            },
            Some(WndGadgetData::ListBox(data)) => {
                Self::list_box_kind(data, id, self.limits, &mut diagnostics)
            }
            Some(WndGadgetData::ComboBox(data)) => Self::combo_box_kind(*data, self.limits),
            Some(WndGadgetData::TextEntry(data)) => {
                Self::text_entry_kind(*data, id, self.limits, &mut diagnostics)
            }
            Some(WndGadgetData::TabControl(data)) => Self::tab_control_kind(data),
            Some(WndGadgetData::StaticTextCentered(centered)) => UiControlKind::StaticText {
                centered: *centered,
            },
            None => Self::kind_from_window_type(window.window_type()),
        };
        (kind, diagnostics)
    }

    fn slider_kind(
        data: WndSliderData,
        id: UiControlId,
        diagnostics: &mut Vec<UiDiagnostic>,
    ) -> UiControlKind {
        let (minimum, maximum) = if data.minimum() <= data.maximum() {
            (data.minimum(), data.maximum())
        } else {
            diagnostics.push(UiDiagnostic {
                control: id,
                kind: UiDiagnosticKind::InvertedSliderBounds {
                    minimum: data.minimum(),
                    maximum: data.maximum(),
                },
            });
            (data.maximum(), data.minimum())
        };
        UiControlKind::Slider {
            minimum,
            maximum,
            value: minimum,
        }
    }

    fn list_box_kind(
        data: &WndListBoxData,
        id: UiControlId,
        limits: UiLimits,
        diagnostics: &mut Vec<UiDiagnostic>,
    ) -> UiControlKind {
        let declared = data.length();
        let visible_rows = usize::try_from(declared.max(0)).unwrap_or(0);
        let visible_rows = if visible_rows > limits.max_list_rows {
            diagnostics.push(UiDiagnostic {
                control: id,
                kind: UiDiagnosticKind::ListRowsClamped {
                    declared,
                    applied: limits.max_list_rows,
                },
            });
            limits.max_list_rows
        } else {
            visible_rows
        };
        UiControlKind::ListBox {
            rows: Vec::new(),
            selected: Vec::new(),
            scroll_top: 0,
            visible_rows,
            multi_select: data.multi_select(),
        }
    }

    fn combo_box_kind(data: WndComboBoxData, limits: UiLimits) -> UiControlKind {
        UiControlKind::ComboBox {
            entries: Vec::new(),
            selected: None,
            open: false,
            max_display: usize::try_from(data.maximum_display().max(0)).unwrap_or(0),
            editable: data.is_editable(),
            max_chars: usize::try_from(data.maximum_characters().max(0))
                .unwrap_or(0)
                .min(limits.max_text_length),
        }
    }

    fn text_entry_kind(
        data: WndTextEntryData,
        id: UiControlId,
        limits: UiLimits,
        diagnostics: &mut Vec<UiDiagnostic>,
    ) -> UiControlKind {
        let declared = data.maximum_length();
        let max_length = usize::try_from(declared.max(0)).unwrap_or(0);
        let max_length = if max_length > limits.max_text_length {
            diagnostics.push(UiDiagnostic {
                control: id,
                kind: UiDiagnosticKind::TextLengthClamped {
                    declared,
                    applied: limits.max_text_length,
                },
            });
            limits.max_text_length
        } else {
            max_length
        };
        UiControlKind::TextEntry {
            text: String::new(),
            caret: 0,
            max_length,
            secret: data.secret_text(),
        }
    }

    /// Builds a tab control's retained state, clamping the declared tab count to the source's
    /// fixed pane array rather than trusting the file's own count.
    fn tab_control_kind(data: &WndTabControlData) -> UiControlKind {
        let panes = usize::try_from(data.tab_count().max(0))
            .unwrap_or(0)
            .min(UI_MAX_TABS);
        let mut disabled = [false; UI_MAX_TABS];
        for (slot, declared) in disabled.iter_mut().zip(data.pane_disabled()) {
            *slot = *declared;
        }
        UiControlKind::TabControl {
            active_pane: 0,
            panes,
            geometry: UiTabGeometry {
                orientation: data.tab_orientation(),
                edge: data.tab_edge(),
                width: data.tab_width(),
                height: data.tab_height(),
                count: panes,
                pane_border: data.pane_border(),
                active: 0,
                disabled,
            },
        }
    }

    fn kind_from_window_type(window_type: &str) -> UiControlKind {
        match window_type {
            "PUSHBUTTON" => UiControlKind::PushButton,
            "CHECKBOX" => UiControlKind::CheckBox { checked: false },
            "STATICTEXT" => UiControlKind::StaticText { centered: false },
            "PROGRESSBAR" => UiControlKind::ProgressBar { progress: 0 },
            _ => UiControlKind::Generic,
        }
    }

    /// Returns the presentation this layout was instantiated for.
    #[must_use]
    pub const fn presentation(&self) -> UiPresentation {
        self.presentation
    }

    /// Returns every control in source order.
    #[must_use]
    pub fn controls(&self) -> &[UiControl] {
        &self.controls
    }

    /// Returns top-level controls in source order.
    #[must_use]
    pub fn roots(&self) -> &[UiControlId] {
        &self.roots
    }

    /// Returns the tab-traversal order: every control declaring `TABSTOP`, in source order.
    ///
    /// The original's per-window `winNextTab`/`winPrevTab` are commented out and return success
    /// without moving focus; the live mechanism is the window manager's tab list, which cycles with
    /// wraparound and is inert while a modal is up. This reproduces the manager's behavior and
    /// derives the list from the declared `TABSTOP` bit, which the original populates elsewhere.
    #[must_use]
    pub fn tab_order(&self) -> &[UiControlId] {
        &self.tab_order
    }

    /// Returns every non-fatal instantiation observation, in control order.
    #[must_use]
    pub fn diagnostics(&self) -> &[UiDiagnostic] {
        &self.diagnostics
    }

    /// Returns the retained `LAYOUTINIT` callback name. It is data: nothing here dispatches it.
    #[must_use]
    pub fn layout_init_callback(&self) -> Option<&str> {
        self.layout_init.as_deref()
    }

    /// Returns the retained `LAYOUTUPDATE` callback name. It is data: nothing here dispatches it.
    #[must_use]
    pub fn layout_update_callback(&self) -> Option<&str> {
        self.layout_update.as_deref()
    }

    /// Returns the retained `LAYOUTSHUTDOWN` callback name. It is data: nothing here dispatches it.
    #[must_use]
    pub fn layout_shutdown_callback(&self) -> Option<&str> {
        self.layout_shutdown.as_deref()
    }

    /// Returns whether the whole layout was hidden by [`UiLayout::hide`].
    ///
    /// This is `WindowLayout::isHidden`, the layout's own recorded state rather than a scan of its
    /// windows, and the shell reads it to decide whether a screen still needs shutting down.
    #[must_use]
    pub const fn is_hidden(&self) -> bool {
        self.hidden
    }

    /// Hides or shows every top-level control, and records the layout's own hidden state.
    ///
    /// `WindowLayout::hide` walks the layout's window list — which `winCreateFromScript` fills with
    /// exactly the file's top-level `WINDOW` blocks, in file order — and calls `winHide` on each.
    /// Children follow because a hidden parent hides its subtree.
    pub fn hide(&mut self, hidden: bool) {
        let roots = self.roots.clone();
        for root in roots {
            self.set_hidden(root, hidden);
        }
        self.hidden = hidden;
    }

    /// Returns one control.
    ///
    /// # Panics
    ///
    /// Panics if the id came from a different layout.
    #[must_use]
    pub fn control(&self, id: UiControlId) -> &UiControl {
        &self.controls[id.0]
    }

    /// Returns the first control whose full decorated name or undecorated control part matches,
    /// in source order.
    ///
    /// Both spellings resolve because a patch overlay addresses the decorated form while a caller
    /// working inside one layout usually knows only the control part.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<UiControlId> {
        self.controls
            .iter()
            .find(|control| {
                control.name.as_deref() == Some(name) || control.control_name() == Some(name)
            })
            .map(|control| control.id)
    }

    /// Returns a control's absolute origin, summing every ancestor's parent-relative position.
    #[must_use]
    pub fn screen_origin(&self, id: UiControlId) -> UiPoint {
        let mut x = 0;
        let mut y = 0;
        let mut current = Some(id);
        while let Some(control) = current {
            let control = &self.controls[control.0];
            x += control.rect.x;
            y += control.rect.y;
            current = control.parent;
        }
        UiPoint::new(x, y)
    }

    /// Returns a control's absolute rectangle.
    #[must_use]
    pub fn screen_rect(&self, id: UiControlId) -> UiRect {
        let origin = self.screen_origin(id);
        let control = &self.controls[id.0];
        UiRect {
            x: origin.x,
            y: origin.y,
            width: control.rect.width,
            height: control.rect.height,
        }
    }

    /// Returns whether a control and every ancestor are visible.
    #[must_use]
    pub fn is_effectively_visible(&self, id: UiControlId) -> bool {
        let mut current = Some(id);
        while let Some(control) = current {
            let control = &self.controls[control.0];
            if control.hidden {
                return false;
            }
            current = control.parent;
        }
        true
    }

    /// Returns whether a control and every ancestor are enabled and visible.
    #[must_use]
    pub fn is_effectively_enabled(&self, id: UiControlId) -> bool {
        let mut current = Some(id);
        while let Some(control) = current {
            let control = &self.controls[control.0];
            if control.hidden || !control.enabled {
                return false;
            }
            current = control.parent;
        }
        true
    }

    /// Shows or hides a control. Hiding also clears its hover, press, and focus state, because a
    /// hidden window takes no input.
    pub fn set_hidden(&mut self, id: UiControlId, hidden: bool) {
        self.controls[id.0].hidden = hidden;
        if hidden {
            self.clear_interaction(id);
        }
    }

    /// Enables or disables a control. Disabling clears its hover, press, and focus state.
    pub fn set_enabled(&mut self, id: UiControlId, enabled: bool) {
        self.controls[id.0].enabled = enabled;
        if !enabled {
            self.clear_interaction(id);
        }
    }

    fn clear_interaction(&mut self, id: UiControlId) {
        self.controls[id.0].hovered = false;
        self.controls[id.0].pressed = false;
        if self.focus == Some(id) {
            self.focus = None;
        }
        if self.capture == Some(id) {
            self.capture = None;
        }
        let children = self.controls[id.0].children.clone();
        for child in children {
            self.clear_interaction(child);
        }
    }

    /// Replaces a control's displayed label.
    ///
    /// This is `GadgetStaticTextSetText`, which the count-up transition uses to walk a score value
    /// upwards. It writes the control's own retained text, so the change outlives the transition
    /// exactly as it does in the original.
    pub fn set_text_label(&mut self, id: UiControlId, text: impl Into<String>) {
        self.controls[id.0].text_label = Some(text.into());
    }

    /// Returns the control holding keyboard focus.
    #[must_use]
    pub const fn focus(&self) -> Option<UiControlId> {
        self.focus
    }

    /// Returns the control holding the mouse, set while a press is in progress.
    #[must_use]
    pub const fn capture(&self) -> Option<UiControlId> {
        self.capture
    }

    pub(crate) fn set_capture(&mut self, id: Option<UiControlId>) {
        self.capture = id;
    }

    pub(crate) fn control_mut(&mut self, id: UiControlId) -> &mut UiControl {
        &mut self.controls[id.0]
    }

    pub(crate) fn set_focus_field(&mut self, id: Option<UiControlId>) {
        self.focus = id;
    }

    /// Selects one radio button, clearing every other button in its group.
    ///
    /// `GadgetRadioButtonSetSelection` walks the owning window's peers and clears any radio button
    /// sharing the group, so exactly one button in a group is selected. Reproduced here over the
    /// control's siblings, which is the same peer set.
    pub fn select_radio(&mut self, id: UiControlId) {
        let Some(group) = self.radio_group(id) else {
            return;
        };
        let peers = self.siblings(id);
        for peer in peers {
            if let UiControlKind::RadioButton {
                group: peer_group,
                selected,
            } = &mut self.controls[peer.0].kind
                && *peer_group == group
            {
                *selected = peer == id;
            }
        }
    }

    fn radio_group(&self, id: UiControlId) -> Option<i32> {
        match self.controls[id.0].kind {
            UiControlKind::RadioButton { group, .. } => Some(group),
            _ => None,
        }
    }

    fn siblings(&self, id: UiControlId) -> Vec<UiControlId> {
        match self.controls[id.0].parent {
            Some(parent) => self.controls[parent.0].children.clone(),
            None => self.roots.clone(),
        }
    }

    /// Toggles a check box and returns its new state.
    pub fn toggle_check(&mut self, id: UiControlId) -> Option<bool> {
        match &mut self.controls[id.0].kind {
            UiControlKind::CheckBox { checked } => {
                *checked = !*checked;
                Some(*checked)
            }
            _ => None,
        }
    }

    /// Sets a slider's value, clamped into its declared bounds, and returns the applied value.
    pub fn set_slider_value(&mut self, id: UiControlId, requested: i32) -> Option<i32> {
        match &mut self.controls[id.0].kind {
            UiControlKind::Slider {
                minimum,
                maximum,
                value,
            } => {
                *value = requested.clamp(*minimum, *maximum);
                Some(*value)
            }
            _ => None,
        }
    }

    /// Sets a progress bar's percentage, clamped to `0..=100`.
    pub fn set_progress(&mut self, id: UiControlId, requested: i32) -> Option<i32> {
        match &mut self.controls[id.0].kind {
            UiControlKind::ProgressBar { progress } => {
                *progress = requested.clamp(0, 100);
                Some(*progress)
            }
            _ => None,
        }
    }

    /// Appends a row to a list box, bounded by [`UiLimits::max_list_rows`].
    pub fn push_list_row(&mut self, id: UiControlId, row: impl Into<String>) -> bool {
        let limit = self.limits.max_list_rows;
        match &mut self.controls[id.0].kind {
            UiControlKind::ListBox { rows, .. } if rows.len() < limit => {
                rows.push(row.into());
                true
            }
            _ => false,
        }
    }

    /// Selects one list row, replacing the selection unless the list allows multi-select.
    ///
    /// A row outside the current row count is refused rather than clamped, so a stale index cannot
    /// silently select a different row.
    pub fn select_list_row(&mut self, id: UiControlId, row: usize, additive: bool) -> bool {
        match &mut self.controls[id.0].kind {
            UiControlKind::ListBox {
                rows,
                selected,
                multi_select,
                ..
            } => {
                if row >= rows.len() {
                    return false;
                }
                if additive && *multi_select {
                    if let Err(position) = selected.binary_search(&row) {
                        selected.insert(position, row);
                    }
                } else {
                    selected.clear();
                    selected.push(row);
                }
                true
            }
            _ => false,
        }
    }

    /// Scrolls a list box so `top` is the first visible row, clamped so the last page stays full.
    pub fn scroll_list(&mut self, id: UiControlId, top: usize) -> Option<usize> {
        match &mut self.controls[id.0].kind {
            UiControlKind::ListBox {
                rows,
                scroll_top,
                visible_rows,
                ..
            } => {
                let last = rows.len().saturating_sub((*visible_rows).max(1));
                *scroll_top = top.min(last);
                Some(*scroll_top)
            }
            _ => None,
        }
    }

    /// Appends an entry to a combo box.
    pub fn push_combo_entry(&mut self, id: UiControlId, entry: impl Into<String>) -> bool {
        let limit = self.limits.max_list_rows;
        match &mut self.controls[id.0].kind {
            UiControlKind::ComboBox { entries, .. } if entries.len() < limit => {
                entries.push(entry.into());
                true
            }
            _ => false,
        }
    }

    /// Replaces a combo box's entries and clears its selection.
    ///
    /// A WND declares no entries for a combo box: retail's own menu code fills one at runtime with
    /// `GadgetComboBoxAddEntry` — `OptionsMenuInit` populates the resolution combo exactly this way —
    /// so an application supplying a list is reproducing the original's arrangement, not working
    /// around a gap in the format. The selection is cleared because the old index means nothing
    /// against a new list.
    ///
    /// Returns whether the control is a combo box.
    pub fn set_combo_entries(&mut self, id: UiControlId, values: Vec<String>) -> bool {
        match &mut self.controls[id.0].kind {
            UiControlKind::ComboBox {
                entries, selected, ..
            } => {
                *entries = values;
                *selected = None;
                true
            }
            _ => false,
        }
    }

    /// Returns a combo box's entries, empty when the control is not one.
    #[must_use]
    pub fn combo_entries(&self, id: UiControlId) -> &[String] {
        match &self.controls[id.0].kind {
            UiControlKind::ComboBox { entries, .. } => entries,
            _ => &[],
        }
    }

    /// Selects one combo entry, refusing an index outside the entry list.
    pub fn select_combo_entry(&mut self, id: UiControlId, entry: usize) -> bool {
        match &mut self.controls[id.0].kind {
            UiControlKind::ComboBox {
                entries, selected, ..
            } => {
                if entry >= entries.len() {
                    return false;
                }
                *selected = Some(entry);
                true
            }
            _ => false,
        }
    }

    /// Opens or closes a combo box's drop-down.
    pub fn set_combo_open(&mut self, id: UiControlId, open: bool) -> bool {
        match &mut self.controls[id.0].kind {
            UiControlKind::ComboBox {
                open: current_open, ..
            } => {
                *current_open = open;
                true
            }
            _ => false,
        }
    }

    /// Selects a tab control's active pane, refusing an index outside the declared panes.
    pub fn select_tab_pane(&mut self, id: UiControlId, pane: usize) -> bool {
        match &mut self.controls[id.0].kind {
            UiControlKind::TabControl {
                active_pane,
                panes,
                geometry,
            } => {
                if pane >= *panes {
                    return false;
                }
                *active_pane = pane;
                // The strip's own active index is what the presentation layer reads to pick the
                // hilited tab image, so it moves with the retained pane.
                geometry.active = pane;
                true
            }
            _ => false,
        }
    }

    /// Returns the set of decorated names declared more than once, which a patch overlay or a
    /// caller addressing controls by name needs to know about.
    #[must_use]
    pub fn duplicate_names(&self) -> BTreeSet<&str> {
        let mut seen = BTreeSet::new();
        let mut duplicates = BTreeSet::new();
        for control in &self.controls {
            if let Some(name) = control.control_name()
                && !seen.insert(name)
            {
                duplicates.insert(name);
            }
        }
        duplicates
    }

    pub(crate) const fn limits(&self) -> UiLimits {
        self.limits
    }
}

/// Returns the scale ratio the original computes, as a 32-bit float.
fn ratio(viewport: i32, creation: i32) -> f32 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "both operands are pixel counts well inside f32's exact integer range; the source \
                  performs the same division in a 32-bit float"
    )]
    let value = viewport as f32 / creation as f32;
    value
}

/// Scales one coordinate and truncates toward zero, matching the source's `(Int)` cast.
fn truncate(coordinate: i32, scale: f32) -> i32 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "stored coordinates are small pixel counts"
    )]
    let scaled = coordinate as f32 * scale;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "reproducing the source's truncating (Int) cast is the point; the saturating cast \
                  additionally makes an absurd scale factor bounded instead of undefined"
    )]
    let truncated = scaled as i32;
    truncated
}
