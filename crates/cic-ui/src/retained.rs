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
    WndColor, WndComboBoxData, WndDocument, WndDrawData, WndDrawDataSlot, WndDrawEntry,
    WndGadgetData, WndListBoxData, WndSliderData, WndTextColors, WndTextEntryData, WndWindow,
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
    /// `WIN_STATUS_ON_MOUSE_DOWN`.
    pub const ON_MOUSE_DOWN: Self = Self(0x0200_0000);

    /// Every name this crate maps, paired with its bit.
    const NAMES: [(&'static str, Self); 19] = [
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
    StaticText,
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
    draw_data: Vec<(WndDrawDataSlot, WndDrawData)>,
    text_colors: Option<WndTextColors>,
    kind: UiControlKind,
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

    /// Returns the family-specific retained state.
    #[must_use]
    pub const fn kind(&self) -> &UiControlKind {
        &self.kind
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
        let mut layout = Self {
            controls: Vec::new(),
            roots: Vec::new(),
            tab_order: Vec::new(),
            presentation,
            limits,
            focus: None,
            capture: None,
            diagnostics: Vec::new(),
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
            status,
            hidden: status.contains(UiStatus::HIDDEN),
            enabled: status.contains(UiStatus::ENABLED),
            hovered: false,
            pressed: false,
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
            draw_data: window.draw_data().to_vec(),
            text_colors: window.text_colors().copied(),
            kind,
        });
        if let Some(parent) = parent {
            self.controls[parent.0].children.push(id);
        }
        for child in window.children() {
            self.add(child, Some(id), depth + 1)?;
        }
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
            Some(WndGadgetData::ComboBox(data)) => Self::combo_box_kind(*data),
            Some(WndGadgetData::TextEntry(data)) => {
                Self::text_entry_kind(*data, id, self.limits, &mut diagnostics)
            }
            Some(WndGadgetData::TabControl(data)) => UiControlKind::TabControl {
                active_pane: 0,
                panes: usize::try_from(data.tab_count().max(0)).unwrap_or(0),
            },
            Some(WndGadgetData::StaticTextCentered(_)) => UiControlKind::StaticText,
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

    fn combo_box_kind(data: WndComboBoxData) -> UiControlKind {
        UiControlKind::ComboBox {
            entries: Vec::new(),
            selected: None,
            open: false,
            max_display: usize::try_from(data.maximum_display().max(0)).unwrap_or(0),
            editable: data.is_editable(),
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

    fn kind_from_window_type(window_type: &str) -> UiControlKind {
        match window_type {
            "PUSHBUTTON" => UiControlKind::PushButton,
            "CHECKBOX" => UiControlKind::CheckBox { checked: false },
            "STATICTEXT" => UiControlKind::StaticText,
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
            UiControlKind::TabControl { active_pane, panes } => {
                if pane >= *panes {
                    return false;
                }
                *active_pane = pane;
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
