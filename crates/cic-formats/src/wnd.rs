// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Bounded immutable decoder for the WND UI-layout text format.
//!
//! Grammar facts (no comment syntax, `;` as a hard statement terminator rather than a
//! comment marker, case-sensitive structural keywords versus case-insensitive status/style
//! names, double-quote-delimited strings with no escapes, decimal-only numbers, and the
//! `STARTLAYOUTBLOCK`/`ENDLAYOUTBLOCK`/`LAYOUTINIT`/`LAYOUTUPDATE`/`LAYOUTSHUTDOWN`,
//! `WINDOW`/`CHILD`/`END`/`ENDALLCHILDREN`, `WINDOWTYPE`, and `SCREENRECT` vocabulary) are
//! derived from `winCreateFromScript` and `parseLayoutBlock` in
//! `Generals/Code/GameEngine/Source/GameClient/GUI/GameWindowManagerScript.cpp` at
//! `GeneralsGameCode` revision `9f7abb866f5afd446db14149979e744c7216baaf`, licensed under
//! GPL-3.0-or-later with Electronic Arts Section 7 terms. Full notices are recorded in
//! `docs/provenance/wnd.md`.
//!
//! This decoder treats WND as untrusted declarative data: callback names are retained as
//! opaque strings and never resolved or invoked. Unlike this crate's other text/INI
//! decoders (`road_ini`, `water_ini`, `terrain_ini`, `object_ini`), which silently ignore
//! fields they do not recognize, this decoder never drops a field: every window field is
//! retained in [`WndWindow::fields`] whether or not its name is recognized, and unrecognized
//! top-level keywords or out-of-vocabulary values are additionally recorded as a
//! [`WndDiagnostic`] so unsupported or missing functionality stays discoverable instead of
//! disappearing silently.
//!
//! `CHILD` is an optional marker rather than a required prefix. The source's child-list
//! loop (`parseChildWindows`) dispatches only on `ENDALLCHILDREN`, `END`, the default-color
//! keywords, and `WINDOW`, with no `CHILD` case and no fallback branch, so a `CHILD` token
//! is simply skipped and a bare `WINDOW` starts the next sibling. Retail data writes
//! `CHILD` before every child but one, so this decoder accepts either spelling inside an
//! open child list and reports the unmarked form as a [`WndDiagnostic`].
//!
//! This first slice covers the source-established file/layout header and the `WINDOW`/
//! `CHILD` hierarchy with `WINDOWTYPE`/`SCREENRECT` typed and every other field preserved
//! generically. Per-gadget typed field decode (fonts, state colors/borders, draw-data
//! arrays, header templates, gadget-specific `DATA`) is deliberately excluded here and
//! belongs to a later slice, alongside mapped-image/font/CSF resource resolution.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

/// Explicit input and allocation bounds for [`parse_wnd`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WndLimits {
    /// Maximum complete input length.
    pub maximum_file_bytes: usize,
    /// Maximum lexical tokens read across the whole document.
    pub maximum_tokens: usize,
    /// Maximum physical lines (`\n` count) before the document is rejected.
    pub maximum_lines: usize,
    /// Maximum bytes in one semicolon-terminated value record.
    pub maximum_record_bytes: usize,
    /// Maximum bytes in one bare or quoted token.
    pub maximum_field_bytes: usize,
    /// Maximum retained tokens in one semicolon-terminated record. Bounds the per-record
    /// token vector independently of [`WndLimits::maximum_record_bytes`], which alone would
    /// permit a 65,536-token allocation from a single record.
    pub maximum_record_tokens: usize,
    /// Maximum `WINDOW` blocks (including nested children) in one document.
    pub maximum_windows: usize,
    /// Maximum `CHILD` nesting depth.
    pub maximum_depth: usize,
}

impl Default for WndLimits {
    fn default() -> Self {
        Self {
            maximum_file_bytes: 8 * 1024 * 1024,
            maximum_tokens: 262_144,
            maximum_lines: 65_536,
            maximum_record_bytes: 65_536,
            maximum_field_bytes: 4_096,
            maximum_record_tokens: 4_096,
            maximum_windows: 16_384,
            maximum_depth: 256,
        }
    }
}

/// One retained, unresolved layout init/update/shutdown callback name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WndLayoutBlock {
    init: Option<Box<str>>,
    update: Option<Box<str>>,
    shutdown: Option<Box<str>>,
}

impl WndLayoutBlock {
    /// Returns the raw `LAYOUTINIT` callback name, if present.
    #[must_use]
    pub fn init(&self) -> Option<&str> {
        self.init.as_deref()
    }

    /// Returns the raw `LAYOUTUPDATE` callback name, if present.
    #[must_use]
    pub fn update(&self) -> Option<&str> {
        self.update.as_deref()
    }

    /// Returns the raw `LAYOUTSHUTDOWN` callback name, if present.
    #[must_use]
    pub fn shutdown(&self) -> Option<&str> {
        self.shutdown.as_deref()
    }
}

/// A window's stored creation rectangle and creation resolution, exactly as authored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WndScreenRect {
    upper_left: (i32, i32),
    bottom_right: (i32, i32),
    creation_resolution: (i32, i32),
}

impl WndScreenRect {
    /// Builds a rectangle from explicit corners and creation resolution, for a patch
    /// overlay that repositions a control.
    pub(crate) const fn new(
        upper_left: (i32, i32),
        bottom_right: (i32, i32),
        creation_resolution: (i32, i32),
    ) -> Self {
        Self {
            upper_left,
            bottom_right,
            creation_resolution,
        }
    }

    /// Returns the stored upper-left corner.
    #[must_use]
    pub const fn upper_left(&self) -> (i32, i32) {
        self.upper_left
    }

    /// Returns the stored bottom-right corner.
    #[must_use]
    pub const fn bottom_right(&self) -> (i32, i32) {
        self.bottom_right
    }

    /// Returns the stored creation resolution.
    #[must_use]
    pub const fn creation_resolution(&self) -> (i32, i32) {
        self.creation_resolution
    }
}

/// How one lexical token inside a record was spelled in the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WndTokenKind {
    /// A bare token delimited by whitespace or punctuation.
    Word,
    /// A double-quote-delimited token. The quotes are not part of [`WndToken::text`], and
    /// the source has no escape syntax, so the text is exactly the enclosed bytes.
    Quoted,
    /// One of the single-character record delimiters `,`, `:`, or `+`.
    Punctuation,
}

/// One lexical token inside a record value.
///
/// Records are retained as a token sequence rather than only as a flattened string because
/// flattening is lossy in ways later gates cannot recover: `FONT = NAME: "Times New Roman",
/// SIZE: 14, BOLD: 0;` and the same record written without quotes collapse to the same
/// characters, so a font name containing spaces becomes impossible to delimit. The token
/// sequence keeps quoting and punctuation explicit; [`WndField::raw_value`] keeps the
/// verbatim source text alongside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WndToken {
    text: Box<str>,
    kind: WndTokenKind,
    line: usize,
}

impl WndToken {
    /// Returns the token text, without surrounding quotes for a [`WndTokenKind::Quoted`]
    /// token.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns how the token was spelled.
    #[must_use]
    pub const fn kind(&self) -> WndTokenKind {
        self.kind
    }

    /// Returns whether the token was double-quote-delimited in the source.
    #[must_use]
    pub const fn is_quoted(&self) -> bool {
        matches!(self.kind, WndTokenKind::Quoted)
    }

    /// Returns whether the token is a `,`, `:`, or `+` record delimiter.
    #[must_use]
    pub const fn is_punctuation(&self) -> bool {
        matches!(self.kind, WndTokenKind::Punctuation)
    }

    /// Returns the one-based source line the token started on.
    #[must_use]
    pub const fn line(&self) -> usize {
        self.line
    }
}

/// One generically retained `NAME = value;` field, preserved whether or not it is
/// recognized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WndField {
    name: Box<str>,
    raw_value: Box<str>,
    tokens: Vec<WndToken>,
    line: usize,
}

impl WndField {
    /// Returns the field name exactly as spelled in the source.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the field's value as verbatim source text with whitespace runs collapsed to
    /// single spaces and the ends trimmed. Quotes and punctuation are preserved exactly as
    /// authored; use [`WndField::tokens`] for a structured view.
    #[must_use]
    pub fn raw_value(&self) -> &str {
        &self.raw_value
    }

    /// Returns the record's tokens in source order, with quoting and punctuation retained.
    #[must_use]
    pub fn tokens(&self) -> &[WndToken] {
        &self.tokens
    }

    /// Returns the one-based source line where the field name appeared.
    #[must_use]
    pub const fn line(&self) -> usize {
        self.line
    }
}

/// One RGBA color, preserving the exact channel bytes the source authored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WndColor {
    red: u8,
    green: u8,
    blue: u8,
    alpha: u8,
}

impl WndColor {
    /// Builds a colour from explicit channel bytes.
    ///
    /// A decoded colour comes from a `COLOR:`/`BORDERCOLOR:` record; this exists for the project-owned
    /// callers that compose one, such as a transition drawing a stand-in at a computed alpha.
    #[must_use]
    pub const fn from_rgba(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }

    /// Returns the channel bytes in `R`, `G`, `B`, `A` order.
    #[must_use]
    pub const fn channels(self) -> [u8; 4] {
        [self.red, self.green, self.blue, self.alpha]
    }

    /// Returns whether this is the source's "no colour here" sentinel.
    ///
    /// `Color.h` defines `GAME_COLOR_UNDEFINED = 0x00FFFFFF` and `GameMakeColor` packs `ARGB`, so
    /// the sentinel is exactly white with zero alpha. Every draw procedure compares against it
    /// rather than testing alpha, and retail writes it into every unused draw-data entry, which is
    /// why unread slots read `COLOR: 255 255 255 0, BORDERCOLOR: 255 255 255 0`.
    #[must_use]
    pub const fn is_undefined(self) -> bool {
        self.red == 255 && self.green == 255 && self.blue == 255 && self.alpha == 0
    }
}

/// A decoded `FONT = NAME: "<name>", SIZE: <size>, BOLD: <flag>;` record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WndFont {
    name: Box<str>,
    size: i32,
    bold: bool,
}

impl WndFont {
    /// Returns the font name with its source quoting removed.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the authored point size.
    #[must_use]
    pub const fn size(&self) -> i32 {
        self.size
    }

    /// Returns whether the bold flag was non-zero.
    #[must_use]
    pub const fn bold(&self) -> bool {
        self.bold
    }
}

/// The six state colors of a `TEXTCOLOR` record, in source order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WndTextColors {
    enabled: WndColor,
    enabled_border: WndColor,
    disabled: WndColor,
    disabled_border: WndColor,
    hilite: WndColor,
    hilite_border: WndColor,
}

impl WndTextColors {
    /// Returns the `ENABLED` color.
    #[must_use]
    pub const fn enabled(self) -> WndColor {
        self.enabled
    }

    /// Returns the `ENABLEDBORDER` color.
    #[must_use]
    pub const fn enabled_border(self) -> WndColor {
        self.enabled_border
    }

    /// Returns the `DISABLED` color.
    #[must_use]
    pub const fn disabled(self) -> WndColor {
        self.disabled
    }

    /// Returns the `DISABLEDBORDER` color.
    #[must_use]
    pub const fn disabled_border(self) -> WndColor {
        self.disabled_border
    }

    /// Returns the `HILITE` color.
    #[must_use]
    pub const fn hilite(self) -> WndColor {
        self.hilite
    }

    /// Returns the `HILITEBORDER` color.
    #[must_use]
    pub const fn hilite_border(self) -> WndColor {
        self.hilite_border
    }
}

/// One `STATUS` or `STYLE` flag name, retained with its exact source spelling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WndFlag {
    name: Box<str>,
    known: bool,
}

impl WndFlag {
    /// Returns the flag name exactly as spelled, including its original letter case.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns whether the name is in the established vocabulary for its field.
    #[must_use]
    pub const fn is_known(&self) -> bool {
        self.known
    }
}

/// Which callback slot a retained callback name came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WndCallbackKind {
    /// `SYSTEMCALLBACK`.
    System,
    /// `INPUTCALLBACK`.
    Input,
    /// `TOOLTIPCALLBACK`.
    Tooltip,
    /// `DRAWCALLBACK`.
    Draw,
}

/// The four retained callback names.
///
/// These are opaque data. The legacy runtime resolved them to native function pointers;
/// this project never does, and an application maps them to typed events through an
/// allowlist. The overwhelmingly common authored value is the literal `[None]`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WndCallbacks {
    system: Option<Box<str>>,
    input: Option<Box<str>>,
    tooltip: Option<Box<str>>,
    draw: Option<Box<str>>,
}

impl WndCallbacks {
    /// Returns the raw name in `kind`'s slot, if the window declared one.
    #[must_use]
    pub fn get(&self, kind: WndCallbackKind) -> Option<&str> {
        match kind {
            WndCallbackKind::System => self.system.as_deref(),
            WndCallbackKind::Input => self.input.as_deref(),
            WndCallbackKind::Tooltip => self.tooltip.as_deref(),
            WndCallbackKind::Draw => self.draw.as_deref(),
        }
    }

    fn slot(&mut self, kind: WndCallbackKind) -> &mut Option<Box<str>> {
        match kind {
            WndCallbackKind::System => &mut self.system,
            WndCallbackKind::Input => &mut self.input,
            WndCallbackKind::Tooltip => &mut self.tooltip,
            WndCallbackKind::Draw => &mut self.draw,
        }
    }
}

/// Number of entries in every draw-data array.
///
/// Fixed by the format: the source's `parseDrawData` loops exactly `MAX_DRAW_DATA` times,
/// and `Gadget.h` pins that constant through `NUM_TAB_PANES = 8, //(MAX_DRAW_DATA - 1)`.
/// All 7,875 draw-data records across both retail editions carry exactly nine entries, so
/// this is a required count rather than a configurable limit.
pub const WND_DRAW_DATA_ENTRIES: usize = 9;

/// One `IMAGE`/`COLOR`/`BORDERCOLOR` triple inside a draw-data array.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WndDrawEntry {
    image: Option<Box<str>>,
    color: WndColor,
    border_color: WndColor,
}

impl WndDrawEntry {
    /// Returns the mapped-image name, or `None` for the source's `NoImage` sentinel.
    #[must_use]
    pub fn image(&self) -> Option<&str> {
        self.image.as_deref()
    }

    /// Returns the fill color.
    #[must_use]
    pub const fn color(&self) -> WndColor {
        self.color
    }

    /// Returns the border color.
    #[must_use]
    pub const fn border_color(&self) -> WndColor {
        self.border_color
    }
}

/// Which visual slot a draw-data array describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WndDrawDataSlot {
    /// `ENABLEDDRAWDATA`.
    Enabled,
    /// `DISABLEDDRAWDATA`.
    Disabled,
    /// `HILITEDRAWDATA`.
    Hilite,
    /// `LISTBOXENABLEDUPBUTTONDRAWDATA`.
    ListBoxEnabledUpButton,
    /// `LISTBOXDISABLEDUPBUTTONDRAWDATA`.
    ListBoxDisabledUpButton,
    /// `LISTBOXHILITEUPBUTTONDRAWDATA`.
    ListBoxHiliteUpButton,
    /// `LISTBOXENABLEDDOWNBUTTONDRAWDATA`.
    ListBoxEnabledDownButton,
    /// `LISTBOXDISABLEDDOWNBUTTONDRAWDATA`.
    ListBoxDisabledDownButton,
    /// `LISTBOXHILITEDOWNBUTTONDRAWDATA`.
    ListBoxHiliteDownButton,
    /// `LISTBOXENABLEDSLIDERDRAWDATA`.
    ListBoxEnabledSlider,
    /// `LISTBOXDISABLEDSLIDERDRAWDATA`.
    ListBoxDisabledSlider,
    /// `LISTBOXHILITESLIDERDRAWDATA`.
    ListBoxHiliteSlider,
    /// `SLIDERTHUMBENABLEDDRAWDATA`.
    SliderThumbEnabled,
    /// `SLIDERTHUMBDISABLEDDRAWDATA`.
    SliderThumbDisabled,
    /// `SLIDERTHUMBHILITEDRAWDATA`.
    SliderThumbHilite,
    /// `COMBOBOXDROPDOWNBUTTONENABLEDDRAWDATA`.
    ComboBoxDropDownButtonEnabled,
    /// `COMBOBOXDROPDOWNBUTTONDISABLEDDRAWDATA`.
    ComboBoxDropDownButtonDisabled,
    /// `COMBOBOXDROPDOWNBUTTONHILITEDRAWDATA`.
    ComboBoxDropDownButtonHilite,
    /// `COMBOBOXEDITBOXENABLEDDRAWDATA`.
    ComboBoxEditBoxEnabled,
    /// `COMBOBOXEDITBOXDISABLEDDRAWDATA`.
    ComboBoxEditBoxDisabled,
    /// `COMBOBOXEDITBOXHILITEDRAWDATA`.
    ComboBoxEditBoxHilite,
    /// `COMBOBOXLISTBOXENABLEDDRAWDATA`.
    ComboBoxListBoxEnabled,
    /// `COMBOBOXLISTBOXDISABLEDDRAWDATA`.
    ComboBoxListBoxDisabled,
    /// `COMBOBOXLISTBOXHILITEDRAWDATA`.
    ComboBoxListBoxHilite,
}

/// The 21 draw-data keywords `parseDrawData` dispatches on, with their slots.
const DRAW_DATA_SLOTS: [(&str, WndDrawDataSlot); 24] = [
    ("ENABLEDDRAWDATA", WndDrawDataSlot::Enabled),
    ("DISABLEDDRAWDATA", WndDrawDataSlot::Disabled),
    ("HILITEDRAWDATA", WndDrawDataSlot::Hilite),
    (
        "LISTBOXENABLEDUPBUTTONDRAWDATA",
        WndDrawDataSlot::ListBoxEnabledUpButton,
    ),
    (
        "LISTBOXDISABLEDUPBUTTONDRAWDATA",
        WndDrawDataSlot::ListBoxDisabledUpButton,
    ),
    (
        "LISTBOXHILITEUPBUTTONDRAWDATA",
        WndDrawDataSlot::ListBoxHiliteUpButton,
    ),
    (
        "LISTBOXENABLEDDOWNBUTTONDRAWDATA",
        WndDrawDataSlot::ListBoxEnabledDownButton,
    ),
    (
        "LISTBOXDISABLEDDOWNBUTTONDRAWDATA",
        WndDrawDataSlot::ListBoxDisabledDownButton,
    ),
    (
        "LISTBOXHILITEDOWNBUTTONDRAWDATA",
        WndDrawDataSlot::ListBoxHiliteDownButton,
    ),
    (
        "LISTBOXENABLEDSLIDERDRAWDATA",
        WndDrawDataSlot::ListBoxEnabledSlider,
    ),
    (
        "LISTBOXDISABLEDSLIDERDRAWDATA",
        WndDrawDataSlot::ListBoxDisabledSlider,
    ),
    (
        "LISTBOXHILITESLIDERDRAWDATA",
        WndDrawDataSlot::ListBoxHiliteSlider,
    ),
    (
        "SLIDERTHUMBENABLEDDRAWDATA",
        WndDrawDataSlot::SliderThumbEnabled,
    ),
    (
        "SLIDERTHUMBDISABLEDDRAWDATA",
        WndDrawDataSlot::SliderThumbDisabled,
    ),
    (
        "SLIDERTHUMBHILITEDRAWDATA",
        WndDrawDataSlot::SliderThumbHilite,
    ),
    (
        "COMBOBOXDROPDOWNBUTTONENABLEDDRAWDATA",
        WndDrawDataSlot::ComboBoxDropDownButtonEnabled,
    ),
    (
        "COMBOBOXDROPDOWNBUTTONDISABLEDDRAWDATA",
        WndDrawDataSlot::ComboBoxDropDownButtonDisabled,
    ),
    (
        "COMBOBOXDROPDOWNBUTTONHILITEDRAWDATA",
        WndDrawDataSlot::ComboBoxDropDownButtonHilite,
    ),
    (
        "COMBOBOXEDITBOXENABLEDDRAWDATA",
        WndDrawDataSlot::ComboBoxEditBoxEnabled,
    ),
    (
        "COMBOBOXEDITBOXDISABLEDDRAWDATA",
        WndDrawDataSlot::ComboBoxEditBoxDisabled,
    ),
    (
        "COMBOBOXEDITBOXHILITEDRAWDATA",
        WndDrawDataSlot::ComboBoxEditBoxHilite,
    ),
    (
        "COMBOBOXLISTBOXENABLEDDRAWDATA",
        WndDrawDataSlot::ComboBoxListBoxEnabled,
    ),
    (
        "COMBOBOXLISTBOXDISABLEDDRAWDATA",
        WndDrawDataSlot::ComboBoxListBoxDisabled,
    ),
    (
        "COMBOBOXLISTBOXHILITEDRAWDATA",
        WndDrawDataSlot::ComboBoxListBoxHilite,
    ),
];

/// A decoded `SCROLLLISTBOX` `LISTBOXDATA` record.
///
/// The boolean count mirrors the source record exactly. These are independent persisted
/// flags, not states of one machine, so collapsing them into enums would invent structure
/// the format does not have.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WndListBoxData {
    length: i32,
    auto_scroll: bool,
    /// Absent when the optional `SCROLLIFATEND` sub-record is omitted, which the source
    /// treats as `false`.
    scroll_if_at_end: Option<bool>,
    auto_purge: bool,
    scroll_bar: bool,
    multi_select: bool,
    columns: i32,
    column_widths: Vec<i32>,
    force_select: bool,
}

impl WndListBoxData {
    /// Returns the `LENGTH` value.
    #[must_use]
    pub const fn length(&self) -> i32 {
        self.length
    }

    /// Returns `AUTOSCROLL`.
    #[must_use]
    pub const fn auto_scroll(&self) -> bool {
        self.auto_scroll
    }

    /// Returns `SCROLLIFATEND`, or `None` when the optional sub-record was omitted. The
    /// source defaults the omitted case to `false`.
    #[must_use]
    pub const fn scroll_if_at_end(&self) -> Option<bool> {
        self.scroll_if_at_end
    }

    /// Returns `AUTOPURGE`.
    #[must_use]
    pub const fn auto_purge(&self) -> bool {
        self.auto_purge
    }

    /// Returns `SCROLLBAR`.
    #[must_use]
    pub const fn scroll_bar(&self) -> bool {
        self.scroll_bar
    }

    /// Returns `MULTISELECT`.
    #[must_use]
    pub const fn multi_select(&self) -> bool {
        self.multi_select
    }

    /// Returns the declared `COLUMNS` count.
    #[must_use]
    pub const fn columns(&self) -> i32 {
        self.columns
    }

    /// Returns the per-column width percentages. The source reads these only when
    /// `COLUMNS` exceeds one, so a single-column list has none.
    #[must_use]
    pub fn column_widths(&self) -> &[i32] {
        &self.column_widths
    }

    /// Returns `FORCESELECT`.
    #[must_use]
    pub const fn force_select(&self) -> bool {
        self.force_select
    }
}

/// A decoded `COMBOBOXDATA` record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WndComboBoxData {
    is_editable: bool,
    maximum_characters: i32,
    maximum_display: i32,
    ascii_only: bool,
    letters_and_numbers_only: bool,
}

impl WndComboBoxData {
    /// Returns `ISEDITABLE`.
    #[must_use]
    pub const fn is_editable(self) -> bool {
        self.is_editable
    }

    /// Returns `MAXCHARS`.
    #[must_use]
    pub const fn maximum_characters(self) -> i32 {
        self.maximum_characters
    }

    /// Returns `MAXDISPLAY`.
    #[must_use]
    pub const fn maximum_display(self) -> i32 {
        self.maximum_display
    }

    /// Returns `ASCIIONLY`.
    #[must_use]
    pub const fn ascii_only(self) -> bool {
        self.ascii_only
    }

    /// Returns `LETTERSANDNUMBERS`.
    #[must_use]
    pub const fn letters_and_numbers_only(self) -> bool {
        self.letters_and_numbers_only
    }
}

/// A decoded `SLIDERDATA` record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WndSliderData {
    minimum: i32,
    maximum: i32,
}

impl WndSliderData {
    /// Returns `MINVALUE`.
    #[must_use]
    pub const fn minimum(self) -> i32 {
        self.minimum
    }

    /// Returns `MAXVALUE`.
    #[must_use]
    pub const fn maximum(self) -> i32 {
        self.maximum
    }
}

/// A decoded `TEXTENTRYDATA` record.
///
/// As with [`WndListBoxData`], the boolean count mirrors the source record's independent
/// persisted flags.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WndTextEntryData {
    maximum_length: i32,
    secret_text: bool,
    numerical_only: bool,
    alphanumerical_only: bool,
    ascii_only: bool,
}

impl WndTextEntryData {
    /// Returns `MAXLEN`.
    #[must_use]
    pub const fn maximum_length(self) -> i32 {
        self.maximum_length
    }

    /// Returns `SECRETTEXT`.
    #[must_use]
    pub const fn secret_text(self) -> bool {
        self.secret_text
    }

    /// Returns `NUMERICALONLY`.
    #[must_use]
    pub const fn numerical_only(self) -> bool {
        self.numerical_only
    }

    /// Returns `ALPHANUMERICALONLY`.
    #[must_use]
    pub const fn alphanumerical_only(self) -> bool {
        self.alphanumerical_only
    }

    /// Returns `ASCIIONLY`.
    #[must_use]
    pub const fn ascii_only(self) -> bool {
        self.ascii_only
    }
}

/// Maximum sub-panes in a `TABCONTROLDATA` record.
///
/// `Gadget.h` sizes the pane array at `NUM_TAB_PANES = 8`. The source reads its
/// `PANEDISABLED` count straight from the file and then writes that many entries into the
/// fixed array without checking it, so a hostile layout would overflow; this decoder treats
/// a count above the array size as a malformed record instead.
pub const WND_TAB_PANES: usize = 8;

/// A decoded `TABCONTROLDATA` record.
///
/// No retail layout in either edition declares one, so this shape comes from the source
/// alone and has not been cross-checked against real data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WndTabControlData {
    tab_orientation: i32,
    tab_edge: i32,
    tab_width: i32,
    tab_height: i32,
    tab_count: i32,
    pane_border: i32,
    pane_disabled: Vec<bool>,
}

impl WndTabControlData {
    /// Returns `TABORIENTATION`.
    #[must_use]
    pub const fn tab_orientation(&self) -> i32 {
        self.tab_orientation
    }

    /// Returns `TABEDGE`.
    #[must_use]
    pub const fn tab_edge(&self) -> i32 {
        self.tab_edge
    }

    /// Returns `TABWIDTH`.
    #[must_use]
    pub const fn tab_width(&self) -> i32 {
        self.tab_width
    }

    /// Returns `TABHEIGHT`.
    #[must_use]
    pub const fn tab_height(&self) -> i32 {
        self.tab_height
    }

    /// Returns `TABCOUNT`.
    #[must_use]
    pub const fn tab_count(&self) -> i32 {
        self.tab_count
    }

    /// Returns `PANEBORDER`.
    #[must_use]
    pub const fn pane_border(&self) -> i32 {
        self.pane_border
    }

    /// Returns the per-pane disabled flags declared after `PANEDISABLED`'s count.
    #[must_use]
    pub fn pane_disabled(&self) -> &[bool] {
        &self.pane_disabled
    }
}

/// One decoded gadget-specific `DATA` record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WndGadgetData {
    /// `LISTBOXDATA`, on `SCROLLLISTBOX` windows.
    ListBox(WndListBoxData),
    /// `COMBOBOXDATA`, on `COMBOBOX` windows.
    ComboBox(WndComboBoxData),
    /// `SLIDERDATA`, on slider windows.
    Slider(WndSliderData),
    /// `RADIOBUTTONDATA`'s `GROUP` value, on `RADIOBUTTON` windows.
    RadioButtonGroup(i32),
    /// `TEXTENTRYDATA`, on `ENTRYFIELD` windows.
    TextEntry(WndTextEntryData),
    /// `STATICTEXTDATA`'s `CENTERED` flag, on `STATICTEXT` windows.
    StaticTextCentered(bool),
    /// `TABCONTROLDATA`, on `TABCONTROL` windows. Absent from retail data.
    TabControl(WndTabControlData),
}

/// One immutable window/gadget declaration, with only `WINDOWTYPE` and `SCREENRECT` typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WndWindow {
    id: usize,
    window_type: Box<str>,
    rect: WndScreenRect,
    typed: TypedFields,
    fields: Vec<WndField>,
    children: Vec<WndWindow>,
}

impl WndWindow {
    /// Returns a stable, source-order identifier (`0`-based) for this window.
    #[must_use]
    pub const fn id(&self) -> usize {
        self.id
    }

    /// Returns the decorated source name from the window's `NAME` record, if it declared
    /// one. The value is the full decorated form (`"MainMenu.wnd:ButtonExit"`), which is
    /// what a later patch overlay targets.
    ///
    /// Typed accessors like this one are views over [`WndWindow::fields`], which remains
    /// the complete retained record. Nothing is moved out of `fields` when it gains a type.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.typed.name.as_deref()
    }

    /// Returns the `STATUS` flag names in source order, each marked known or unknown.
    #[must_use]
    pub fn status(&self) -> &[WndFlag] {
        &self.typed.status
    }

    /// Returns the `STYLE` flag names in source order, each marked known or unknown.
    ///
    /// `STYLE` restates the window's type alongside modifiers, so the type name itself
    /// appears here as well as in [`WndWindow::window_type`].
    #[must_use]
    pub fn style(&self) -> &[WndFlag] {
        &self.typed.style
    }

    /// Returns the four retained callback names.
    #[must_use]
    pub const fn callbacks(&self) -> &WndCallbacks {
        &self.typed.callbacks
    }

    /// Returns the decoded `FONT` record, if one was declared and well-formed.
    #[must_use]
    pub const fn font(&self) -> Option<&WndFont> {
        self.typed.font.as_ref()
    }

    /// Returns the raw `HEADERTEMPLATE` name.
    ///
    /// The value is retained exactly, including the case-variant "no template" sentinel
    /// (`[NONE]` and `[None]` both occur in retail data). Interpreting the sentinel is a
    /// resource-resolution concern, not a syntax one.
    #[must_use]
    pub fn header_template(&self) -> Option<&str> {
        self.typed.header_template.as_deref()
    }

    /// Returns the `TOOLTIPDELAY` value. `-1` is the overwhelmingly common authored value.
    #[must_use]
    pub const fn tooltip_delay(&self) -> Option<i32> {
        self.typed.tooltip_delay
    }

    /// Returns the raw `TEXT` value.
    ///
    /// This is not always a localization label: retail layouts author both label keys
    /// (`GUI:Monkey`) and literal strings (`Static Text`) here, so the parser retains the
    /// value rather than classifying it.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        self.typed.text.as_deref()
    }

    /// Returns the raw `TOOLTIPTEXT` value, under the same retention rule as
    /// [`WndWindow::text`].
    #[must_use]
    pub fn tooltip_text(&self) -> Option<&str> {
        self.typed.tooltip_text.as_deref()
    }

    /// Returns the decoded `TEXTCOLOR` state colors, if declared and well-formed.
    #[must_use]
    pub const fn text_colors(&self) -> Option<&WndTextColors> {
        self.typed.text_colors.as_ref()
    }

    /// Returns every decoded draw-data array with its slot, in source order.
    #[must_use]
    pub fn draw_data(&self) -> &[(WndDrawDataSlot, WndDrawData)] {
        &self.typed.draw_data
    }

    /// Returns the draw-data array for one slot, if the window declared it.
    #[must_use]
    pub fn draw_data_for(&self, slot: WndDrawDataSlot) -> Option<&WndDrawData> {
        self.typed
            .draw_data
            .iter()
            .find(|(candidate, _)| *candidate == slot)
            .map(|(_, data)| data)
    }

    /// Returns the window's gadget-specific `DATA` record, if it declared a decodable one.
    #[must_use]
    pub const fn gadget_data(&self) -> Option<&WndGadgetData> {
        self.typed.gadget_data.as_ref()
    }

    /// Returns the `IMAGEOFFSET` `(x, y)` pair. No retail layout declares one.
    #[must_use]
    pub const fn image_offset(&self) -> Option<(i32, i32)> {
        self.typed.image_offset
    }

    /// Returns the control portion of the decorated name — the part after the first `:` —
    /// when it is non-empty.
    ///
    /// Retail layouts give every window a `NAME`, but 126 of 1,667 across both editions
    /// declare only the layout prefix with an empty control part (`"OptionsMenu.wnd:"`).
    /// Those are unnamed windows rather than windows sharing a name, so identity comparison
    /// and duplicate detection use this accessor, not [`WndWindow::name`].
    #[must_use]
    pub fn control_name(&self) -> Option<&str> {
        let name = self.typed.name.as_deref()?;
        let (_, control) = name.split_once(':')?;
        (!control.is_empty()).then_some(control)
    }

    /// Returns the raw `WINDOWTYPE` value exactly as spelled in the source.
    #[must_use]
    pub fn window_type(&self) -> &str {
        &self.window_type
    }

    /// Returns the decoded creation rectangle and resolution.
    #[must_use]
    pub const fn rect(&self) -> WndScreenRect {
        self.rect
    }

    /// Returns every generically retained field on this window, in source order.
    #[must_use]
    pub fn fields(&self) -> &[WndField] {
        &self.fields
    }

    /// Returns nested `CHILD` windows, in source order.
    #[must_use]
    pub fn children(&self) -> &[WndWindow] {
        &self.children
    }
}

/// Non-fatal detail about an unrecognized field name or an out-of-vocabulary value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WndDiagnosticKind {
    /// A top-level keyword outside the source-confirmed default-visual-record vocabulary.
    UnknownField {
        /// Raw field name.
        name: Box<str>,
    },
    /// A recognized field whose value is outside the source-confirmed name vocabulary.
    UnrecognizedValue {
        /// Raw field name.
        field: Box<str>,
        /// Raw value.
        value: Box<str>,
    },
    /// A child `WINDOW` opened inside an already-open child list without its own
    /// preceding `CHILD` marker. The source child-list loop dispatches on `WINDOW` and
    /// has no `CHILD` case at all, so this is valid but unconventional: retail data
    /// writes `CHILD` before all but one child across both editions.
    MissingChildKeyword,
    /// A field whose established shape is known did not match it, so its typed view is
    /// absent. The field itself is still retained generically.
    MalformedField {
        /// Raw field name.
        field: Box<str>,
        /// What the decoder expected.
        reason: Box<str>,
    },
    /// Two windows in one document declared the same non-empty decorated control name.
    ///
    /// This is a diagnostic rather than an error. The legacy runtime creates both windows
    /// and no retail layout in either edition contains such a pair, so rejecting the
    /// document would only make an unusual modded layout undecodable while hiding the
    /// collision that a patch overlay actually needs to know about.
    DuplicateWindowName {
        /// The repeated decorated name, exactly as spelled.
        name: Box<str>,
        /// Source-order id of the window that declared the name first.
        first_window_id: usize,
    },
}

/// One non-fatal parse-time observation; never causes [`parse_wnd`] to fail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WndDiagnostic {
    line: usize,
    window_id: Option<usize>,
    kind: WndDiagnosticKind,
}

impl WndDiagnostic {
    /// Returns the one-based source line the diagnostic applies to.
    #[must_use]
    pub const fn line(&self) -> usize {
        self.line
    }

    /// Returns the enclosing window's id, or `None` for a top-level diagnostic.
    #[must_use]
    pub const fn window_id(&self) -> Option<usize> {
        self.window_id
    }

    /// Returns the diagnostic detail.
    #[must_use]
    pub const fn kind(&self) -> &WndDiagnosticKind {
        &self.kind
    }
}

/// One complete, immutable WND document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WndDocument {
    file_version: u32,
    layout: Option<WndLayoutBlock>,
    top_level_fields: Vec<WndField>,
    windows: Vec<WndWindow>,
    diagnostics: Vec<WndDiagnostic>,
}

impl WndDocument {
    /// Returns the declared `FILE_VERSION`.
    #[must_use]
    pub const fn file_version(&self) -> u32 {
        self.file_version
    }

    /// Returns the layout init/update/shutdown block. Always present: version 1 documents
    /// receive the source-confirmed `"[None]"` default for every callback name rather than
    /// an absent block.
    #[must_use]
    pub const fn layout(&self) -> Option<&WndLayoutBlock> {
        self.layout.as_ref()
    }

    /// Returns optional pre/inter-window default-visual records (`ENABLEDCOLOR`, `FONT`,
    /// and similar), in source order.
    #[must_use]
    pub fn top_level_fields(&self) -> &[WndField] {
        &self.top_level_fields
    }

    /// Returns top-level `WINDOW` declarations, in source order.
    #[must_use]
    pub fn windows(&self) -> &[WndWindow] {
        &self.windows
    }

    /// Returns every non-fatal diagnostic collected while parsing, in encounter order.
    #[must_use]
    pub fn diagnostics(&self) -> &[WndDiagnostic] {
        &self.diagnostics
    }
}

/// A structured, bounded WND decoding failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WndError {
    /// The complete input exceeds [`WndLimits::maximum_file_bytes`].
    FileTooLarge { size: usize, limit: usize },
    /// The document exceeds [`WndLimits::maximum_tokens`].
    TooManyTokens { limit: usize },
    /// The document exceeds [`WndLimits::maximum_lines`].
    TooManyLines { limit: usize },
    /// One semicolon-terminated record exceeds [`WndLimits::maximum_record_bytes`].
    RecordTooLong {
        line: usize,
        size: usize,
        limit: usize,
    },
    /// One bare or quoted token exceeds [`WndLimits::maximum_field_bytes`].
    FieldTooLong {
        line: usize,
        size: usize,
        limit: usize,
    },
    /// One record exceeds [`WndLimits::maximum_record_tokens`].
    TooManyRecordTokens { line: usize, limit: usize },
    /// Input ended while a token, record, or block was still open.
    UnexpectedEof { line: usize },
    /// A quoted string was never closed before input ended.
    UnterminatedString { line: usize },
    /// A token or record was not valid UTF-8.
    InvalidUtf8 { line: usize },
    /// The document did not begin with a `FILE_VERSION` record.
    MissingFileVersion { line: usize },
    /// `FILE_VERSION`'s value was not a valid non-negative integer.
    InvalidFileVersion { line: usize },
    /// A `NAME = value` record was missing its `=`.
    MissingEquals { line: usize },
    /// `FILE_VERSION >= 2` requires a `STARTLAYOUTBLOCK` immediately afterward.
    MissingLayoutBlock { line: usize },
    /// A token inside `STARTLAYOUTBLOCK`/`ENDLAYOUTBLOCK` was not one of the three known
    /// callback names.
    UnknownLayoutBlockToken { line: usize, token: Box<str> },
    /// A `WINDOW` block's first field was not `WINDOWTYPE`.
    MissingWindowType { line: usize },
    /// A `WINDOW` block closed without ever declaring `SCREENRECT`.
    MissingScreenRect { line: usize },
    /// A `SCREENRECT` value did not match the source `UPPERLEFT`/`BOTTOMRIGHT`/
    /// `CREATIONRESOLUTION` grammar.
    InvalidScreenRect { line: usize },
    /// The token immediately after a `CHILD` keyword was not `WINDOW`. `CHILD` itself is
    /// optional before a sibling once the child list is open; only this ordering is fatal.
    ExpectedChildWindow { line: usize },
    /// A `WINDOW` block declared at least one `CHILD` but closed without `ENDALLCHILDREN`.
    MissingEndAllChildren { line: usize },
    /// The document exceeds [`WndLimits::maximum_windows`].
    TooManyWindows { limit: usize },
    /// A `CHILD` nesting exceeds [`WndLimits::maximum_depth`].
    TooDeeplyNested { limit: usize },
    /// The document closed without declaring a single top-level `WINDOW`.
    NoWindows,
}

impl Display for WndError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::FileTooLarge { size, limit } => {
                write!(formatter, "WND is {size} bytes; limit is {limit}")
            }
            Self::TooManyTokens { limit } => write!(formatter, "WND exceeds {limit} tokens"),
            Self::TooManyLines { limit } => write!(formatter, "WND exceeds {limit} lines"),
            Self::RecordTooLong { line, size, limit } => write!(
                formatter,
                "WND record at line {line} is {size} bytes; limit is {limit}"
            ),
            Self::FieldTooLong { line, size, limit } => write!(
                formatter,
                "WND token at line {line} is {size} bytes; limit is {limit}"
            ),
            Self::TooManyRecordTokens { line, limit } => write!(
                formatter,
                "WND record at line {line} exceeds {limit} tokens"
            ),
            Self::UnexpectedEof { line } => {
                write!(formatter, "WND ended unexpectedly at line {line}")
            }
            Self::UnterminatedString { line } => {
                write!(
                    formatter,
                    "WND quoted string at line {line} was never closed"
                )
            }
            Self::InvalidUtf8 { line } => {
                write!(formatter, "WND token at line {line} is not valid UTF-8")
            }
            Self::MissingFileVersion { line } => write!(
                formatter,
                "WND at line {line} does not begin with FILE_VERSION"
            ),
            Self::InvalidFileVersion { line } => write!(
                formatter,
                "WND FILE_VERSION at line {line} is not a valid integer"
            ),
            Self::MissingEquals { line } => {
                write!(formatter, "WND record at line {line} is missing '='")
            }
            Self::MissingLayoutBlock { line } => write!(
                formatter,
                "WND at line {line} requires STARTLAYOUTBLOCK for FILE_VERSION >= 2"
            ),
            Self::UnknownLayoutBlockToken { line, token } => write!(
                formatter,
                "WND layout block at line {line} has unknown token '{token}'"
            ),
            Self::MissingWindowType { line } => write!(
                formatter,
                "WND WINDOW at line {line} must begin with WINDOWTYPE"
            ),
            Self::MissingScreenRect { line } => write!(
                formatter,
                "WND WINDOW at line {line} never declared SCREENRECT"
            ),
            Self::InvalidScreenRect { line } => {
                write!(formatter, "WND SCREENRECT at line {line} is malformed")
            }
            Self::ExpectedChildWindow { line } => write!(
                formatter,
                "WND CHILD at line {line} must be followed by WINDOW"
            ),
            Self::MissingEndAllChildren { line } => write!(
                formatter,
                "WND WINDOW at line {line} declared CHILD but never ENDALLCHILDREN"
            ),
            Self::TooManyWindows { limit } => write!(formatter, "WND exceeds {limit} windows"),
            Self::TooDeeplyNested { limit } => {
                write!(formatter, "WND CHILD nesting exceeds depth {limit}")
            }
            Self::NoWindows => write!(formatter, "WND declares no top-level WINDOW"),
        }
    }
}

impl Error for WndError {}

/// Top-level default-visual keywords confirmed directly from `winCreateFromScript`'s flat
/// parse loop. Matched case-sensitively, matching the source's `asciibuf.compare(...)`.
const TOP_LEVEL_KEYWORDS: [&str; 7] = [
    "ENABLEDCOLOR",
    "DISABLEDCOLOR",
    "HILITECOLOR",
    "SELECTEDCOLOR",
    "TEXTCOLOR",
    "BACKGROUNDCOLOR",
    "FONT",
];

fn is_known_top_level_field(name: &str) -> bool {
    TOP_LEVEL_KEYWORDS.contains(&name)
}

/// Established `WindowStyleNames` vocabulary. Matched case-insensitively, matching the
/// source's `stricmp`-based status/style lookup.
const KNOWN_STYLES: [&str; 16] = [
    "PUSHBUTTON",
    "RADIOBUTTON",
    "CHECKBOX",
    "VERTSLIDER",
    "HORZSLIDER",
    "SCROLLLISTBOX",
    "ENTRYFIELD",
    "STATICTEXT",
    "PROGRESSBAR",
    "USER",
    "MOUSETRACK",
    "ANIMATED",
    "TABSTOP",
    "TABCONTROL",
    "TABPANE",
    "COMBOBOX",
];

fn is_known_style(value: &str) -> bool {
    KNOWN_STYLES
        .iter()
        .any(|candidate| value.eq_ignore_ascii_case(candidate))
}

/// Established `WindowStatusNames` vocabulary, matched case-insensitively.
///
/// This is the union of both source paths. The `Generals` path defines the first 25 names;
/// the `GeneralsMD` (Zero Hour) path appends `ON_MOUSE_DOWN`, which occurs 67 times in
/// retail Zero Hour layouts. Bit positions are array indices, so the shared prefix carries
/// identical values in both editions and Zero Hour only extends the high end. A decoder is
/// not told which edition a document came from, so it validates against the union; an
/// edition-specific check belongs to a profile, not to the syntax layer.
const KNOWN_STATUSES: [&str; 26] = [
    "ACTIVE",
    "TOGGLE",
    "DRAGABLE",
    "ENABLED",
    "HIDDEN",
    "ABOVE",
    "BELOW",
    "IMAGE",
    "TABSTOP",
    "NOINPUT",
    "NOFOCUS",
    "DESTROYED",
    "BORDER",
    "SMOOTH_TEXT",
    "ONE_LINE",
    "NO_FLUSH",
    "SEE_THRU",
    "RIGHT_CLICK",
    "WRAP_CENTERED",
    "CHECK_LIKE",
    "HOTKEY_TEXT",
    "USE_OVERLAY_STATES",
    "NOT_READY",
    "FLASHING",
    "ALWAYS_COLOR",
    "ON_MOUSE_DOWN",
];

fn is_known_status(value: &str) -> bool {
    KNOWN_STATUSES
        .iter()
        .any(|candidate| value.eq_ignore_ascii_case(candidate))
}

struct Token<'a> {
    text: &'a [u8],
    kind: WndTokenKind,
    line: usize,
}

/// Single-character record delimiters split into their own tokens. `=` and `;` are
/// structural and handled separately; `,`, `:`, and `+` separate sub-records and flag
/// names inside a value. Splitting them is lossless because [`WndField::raw_value`] keeps
/// the verbatim source span alongside the token view.
const PUNCTUATION: [u8; 3] = [b',', b':', b'+'];

fn is_token_break(byte: u8) -> bool {
    byte.is_ascii_whitespace() || byte == b'=' || byte == b';' || PUNCTUATION.contains(&byte)
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
    line: usize,
    tokens_read: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            pos: 0,
            line: 1,
            tokens_read: 0,
        }
    }

    fn peek_byte(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<u8> {
        let byte = self.peek_byte()?;
        self.pos += 1;
        if byte == b'\n' {
            self.line += 1;
        }
        Some(byte)
    }

    fn skip_whitespace(&mut self) {
        while self
            .peek_byte()
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            self.advance();
        }
    }

    fn at_end(&mut self) -> bool {
        self.skip_whitespace();
        self.peek_byte().is_none()
    }

    /// Reads the next bare or quoted token. `=` and `;` are always returned as their own
    /// single-character tokens; anything else stops at whitespace, `=`, or `;`.
    fn next_token(&mut self, limits: WndLimits) -> Result<Token<'a>, WndError> {
        self.skip_whitespace();
        if self.line > limits.maximum_lines {
            return Err(WndError::TooManyLines {
                limit: limits.maximum_lines,
            });
        }
        let line = self.line;
        let Some(byte) = self.peek_byte() else {
            return Err(WndError::UnexpectedEof { line });
        };
        self.tokens_read = self
            .tokens_read
            .checked_add(1)
            .ok_or(WndError::TooManyTokens {
                limit: limits.maximum_tokens,
            })?;
        if self.tokens_read > limits.maximum_tokens {
            return Err(WndError::TooManyTokens {
                limit: limits.maximum_tokens,
            });
        }
        if byte == b'"' {
            self.advance();
            let start = self.pos;
            loop {
                match self.peek_byte() {
                    Some(b'"') => {
                        let text = &self.bytes[start..self.pos];
                        self.advance();
                        return Ok(Token {
                            text,
                            kind: WndTokenKind::Quoted,
                            line,
                        });
                    }
                    Some(_) => {
                        self.advance();
                        if self.pos - start > limits.maximum_field_bytes {
                            return Err(WndError::FieldTooLong {
                                line,
                                size: self.pos - start,
                                limit: limits.maximum_field_bytes,
                            });
                        }
                    }
                    None => return Err(WndError::UnterminatedString { line }),
                }
            }
        }
        if byte == b'=' || byte == b';' || PUNCTUATION.contains(&byte) {
            self.advance();
            return Ok(Token {
                text: &self.bytes[self.pos - 1..self.pos],
                kind: if byte == b'=' || byte == b';' {
                    WndTokenKind::Word
                } else {
                    WndTokenKind::Punctuation
                },
                line,
            });
        }
        let start = self.pos;
        loop {
            match self.peek_byte() {
                Some(next) if !is_token_break(next) => {
                    self.advance();
                }
                _ => break,
            }
            if self.pos - start > limits.maximum_field_bytes {
                return Err(WndError::FieldTooLong {
                    line,
                    size: self.pos - start,
                    limit: limits.maximum_field_bytes,
                });
            }
        }
        Ok(Token {
            text: &self.bytes[start..self.pos],
            kind: WndTokenKind::Word,
            line,
        })
    }

    /// Reads a semicolon-terminated record, returning both its verbatim source text
    /// (whitespace-collapsed and trimmed) and its ordered tokens.
    ///
    /// The source reader's own scan collapses whitespace runs, trims, and stops at `;`;
    /// this reproduces that text exactly while additionally retaining quoting and
    /// punctuation as structure. A missing semicolon is not specially detected: like the
    /// source, subsequent structural tokens are folded into the value until a `;` is found
    /// or a limit/EOF error occurs.
    fn read_record(&mut self, limits: WndLimits) -> Result<Record, WndError> {
        self.skip_whitespace();
        let start = self.pos;
        let mut end = start;
        let mut tokens = Vec::new();
        loop {
            let token = self.next_token(limits)?;
            if token.text == b";" && token.kind == WndTokenKind::Word {
                break;
            }
            end = self.pos;
            if end - start > limits.maximum_record_bytes {
                return Err(WndError::RecordTooLong {
                    line: token.line,
                    size: end - start,
                    limit: limits.maximum_record_bytes,
                });
            }
            if tokens.len() >= limits.maximum_record_tokens {
                return Err(WndError::TooManyRecordTokens {
                    line: token.line,
                    limit: limits.maximum_record_tokens,
                });
            }
            let text = std::str::from_utf8(token.text)
                .map_err(|_| WndError::InvalidUtf8 { line: token.line })?;
            tokens.push(WndToken {
                text: text.into(),
                kind: token.kind,
                line: token.line,
            });
        }
        let span = std::str::from_utf8(&self.bytes[start..end])
            .map_err(|_| WndError::InvalidUtf8 { line: self.line })?;
        Ok(Record {
            raw_value: collapse_whitespace(span),
            tokens,
        })
    }
}

/// One decoded semicolon-terminated record.
struct Record {
    raw_value: Box<str>,
    tokens: Vec<WndToken>,
}

/// Collapses whitespace runs to single spaces and trims both ends, matching the source
/// reader's record scan. Whitespace is ASCII, so this is safe over UTF-8 text.
fn collapse_whitespace(text: &str) -> Box<str> {
    let mut collapsed = String::with_capacity(text.len());
    let mut in_whitespace = true;
    for character in text.chars() {
        if character.is_ascii_whitespace() {
            in_whitespace = true;
            continue;
        }
        if in_whitespace && !collapsed.is_empty() {
            collapsed.push(' ');
        }
        in_whitespace = false;
        collapsed.push(character);
    }
    collapsed.into_boxed_str()
}

fn expect_equals(cursor: &mut Cursor<'_>, limits: WndLimits, line: usize) -> Result<(), WndError> {
    let token = cursor.next_token(limits)?;
    if token.text == b"=" {
        Ok(())
    } else {
        Err(WndError::MissingEquals { line })
    }
}

fn decode_token<'a>(token: &Token<'a>) -> Result<&'a str, WndError> {
    std::str::from_utf8(token.text).map_err(|_| WndError::InvalidUtf8 { line: token.line })
}

fn read_file_version(cursor: &mut Cursor<'_>, limits: WndLimits) -> Result<u32, WndError> {
    let keyword = cursor.next_token(limits)?;
    if keyword.text != b"FILE_VERSION" {
        return Err(WndError::MissingFileVersion { line: keyword.line });
    }
    expect_equals(cursor, limits, keyword.line)?;
    let record = cursor.read_record(limits)?;
    record
        .raw_value
        .parse::<u32>()
        .map_err(|_| WndError::InvalidFileVersion { line: keyword.line })
}

fn read_layout_block(
    cursor: &mut Cursor<'_>,
    limits: WndLimits,
) -> Result<WndLayoutBlock, WndError> {
    let start = cursor.next_token(limits)?;
    if start.text != b"STARTLAYOUTBLOCK" {
        return Err(WndError::MissingLayoutBlock { line: start.line });
    }
    let mut init = None;
    let mut update = None;
    let mut shutdown = None;
    loop {
        let token = cursor.next_token(limits)?;
        if token.text == b"ENDLAYOUTBLOCK" {
            break;
        }
        let slot = match token.text {
            b"LAYOUTINIT" => &mut init,
            b"LAYOUTUPDATE" => &mut update,
            b"LAYOUTSHUTDOWN" => &mut shutdown,
            _ => {
                let text = decode_token(&token)?;
                return Err(WndError::UnknownLayoutBlockToken {
                    line: token.line,
                    token: text.into(),
                });
            }
        };
        expect_equals(cursor, limits, token.line)?;
        *slot = Some(cursor.read_record(limits)?.raw_value);
    }
    Ok(WndLayoutBlock {
        init,
        update,
        shutdown,
    })
}

/// Yields a record's meaningful tokens, skipping the `,`/`:`/`+` delimiters that separate
/// sub-records. `SCREENRECT` is authored both with and without commas across retail files.
fn significant_tokens(tokens: &[WndToken]) -> impl Iterator<Item = &str> {
    tokens
        .iter()
        .filter(|token| !token.is_punctuation())
        .map(WndToken::text)
}

fn expect_literal<'a>(
    tokens: &mut impl Iterator<Item = &'a str>,
    expected: &str,
    line: usize,
) -> Result<(), WndError> {
    match tokens.next() {
        Some(token) if token == expected => Ok(()),
        _ => Err(WndError::InvalidScreenRect { line }),
    }
}

fn next_screen_rect_int<'a>(
    tokens: &mut impl Iterator<Item = &'a str>,
    line: usize,
) -> Result<i32, WndError> {
    tokens
        .next()
        .and_then(|token| token.parse::<i32>().ok())
        .ok_or(WndError::InvalidScreenRect { line })
}

fn parse_screen_rect(record: &[WndToken], line: usize) -> Result<WndScreenRect, WndError> {
    let mut tokens = significant_tokens(record);
    expect_literal(&mut tokens, "UPPERLEFT", line)?;
    let upper_left = (
        next_screen_rect_int(&mut tokens, line)?,
        next_screen_rect_int(&mut tokens, line)?,
    );
    expect_literal(&mut tokens, "BOTTOMRIGHT", line)?;
    let bottom_right = (
        next_screen_rect_int(&mut tokens, line)?,
        next_screen_rect_int(&mut tokens, line)?,
    );
    expect_literal(&mut tokens, "CREATIONRESOLUTION", line)?;
    let creation_resolution = (
        next_screen_rect_int(&mut tokens, line)?,
        next_screen_rect_int(&mut tokens, line)?,
    );
    if tokens.next().is_some() {
        return Err(WndError::InvalidScreenRect { line });
    }
    Ok(WndScreenRect {
        upper_left,
        bottom_right,
        creation_resolution,
    })
}

struct ParseState {
    windows_seen: usize,
    /// Decorated control names already seen, mapped to the first window that declared
    /// them. A `BTreeMap` keeps lookups ordered and iteration-stable; only encounter-order
    /// diagnostics are emitted from it.
    control_names: BTreeMap<Box<str>, usize>,
}

/// Typed views over a window's retained fields.
///
/// Every value here is derived from an entry that also stays in [`WndWindow::fields`]; the
/// never-dropped invariant is unchanged by typing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TypedFields {
    name: Option<Box<str>>,
    status: Vec<WndFlag>,
    style: Vec<WndFlag>,
    callbacks: WndCallbacks,
    font: Option<WndFont>,
    header_template: Option<Box<str>>,
    tooltip_delay: Option<i32>,
    text: Option<Box<str>>,
    tooltip_text: Option<Box<str>>,
    text_colors: Option<WndTextColors>,
    draw_data: Vec<(WndDrawDataSlot, WndDrawData)>,
    gadget_data: Option<WndGadgetData>,
    image_offset: Option<(i32, i32)>,
}

/// One decoded draw-data array: exactly [`WND_DRAW_DATA_ENTRIES`] state entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WndDrawData {
    entries: Vec<WndDrawEntry>,
}

impl WndDrawData {
    /// Returns the entries in source order. Always [`WND_DRAW_DATA_ENTRIES`] long.
    #[must_use]
    pub fn entries(&self) -> &[WndDrawEntry] {
        &self.entries
    }
}

/// Maximum `COLUMNSWIDTH` entries decoded from one `LISTBOXDATA` record.
///
/// The count comes from the record's own `COLUMNS` value, so it is attacker-controlled and
/// needs a bound before allocation. The widest retail list declares eight columns.
const MAXIMUM_LIST_COLUMNS: usize = 256;

/// The `TEXTCOLOR` sub-labels, in the order the source writes them.
const TEXT_COLOR_LABELS: [&str; 6] = [
    "ENABLED",
    "ENABLEDBORDER",
    "DISABLED",
    "DISABLEDBORDER",
    "HILITE",
    "HILITEBORDER",
];

impl TypedFields {
    /// Types one field, leaving the value absent and emitting a
    /// [`WndDiagnosticKind::MalformedField`] when the record does not match its established
    /// shape.
    ///
    /// Typed decoding never fails the document. Required structural values (`FILE_VERSION`,
    /// `WINDOWTYPE`, `SCREENRECT`) are hard errors because nothing downstream can proceed
    /// without them; an optional presentation record is a *view* over a field that is
    /// retained generically either way, so a malformed one stays visible as a diagnostic
    /// rather than making the whole layout undecodable.
    fn absorb(&mut self, field: &WndField, id: usize, diagnostics: &mut Vec<WndDiagnostic>) {
        match field.name() {
            // Retail always quotes this value, so the single quoted token carries the
            // decorated name with its `:` intact (quoted tokens are never split). An
            // unquoted spelling arrives as `Word(File.wnd) Punct(:) Word(Control)`, so
            // concatenating token text without separators reconstructs both forms. Using
            // `raw_value` here would embed the source's quote characters in the name and
            // break exact-name matching for later patch overlays.
            "NAME" => {
                let value: String = field.tokens().iter().map(WndToken::text).collect();
                self.name = (!value.is_empty()).then(|| value.into_boxed_str());
            }
            "STATUS" => self.status = parse_flags(field, id, is_known_status, diagnostics),
            "STYLE" => self.style = parse_flags(field, id, is_known_style, diagnostics),
            "SYSTEMCALLBACK" => self.set_callback(WndCallbackKind::System, field, id, diagnostics),
            "INPUTCALLBACK" => self.set_callback(WndCallbackKind::Input, field, id, diagnostics),
            "TOOLTIPCALLBACK" => {
                self.set_callback(WndCallbackKind::Tooltip, field, id, diagnostics);
            }
            "DRAWCALLBACK" => self.set_callback(WndCallbackKind::Draw, field, id, diagnostics),
            "HEADERTEMPLATE" => self.header_template = single_value(field, id, diagnostics),
            "TEXT" => self.text = single_value(field, id, diagnostics),
            "TOOLTIPTEXT" => self.tooltip_text = single_value(field, id, diagnostics),
            "TOOLTIPDELAY" => {
                self.tooltip_delay = single_value(field, id, diagnostics).and_then(|value| {
                    let delay = value.parse::<i32>().ok();
                    if delay.is_none() {
                        push_malformed(field, id, "value is not an integer", diagnostics);
                    }
                    delay
                });
            }
            "FONT" => self.font = parse_font(field, id, diagnostics),
            "TEXTCOLOR" => self.text_colors = parse_text_colors(field, id, diagnostics),
            "LISTBOXDATA" => self.set_gadget_data(parse_list_box_data(field, id, diagnostics)),
            "COMBOBOXDATA" => self.set_gadget_data(parse_combo_box_data(field, id, diagnostics)),
            "SLIDERDATA" => self.set_gadget_data(parse_slider_data(field, id, diagnostics)),
            "RADIOBUTTONDATA" => self.set_gadget_data(
                parse_single_int(field, id, diagnostics).map(WndGadgetData::RadioButtonGroup),
            ),
            "TEXTENTRYDATA" => self.set_gadget_data(parse_text_entry_data(field, id, diagnostics)),
            "TABCONTROLDATA" => {
                self.set_gadget_data(parse_tab_control_data(field, id, diagnostics));
            }
            "IMAGEOFFSET" => self.image_offset = parse_image_offset(field, id, diagnostics),
            // `TOOLTIP` is deliberately not typed. The source's `parseTooltip` ignores its
            // buffer entirely and stores a placeholder string, marked `@todo`, so the
            // record has no established grammar to decode. It stays retained generically.
            "STATICTEXTDATA" => self.set_gadget_data(
                parse_single_int(field, id, diagnostics)
                    .map(|centered| WndGadgetData::StaticTextCentered(centered != 0)),
            ),
            name => {
                if let Some((_, slot)) =
                    DRAW_DATA_SLOTS.iter().find(|(keyword, _)| *keyword == name)
                    && let Some(draw_data) = parse_draw_data(field, id, diagnostics)
                {
                    self.draw_data.push((*slot, draw_data));
                }
            }
        }
    }

    fn set_gadget_data(&mut self, data: Option<WndGadgetData>) {
        if let Some(data) = data {
            self.gadget_data = Some(data);
        }
    }

    fn set_callback(
        &mut self,
        kind: WndCallbackKind,
        field: &WndField,
        id: usize,
        diagnostics: &mut Vec<WndDiagnostic>,
    ) {
        *self.callbacks.slot(kind) = single_value(field, id, diagnostics);
    }
}

fn push_malformed(field: &WndField, id: usize, reason: &str, diagnostics: &mut Vec<WndDiagnostic>) {
    diagnostics.push(WndDiagnostic {
        line: field.line(),
        window_id: Some(id),
        kind: WndDiagnosticKind::MalformedField {
            field: field.name().into(),
            reason: reason.into(),
        },
    });
}

/// Returns the sole meaningful token's text. Callback, template, and text records are each
/// authored as exactly one (normally quoted) token.
fn single_value(
    field: &WndField,
    id: usize,
    diagnostics: &mut Vec<WndDiagnostic>,
) -> Option<Box<str>> {
    let mut tokens = significant_tokens(field.tokens());
    let value = tokens.next();
    match (value, tokens.next()) {
        (Some(value), None) => Some(value.into()),
        (None, _) => {
            push_malformed(field, id, "record has no value", diagnostics);
            None
        }
        (Some(_), Some(_)) => {
            push_malformed(field, id, "record has more than one value", diagnostics);
            None
        }
    }
}

/// Splits a `+`-separated flag list (`ENABLED+NOFOCUS+SEE_THRU`), retaining each name's
/// exact spelling. Names are matched case-insensitively, matching the source's `stricmp`
/// lookup, while the field keyword itself was matched case-sensitively.
fn parse_flags(
    field: &WndField,
    id: usize,
    known: fn(&str) -> bool,
    diagnostics: &mut Vec<WndDiagnostic>,
) -> Vec<WndFlag> {
    let mut flags = Vec::new();
    for token in field.tokens() {
        if token.is_punctuation() {
            continue;
        }
        let known = known(token.text());
        if !known {
            diagnostics.push(WndDiagnostic {
                line: token.line(),
                window_id: Some(id),
                kind: WndDiagnosticKind::UnrecognizedValue {
                    field: field.name().into(),
                    value: token.text().into(),
                },
            });
        }
        flags.push(WndFlag {
            name: token.text().into(),
            known,
        });
    }
    flags
}

fn expect_sub_label<'a>(tokens: &mut impl Iterator<Item = &'a str>, expected: &str) -> bool {
    tokens.next().is_some_and(|token| token == expected)
}

fn next_channel<'a>(tokens: &mut impl Iterator<Item = &'a str>) -> Option<u8> {
    tokens.next()?.parse::<u8>().ok()
}

fn next_color<'a>(tokens: &mut impl Iterator<Item = &'a str>) -> Option<WndColor> {
    Some(WndColor {
        red: next_channel(tokens)?,
        green: next_channel(tokens)?,
        blue: next_channel(tokens)?,
        alpha: next_channel(tokens)?,
    })
}

/// Decodes `FONT = NAME: "<name>", SIZE: <size>, BOLD: <flag>;`.
fn parse_font(
    field: &WndField,
    id: usize,
    diagnostics: &mut Vec<WndDiagnostic>,
) -> Option<WndFont> {
    let mut tokens = significant_tokens(field.tokens());
    let font = (|| {
        if !expect_sub_label(&mut tokens, "NAME") {
            return None;
        }
        let name: Box<str> = tokens.next()?.into();
        if !expect_sub_label(&mut tokens, "SIZE") {
            return None;
        }
        let size = tokens.next()?.parse::<i32>().ok()?;
        if !expect_sub_label(&mut tokens, "BOLD") {
            return None;
        }
        let bold = tokens.next()?.parse::<i32>().ok()? != 0;
        tokens
            .next()
            .is_none()
            .then_some(WndFont { name, size, bold })
    })();
    if font.is_none() {
        push_malformed(
            field,
            id,
            "expected NAME: <name>, SIZE: <integer>, BOLD: <integer>",
            diagnostics,
        );
    }
    font
}

fn next_bool<'a>(tokens: &mut impl Iterator<Item = &'a str>) -> Option<bool> {
    Some(tokens.next()?.parse::<i32>().ok()? != 0)
}

fn next_int<'a>(tokens: &mut impl Iterator<Item = &'a str>) -> Option<i32> {
    tokens.next()?.parse::<i32>().ok()
}

/// Skips a sub-label position without checking its spelling.
///
/// This mirrors the source, whose gadget-data parsers `strtok` past each label and read the
/// value positionally; a layout may spell a label anything and the legacy runtime still
/// accepts it. Validating labels here would reject files the game itself loads. The one
/// label the source genuinely tests is `SCROLLIFATEND`, handled explicitly below.
fn skip_sub_label<'a>(tokens: &mut impl Iterator<Item = &'a str>) -> Option<()> {
    tokens.next().map(|_| ())
}

fn labeled_bool<'a>(tokens: &mut impl Iterator<Item = &'a str>) -> Option<bool> {
    skip_sub_label(tokens)?;
    next_bool(tokens)
}

fn labeled_int<'a>(tokens: &mut impl Iterator<Item = &'a str>) -> Option<i32> {
    skip_sub_label(tokens)?;
    next_int(tokens)
}

/// Decodes a record that is one label followed by one integer.
fn parse_single_int(
    field: &WndField,
    id: usize,
    diagnostics: &mut Vec<WndDiagnostic>,
) -> Option<i32> {
    let mut tokens = significant_tokens(field.tokens());
    let value = labeled_int(&mut tokens).filter(|_| tokens.next().is_none());
    if value.is_none() {
        push_malformed(field, id, "expected <label>: <integer>", diagnostics);
    }
    value
}

/// Decodes a draw-data array: exactly nine `IMAGE`/`COLOR`/`BORDERCOLOR` triples.
fn parse_draw_data(
    field: &WndField,
    id: usize,
    diagnostics: &mut Vec<WndDiagnostic>,
) -> Option<WndDrawData> {
    let mut tokens = significant_tokens(field.tokens());
    let data = (|| {
        let mut entries = Vec::with_capacity(WND_DRAW_DATA_ENTRIES);
        for _ in 0..WND_DRAW_DATA_ENTRIES {
            skip_sub_label(&mut tokens)?;
            let image = tokens.next()?;
            skip_sub_label(&mut tokens)?;
            let color = next_color(&mut tokens)?;
            skip_sub_label(&mut tokens)?;
            let border_color = next_color(&mut tokens)?;
            entries.push(WndDrawEntry {
                // `NoImage` is the source's explicit "no mapped image" sentinel.
                image: (image != "NoImage").then(|| image.into()),
                color,
                border_color,
            });
        }
        tokens.next().is_none().then_some(WndDrawData { entries })
    })();
    if data.is_none() {
        push_malformed(
            field,
            id,
            "expected exactly nine IMAGE/COLOR/BORDERCOLOR entries",
            diagnostics,
        );
    }
    data
}

/// Decodes `LISTBOXDATA`, whose `SCROLLIFATEND` sub-record is optional and whose
/// `COLUMNSWIDTH` entries appear only when `COLUMNS` exceeds one.
fn parse_list_box_data(
    field: &WndField,
    id: usize,
    diagnostics: &mut Vec<WndDiagnostic>,
) -> Option<WndGadgetData> {
    let mut tokens = significant_tokens(field.tokens()).peekable();
    let mut over_limit = false;
    let data = (|| {
        let length = labeled_int(&mut tokens)?;
        let auto_scroll = labeled_bool(&mut tokens)?;
        // The source peeks this label and only consumes a value when it names
        // SCROLLIFATEND, matched case-insensitively; otherwise the label it just read is
        // AUTOPURGE's. Five retail records omit it.
        let scroll_if_at_end = if tokens
            .peek()
            .is_some_and(|label| label.eq_ignore_ascii_case("SCROLLIFATEND"))
        {
            skip_sub_label(&mut tokens)?;
            Some(next_bool(&mut tokens)?)
        } else {
            None
        };
        let auto_purge = labeled_bool(&mut tokens)?;
        let scroll_bar = labeled_bool(&mut tokens)?;
        let multi_select = labeled_bool(&mut tokens)?;
        let columns = labeled_int(&mut tokens)?;
        let mut column_widths = Vec::new();
        if columns > 1 {
            let count = usize::try_from(columns).ok()?;
            if count > MAXIMUM_LIST_COLUMNS {
                over_limit = true;
                return None;
            }
            for _ in 0..count {
                column_widths.push(labeled_int(&mut tokens)?);
            }
        }
        let force_select = labeled_bool(&mut tokens)?;
        tokens
            .next()
            .is_none()
            .then_some(WndGadgetData::ListBox(WndListBoxData {
                length,
                auto_scroll,
                scroll_if_at_end,
                auto_purge,
                scroll_bar,
                multi_select,
                columns,
                column_widths,
                force_select,
            }))
    })();
    if data.is_none() {
        let reason = if over_limit {
            "COLUMNS exceeds the column limit"
        } else {
            "expected LENGTH, AUTOSCROLL, optional SCROLLIFATEND, AUTOPURGE, SCROLLBAR, MULTISELECT, COLUMNS with one width per column above one, and FORCESELECT"
        };
        push_malformed(field, id, reason, diagnostics);
    }
    data
}

/// Decodes `IMAGEOFFSET`, which is two bare integers with no sub-labels. Note the source
/// splits this record on whitespace only, unlike the label-bearing records.
fn parse_image_offset(
    field: &WndField,
    id: usize,
    diagnostics: &mut Vec<WndDiagnostic>,
) -> Option<(i32, i32)> {
    let mut tokens = significant_tokens(field.tokens());
    let offset = (|| {
        let x = next_int(&mut tokens)?;
        let y = next_int(&mut tokens)?;
        tokens.next().is_none().then_some((x, y))
    })();
    if offset.is_none() {
        push_malformed(field, id, "expected two integers", diagnostics);
    }
    offset
}

/// Decodes `TABCONTROLDATA`, whose trailing pane flags are counted by the record itself.
fn parse_tab_control_data(
    field: &WndField,
    id: usize,
    diagnostics: &mut Vec<WndDiagnostic>,
) -> Option<WndGadgetData> {
    let mut tokens = significant_tokens(field.tokens());
    let mut over_limit = false;
    let data = (|| {
        let tab_orientation = labeled_int(&mut tokens)?;
        let tab_edge = labeled_int(&mut tokens)?;
        let tab_width = labeled_int(&mut tokens)?;
        let tab_height = labeled_int(&mut tokens)?;
        let tab_count = labeled_int(&mut tokens)?;
        let pane_border = labeled_int(&mut tokens)?;
        let declared = labeled_int(&mut tokens)?;
        let count = usize::try_from(declared).ok()?;
        if count > WND_TAB_PANES {
            over_limit = true;
            return None;
        }
        let mut pane_disabled = Vec::with_capacity(count);
        for _ in 0..count {
            pane_disabled.push(next_bool(&mut tokens)?);
        }
        tokens
            .next()
            .is_none()
            .then_some(WndGadgetData::TabControl(WndTabControlData {
                tab_orientation,
                tab_edge,
                tab_width,
                tab_height,
                tab_count,
                pane_border,
                pane_disabled,
            }))
    })();
    if data.is_none() {
        let reason = if over_limit {
            "PANEDISABLED count exceeds the pane limit"
        } else {
            "expected TABORIENTATION, TABEDGE, TABWIDTH, TABHEIGHT, TABCOUNT, PANEBORDER, and PANEDISABLED with one flag per counted pane"
        };
        push_malformed(field, id, reason, diagnostics);
    }
    data
}

/// Decodes `COMBOBOXDATA`.
fn parse_combo_box_data(
    field: &WndField,
    id: usize,
    diagnostics: &mut Vec<WndDiagnostic>,
) -> Option<WndGadgetData> {
    let mut tokens = significant_tokens(field.tokens());
    let data = (|| {
        let decoded = WndComboBoxData {
            is_editable: labeled_bool(&mut tokens)?,
            maximum_characters: labeled_int(&mut tokens)?,
            maximum_display: labeled_int(&mut tokens)?,
            ascii_only: labeled_bool(&mut tokens)?,
            letters_and_numbers_only: labeled_bool(&mut tokens)?,
        };
        tokens
            .next()
            .is_none()
            .then_some(WndGadgetData::ComboBox(decoded))
    })();
    if data.is_none() {
        push_malformed(
            field,
            id,
            "expected ISEDITABLE, MAXCHARS, MAXDISPLAY, ASCIIONLY, LETTERSANDNUMBERS",
            diagnostics,
        );
    }
    data
}

/// Decodes `SLIDERDATA`.
fn parse_slider_data(
    field: &WndField,
    id: usize,
    diagnostics: &mut Vec<WndDiagnostic>,
) -> Option<WndGadgetData> {
    let mut tokens = significant_tokens(field.tokens());
    let data = (|| {
        let decoded = WndSliderData {
            minimum: labeled_int(&mut tokens)?,
            maximum: labeled_int(&mut tokens)?,
        };
        tokens
            .next()
            .is_none()
            .then_some(WndGadgetData::Slider(decoded))
    })();
    if data.is_none() {
        push_malformed(field, id, "expected MINVALUE and MAXVALUE", diagnostics);
    }
    data
}

/// Decodes `TEXTENTRYDATA`.
fn parse_text_entry_data(
    field: &WndField,
    id: usize,
    diagnostics: &mut Vec<WndDiagnostic>,
) -> Option<WndGadgetData> {
    let mut tokens = significant_tokens(field.tokens());
    let data = (|| {
        let decoded = WndTextEntryData {
            maximum_length: labeled_int(&mut tokens)?,
            secret_text: labeled_bool(&mut tokens)?,
            numerical_only: labeled_bool(&mut tokens)?,
            alphanumerical_only: labeled_bool(&mut tokens)?,
            ascii_only: labeled_bool(&mut tokens)?,
        };
        tokens
            .next()
            .is_none()
            .then_some(WndGadgetData::TextEntry(decoded))
    })();
    if data.is_none() {
        push_malformed(
            field,
            id,
            "expected MAXLEN, SECRETTEXT, NUMERICALONLY, ALPHANUMERICALONLY, ASCIIONLY",
            diagnostics,
        );
    }
    data
}

/// Decodes the six labeled state colors of a `TEXTCOLOR` record.
fn parse_text_colors(
    field: &WndField,
    id: usize,
    diagnostics: &mut Vec<WndDiagnostic>,
) -> Option<WndTextColors> {
    let mut tokens = significant_tokens(field.tokens());
    let colors = (|| {
        let mut decoded = [None; 6];
        for (slot, label) in decoded.iter_mut().zip(TEXT_COLOR_LABELS) {
            if !expect_sub_label(&mut tokens, label) {
                return None;
            }
            *slot = Some(next_color(&mut tokens)?);
        }
        if tokens.next().is_some() {
            return None;
        }
        Some(WndTextColors {
            enabled: decoded[0]?,
            enabled_border: decoded[1]?,
            disabled: decoded[2]?,
            disabled_border: decoded[3]?,
            hilite: decoded[4]?,
            hilite_border: decoded[5]?,
        })
    })();
    if colors.is_none() {
        push_malformed(
            field,
            id,
            "expected ENABLED/ENABLEDBORDER/DISABLED/DISABLEDBORDER/HILITE/HILITEBORDER, each with four 0-255 channels",
            diagnostics,
        );
    }
    colors
}

#[allow(clippy::too_many_lines)]
fn parse_window(
    cursor: &mut Cursor<'_>,
    limits: WndLimits,
    state: &mut ParseState,
    diagnostics: &mut Vec<WndDiagnostic>,
    depth: usize,
    window_line: usize,
) -> Result<WndWindow, WndError> {
    if depth > limits.maximum_depth {
        return Err(WndError::TooDeeplyNested {
            limit: limits.maximum_depth,
        });
    }
    let id = state.windows_seen;
    state.windows_seen = state
        .windows_seen
        .checked_add(1)
        .ok_or(WndError::TooManyWindows {
            limit: limits.maximum_windows,
        })?;
    if state.windows_seen > limits.maximum_windows {
        return Err(WndError::TooManyWindows {
            limit: limits.maximum_windows,
        });
    }

    let type_keyword = cursor.next_token(limits)?;
    if type_keyword.text != b"WINDOWTYPE" {
        return Err(WndError::MissingWindowType {
            line: type_keyword.line,
        });
    }
    expect_equals(cursor, limits, type_keyword.line)?;
    let window_type = cursor.read_record(limits)?.raw_value;
    if !is_known_style(&window_type) {
        diagnostics.push(WndDiagnostic {
            line: type_keyword.line,
            window_id: Some(id),
            kind: WndDiagnosticKind::UnrecognizedValue {
                field: "WINDOWTYPE".into(),
                value: window_type.clone(),
            },
        });
    }

    let mut rect = None;
    let mut typed = TypedFields::default();
    let mut fields = Vec::new();
    let mut children = Vec::new();
    let mut saw_endallchildren = false;
    let mut child_list_open = false;

    loop {
        let token = cursor.next_token(limits)?;
        if token.text == b"END" {
            if !children.is_empty() && !saw_endallchildren {
                return Err(WndError::MissingEndAllChildren { line: window_line });
            }
            break;
        }
        if token.text == b"ENDALLCHILDREN" {
            saw_endallchildren = true;
            child_list_open = false;
            continue;
        }
        if token.text == b"CHILD" {
            child_list_open = true;
            let child_token = cursor.next_token(limits)?;
            if child_token.text != b"WINDOW" {
                return Err(WndError::ExpectedChildWindow {
                    line: child_token.line,
                });
            }
            let child = parse_window(
                cursor,
                limits,
                state,
                diagnostics,
                depth + 1,
                child_token.line,
            )?;
            children.push(child);
            continue;
        }
        // Once `CHILD` has opened the child list, a bare `WINDOW` starts the next
        // sibling: the source's child-list loop dispatches on `WINDOW` and never
        // examines `CHILD`, so the marker is optional there. Outside an open child list
        // this token stays a field name and fails below, matching the source's separate
        // field loop.
        if child_list_open && token.text == b"WINDOW" {
            diagnostics.push(WndDiagnostic {
                line: token.line,
                window_id: Some(id),
                kind: WndDiagnosticKind::MissingChildKeyword,
            });
            let child = parse_window(cursor, limits, state, diagnostics, depth + 1, token.line)?;
            children.push(child);
            continue;
        }
        if token.text == b"SCREENRECT" {
            expect_equals(cursor, limits, token.line)?;
            let record = cursor.read_record(limits)?;
            rect = Some(parse_screen_rect(&record.tokens, token.line)?);
            continue;
        }
        let field_name = decode_token(&token)?;
        expect_equals(cursor, limits, token.line)?;
        let record = cursor.read_record(limits)?;
        let field = WndField {
            name: field_name.into(),
            raw_value: record.raw_value,
            tokens: record.tokens,
            line: token.line,
        };
        typed.absorb(&field, id, diagnostics);
        fields.push(field);
    }

    let rect = rect.ok_or(WndError::MissingScreenRect { line: window_line })?;

    let window = WndWindow {
        id,
        window_type,
        rect,
        typed,
        fields,
        children,
    };
    if let Some(control) = window.control_name() {
        match state.control_names.get(control) {
            Some(&first_window_id) => diagnostics.push(WndDiagnostic {
                line: window_line,
                window_id: Some(id),
                kind: WndDiagnosticKind::DuplicateWindowName {
                    name: window.name().unwrap_or_default().into(),
                    first_window_id,
                },
            }),
            None => {
                state.control_names.insert(control.into(), id);
            }
        }
    }
    Ok(window)
}

/// Tokenizes a bare record value (no trailing `;`) using the WND record lexer, so a value
/// supplied by a patch is retained exactly as an authored one would be.
///
/// # Errors
///
/// Returns [`WndError`] when the value exceeds a limit or contains an unterminated string.
pub(crate) fn tokenize_record_value(
    text: &str,
    limits: WndLimits,
) -> Result<(Box<str>, Vec<WndToken>), WndError> {
    let mut terminated = String::with_capacity(text.len() + 1);
    terminated.push_str(text);
    terminated.push(';');
    let mut cursor = Cursor::new(terminated.as_bytes());
    let record = cursor.read_record(limits)?;
    Ok((record.raw_value, record.tokens))
}

impl WndField {
    /// Builds a field from a name and an unparsed value, for patch-supplied records.
    pub(crate) fn from_patch_value(
        name: &str,
        value: &str,
        line: usize,
        limits: WndLimits,
    ) -> Result<Self, WndError> {
        let (raw_value, tokens) = tokenize_record_value(value, limits)?;
        Ok(Self {
            name: name.into(),
            raw_value,
            tokens,
            line,
        })
    }
}

impl WndWindow {
    /// Returns this window or a descendant whose full decorated name matches exactly.
    ///
    /// Windows whose control part is empty (`"OptionsMenu.wnd:"`, 126 of 1,667 in retail)
    /// are never matched: several windows in one layout share that spelling, so targeting
    /// one would be ambiguous. A patch must name a uniquely identified control.
    pub(crate) fn find_by_decorated_name_mut(&mut self, name: &str) -> Option<&mut Self> {
        if self.control_name().is_some() && self.name() == Some(name) {
            return Some(self);
        }
        self.children
            .iter_mut()
            .find_map(|child| child.find_by_decorated_name_mut(name))
    }

    pub(crate) fn field_mut(&mut self, name: &str) -> Option<&mut WndField> {
        self.fields.iter_mut().find(|field| &*field.name == name)
    }

    pub(crate) fn push_field(&mut self, field: WndField) {
        self.fields.push(field);
    }

    pub(crate) fn set_rect(&mut self, rect: WndScreenRect) {
        self.rect = rect;
    }

    pub(crate) fn children_mut(&mut self) -> &mut Vec<Self> {
        &mut self.children
    }

    /// Returns whether this window or any descendant carries `control` as its control name.
    pub(crate) fn subtree_contains(&self, control: &str) -> bool {
        if self.control_name().is_some() && self.name() == Some(control) {
            return true;
        }
        self.children
            .iter()
            .any(|child| child.subtree_contains(control))
    }

    /// Appends every non-empty decorated name in this subtree, in source order.
    pub(crate) fn collect_decorated_names(&self, out: &mut Vec<Box<str>>) {
        if self.control_name().is_some()
            && let Some(name) = self.name()
        {
            out.push(name.into());
        }
        for child in &self.children {
            child.collect_decorated_names(out);
        }
    }

    /// Returns the highest source-order id in this subtree.
    pub(crate) fn maximum_id(&self) -> usize {
        self.children
            .iter()
            .map(Self::maximum_id)
            .max()
            .map_or(self.id, |child| child.max(self.id))
    }

    /// Reassigns ids depth-first from `next`, so an inserted subtree cannot collide with
    /// ids the document already uses.
    pub(crate) fn renumber_from(&mut self, next: &mut usize) {
        self.id = *next;
        *next += 1;
        for child in &mut self.children {
            child.renumber_from(next);
        }
    }

    /// Recomputes every typed view from the current field list.
    ///
    /// A patch that rewrites `STATUS` must be visible through [`WndWindow::status`], not
    /// only in the raw field, so typed values are derived again rather than patched in
    /// parallel. Diagnostics raised here describe the patched document.
    pub(crate) fn retype(&mut self, diagnostics: &mut Vec<WndDiagnostic>) {
        let mut typed = TypedFields::default();
        for field in &self.fields {
            typed.absorb(field, self.id, diagnostics);
        }
        self.typed = typed;
    }
}

impl WndDocument {
    pub(crate) fn windows_mut(&mut self) -> &mut Vec<WndWindow> {
        &mut self.windows
    }

    pub(crate) fn push_diagnostics(&mut self, extra: Vec<WndDiagnostic>) {
        self.diagnostics.extend(extra);
    }
}

/// Parses a complete WND document.
///
/// # Errors
///
/// Returns [`WndError`] for truncation, malformed structure, a missing required field, or
/// any explicit [`WndLimits`] excess. Unrecognized field names and out-of-vocabulary values
/// never fail the parse; they are retained and reported through
/// [`WndDocument::diagnostics`].
pub fn parse_wnd(bytes: &[u8], limits: WndLimits) -> Result<WndDocument, WndError> {
    if bytes.len() > limits.maximum_file_bytes {
        return Err(WndError::FileTooLarge {
            size: bytes.len(),
            limit: limits.maximum_file_bytes,
        });
    }
    let mut cursor = Cursor::new(bytes);
    let file_version = read_file_version(&mut cursor, limits)?;
    let layout = if file_version >= 2 {
        Some(read_layout_block(&mut cursor, limits)?)
    } else {
        Some(WndLayoutBlock {
            init: Some("[None]".into()),
            update: Some("[None]".into()),
            shutdown: Some("[None]".into()),
        })
    };

    let mut diagnostics = Vec::new();
    let mut top_level_fields = Vec::new();
    let mut windows = Vec::new();
    let mut state = ParseState {
        windows_seen: 0,
        control_names: BTreeMap::new(),
    };

    while !cursor.at_end() {
        let keyword = cursor.next_token(limits)?;
        if keyword.text == b"END" {
            continue;
        }
        if keyword.text == b"WINDOW" {
            let window = parse_window(
                &mut cursor,
                limits,
                &mut state,
                &mut diagnostics,
                1,
                keyword.line,
            )?;
            windows.push(window);
            continue;
        }
        let name = decode_token(&keyword)?;
        expect_equals(&mut cursor, limits, keyword.line)?;
        let record = cursor.read_record(limits)?;
        if !is_known_top_level_field(name) {
            diagnostics.push(WndDiagnostic {
                line: keyword.line,
                window_id: None,
                kind: WndDiagnosticKind::UnknownField { name: name.into() },
            });
        }
        top_level_fields.push(WndField {
            name: name.into(),
            raw_value: record.raw_value,
            tokens: record.tokens,
            line: keyword.line,
        });
    }

    if windows.is_empty() {
        return Err(WndError::NoWindows);
    }

    Ok(WndDocument {
        file_version,
        layout,
        top_level_fields,
        windows,
        diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        WND_DRAW_DATA_ENTRIES, WND_TAB_PANES, WndCallbackKind, WndDiagnosticKind, WndDrawDataSlot,
        WndError, WndField, WndFlag, WndGadgetData, WndLimits, WndTokenKind, parse_wnd,
    };

    fn positive_fixture() -> &'static [u8] {
        b"FILE_VERSION = 2;\n\
STARTLAYOUTBLOCK\n\
  LAYOUTINIT = SyntheticMenuInit;\n\
  LAYOUTUPDATE = SyntheticMenuUpdate;\n\
  LAYOUTSHUTDOWN = SyntheticMenuShutdown;\n\
ENDLAYOUTBLOCK\n\
WINDOW\n\
  WINDOWTYPE = PUSHBUTTON;\n\
  SCREENRECT = UPPERLEFT: 10 20 BOTTOMRIGHT: 210 70\n\
               CREATIONRESOLUTION: 800 600;\n\
  NAME = \"Synthetic.wnd:ButtonStart\";\n\
  STATUS = ENABLED+IMAGE;\n\
  CHILD\n\
    WINDOW\n\
      WINDOWTYPE = STATICTEXT;\n\
      SCREENRECT = UPPERLEFT: 20 30 BOTTOMRIGHT: 200 50\n\
                   CREATIONRESOLUTION: 800 600;\n\
      NAME = \"Synthetic.wnd:LabelStart\";\n\
      FONT = NAME: \"Times New Roman\", SIZE: 14, BOLD: 0;\n\
    END\n\
  ENDALLCHILDREN\n\
END"
    }

    #[test]
    fn decodes_layout_block_and_nested_window_hierarchy_in_source_order() {
        let document = parse_wnd(positive_fixture(), WndLimits::default()).expect("valid WND");
        assert_eq!(document.file_version(), 2);
        let layout = document.layout().expect("layout block");
        assert_eq!(layout.init(), Some("SyntheticMenuInit"));
        assert_eq!(layout.update(), Some("SyntheticMenuUpdate"));
        assert_eq!(layout.shutdown(), Some("SyntheticMenuShutdown"));

        assert_eq!(document.windows().len(), 1);
        let root = &document.windows()[0];
        assert_eq!(root.id(), 0);
        assert_eq!(root.window_type(), "PUSHBUTTON");
        assert_eq!(root.rect().upper_left(), (10, 20));
        assert_eq!(root.rect().bottom_right(), (210, 70));
        assert_eq!(root.rect().creation_resolution(), (800, 600));
        assert_eq!(root.name(), Some("Synthetic.wnd:ButtonStart"));
        assert_eq!(root.control_name(), Some("ButtonStart"));
        assert_eq!(root.fields().len(), 2);
        assert_eq!(root.fields()[1].name(), "STATUS");
        assert_eq!(root.fields()[1].raw_value(), "ENABLED+IMAGE");
        assert_eq!(root.children().len(), 1);

        let child = &root.children()[0];
        assert_eq!(child.id(), 1);
        assert_eq!(child.window_type(), "STATICTEXT");
        assert_eq!(child.rect().upper_left(), (20, 30));
        assert!(document.diagnostics().is_empty());
    }

    #[test]
    fn version_one_defaults_every_layout_callback_to_the_source_none_literal() {
        let bytes = b"FILE_VERSION = 1;\nWINDOW\n  WINDOWTYPE = PUSHBUTTON;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\nEND\n";
        let document = parse_wnd(bytes, WndLimits::default()).expect("version 1 WND");
        let layout = document.layout().expect("default layout block");
        assert_eq!(layout.init(), Some("[None]"));
        assert_eq!(layout.update(), Some("[None]"));
        assert_eq!(layout.shutdown(), Some("[None]"));
    }

    #[test]
    fn rejects_every_truncated_prefix() {
        let fixture = positive_fixture();
        for length in 0..fixture.len() {
            assert!(
                parse_wnd(&fixture[..length], WndLimits::default()).is_err(),
                "prefix of {length} bytes must fail"
            );
        }
    }

    #[test]
    fn rejects_a_document_with_no_windows() {
        let bytes = b"FILE_VERSION = 2;\nSTARTLAYOUTBLOCK\nENDLAYOUTBLOCK\n";
        assert_eq!(
            parse_wnd(bytes, WndLimits::default()),
            Err(WndError::NoWindows)
        );
    }

    #[test]
    fn enforces_every_limit_before_retention() {
        let default = WndLimits::default();
        let cases: [(&[u8], WndLimits); 6] = [
            (
                b"FILE_VERSION = 1;\n",
                WndLimits {
                    maximum_file_bytes: 4,
                    ..default
                },
            ),
            (
                positive_fixture(),
                WndLimits {
                    maximum_tokens: 3,
                    ..default
                },
            ),
            (
                positive_fixture(),
                WndLimits {
                    maximum_lines: 1,
                    ..default
                },
            ),
            (
                b"FILE_VERSION = 1;\nWINDOW\n  WINDOWTYPE = PUSHBUTTON;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n  DATA = one two three four five six seven eight nine ten eleven twelve;\nEND\n",
                WndLimits {
                    maximum_record_bytes: 8,
                    ..default
                },
            ),
            (
                b"FILE_VERSION = 1;\nWINDOW\n  WINDOWTYPE = ASuperLongWindowTypeNameThatExceedsTheField;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\nEND\n",
                WndLimits {
                    maximum_field_bytes: 4,
                    ..default
                },
            ),
            (
                b"FILE_VERSION = 1;\nWINDOW\n  WINDOWTYPE = PUSHBUTTON;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\nEND\n",
                WndLimits {
                    maximum_windows: 0,
                    ..default
                },
            ),
        ];
        for (index, (bytes, limits)) in cases.into_iter().enumerate() {
            assert!(
                parse_wnd(bytes, limits).is_err(),
                "case {index} unexpectedly accepted"
            );
        }

        let nested = b"FILE_VERSION = 1;\nWINDOW\n  WINDOWTYPE = PUSHBUTTON;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n  CHILD\n    WINDOW\n      WINDOWTYPE = STATICTEXT;\n      SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n    END\n  ENDALLCHILDREN\nEND\n";
        assert!(matches!(
            parse_wnd(
                nested,
                WndLimits {
                    maximum_depth: 1,
                    ..default
                }
            ),
            Err(WndError::TooDeeplyNested { limit: 1 })
        ));
    }

    #[test]
    fn unknown_top_level_and_window_fields_are_retained_and_diagnosed() {
        let bytes = b"FILE_VERSION = 1;\nSOMEUNKNOWNTOPLEVEL = 1;\nWINDOW\n  WINDOWTYPE = PUSHBUTTON;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n  SOMEUNKNOWNWINDOWFIELD = value;\nEND\n";
        let document = parse_wnd(bytes, WndLimits::default()).expect("valid WND");

        assert_eq!(document.top_level_fields().len(), 1);
        assert_eq!(document.top_level_fields()[0].name(), "SOMEUNKNOWNTOPLEVEL");
        assert_eq!(document.top_level_fields()[0].raw_value(), "1");

        let window = &document.windows()[0];
        assert_eq!(window.fields().len(), 1);
        assert_eq!(window.fields()[0].name(), "SOMEUNKNOWNWINDOWFIELD");
        assert_eq!(window.fields()[0].raw_value(), "value");

        assert!(document.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic.kind(),
            WndDiagnosticKind::UnknownField { name } if &**name == "SOMEUNKNOWNTOPLEVEL"
        )));
    }

    #[test]
    fn recognizes_confirmed_top_level_default_visual_keywords_without_diagnostics() {
        let bytes = b"FILE_VERSION = 1;\nENABLEDCOLOR = 255 255 255 255;\nFONT = Arial 10 0;\nWINDOW\n  WINDOWTYPE = PUSHBUTTON;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\nEND\n";
        let document = parse_wnd(bytes, WndLimits::default()).expect("valid WND");
        assert_eq!(document.top_level_fields().len(), 2);
        assert!(document.diagnostics().is_empty());
    }

    #[test]
    fn decodes_multiple_children_each_wrapped_in_its_own_child_keyword() {
        // Every retail WND file precedes each child with its own CHILD keyword (`CHILD WINDOW
        // ... END`, repeated) rather than one CHILD wrapping the whole list, and closes the
        // list with a single ENDALLCHILDREN before the parent's own END.
        let bytes = b"FILE_VERSION = 1;\nWINDOW\n  WINDOWTYPE = USER;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n  CHILD\n    WINDOW\n      WINDOWTYPE = PUSHBUTTON;\n      SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n    END\n  CHILD\n    WINDOW\n      WINDOWTYPE = STATICTEXT;\n      SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n    END\n  ENDALLCHILDREN\nEND\n";
        let document = parse_wnd(bytes, WndLimits::default()).expect("valid WND");
        let root = &document.windows()[0];
        assert_eq!(root.children().len(), 2);
        assert_eq!(root.children()[0].window_type(), "PUSHBUTTON");
        assert_eq!(root.children()[1].window_type(), "STATICTEXT");
    }

    /// A window carrying every field slice 2 types, in the shapes retail authors them.
    fn typed_fixture() -> &'static [u8] {
        b"FILE_VERSION = 2;\n\
STARTLAYOUTBLOCK\n\
  LAYOUTINIT = Init;\n\
  LAYOUTUPDATE = Update;\n\
  LAYOUTSHUTDOWN = Shutdown;\n\
ENDLAYOUTBLOCK\n\
WINDOW\n\
  WINDOWTYPE = PUSHBUTTON;\n\
  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 10 10 CREATIONRESOLUTION: 800 600;\n\
  NAME = \"Synthetic.wnd:ButtonOne\";\n\
  STATUS = ENABLED+IMAGE;\n\
  STYLE = PUSHBUTTON+MOUSETRACK;\n\
  SYSTEMCALLBACK = \"SyntheticSystem\";\n\
  INPUTCALLBACK = \"[None]\";\n\
  TOOLTIPCALLBACK = \"[None]\";\n\
  DRAWCALLBACK = \"SyntheticDraw\";\n\
  FONT = NAME: \"Times New Roman\", SIZE: 14, BOLD: 1;\n\
  HEADERTEMPLATE = \"[NONE]\";\n\
  TOOLTIPDELAY = -1;\n\
  TEXT = \"GUI:Synthetic\";\n\
  TOOLTIPTEXT = \"Tooltip:Synthetic\";\n\
  TEXTCOLOR = ENABLED: 1 2 3 4, ENABLEDBORDER: 5 6 7 8,\n\
              DISABLED: 9 10 11 12, DISABLEDBORDER: 13 14 15 16,\n\
              HILITE: 17 18 19 20, HILITEBORDER: 21 22 23 24;\n\
END\n"
    }

    #[test]
    fn types_every_common_window_field_from_its_established_shape() {
        let document = parse_wnd(typed_fixture(), WndLimits::default()).expect("valid WND");
        let window = &document.windows()[0];
        assert!(
            document.diagnostics().is_empty(),
            "well-formed records must not diagnose: {:?}",
            document.diagnostics()
        );

        assert_eq!(window.name(), Some("Synthetic.wnd:ButtonOne"));
        assert_eq!(
            window
                .status()
                .iter()
                .map(|flag| (flag.name(), flag.is_known()))
                .collect::<Vec<_>>(),
            vec![("ENABLED", true), ("IMAGE", true)]
        );
        assert_eq!(
            window.style().iter().map(WndFlag::name).collect::<Vec<_>>(),
            vec!["PUSHBUTTON", "MOUSETRACK"],
            "STYLE restates the window type alongside its modifiers"
        );

        let callbacks = window.callbacks();
        assert_eq!(
            callbacks.get(WndCallbackKind::System),
            Some("SyntheticSystem")
        );
        assert_eq!(callbacks.get(WndCallbackKind::Input), Some("[None]"));
        assert_eq!(callbacks.get(WndCallbackKind::Draw), Some("SyntheticDraw"));

        let font = window.font().expect("FONT decodes");
        assert_eq!(font.name(), "Times New Roman");
        assert_eq!(font.size(), 14);
        assert!(font.bold());

        assert_eq!(window.header_template(), Some("[NONE]"));
        assert_eq!(window.tooltip_delay(), Some(-1));
        assert_eq!(window.text(), Some("GUI:Synthetic"));
        assert_eq!(window.tooltip_text(), Some("Tooltip:Synthetic"));

        let colors = window.text_colors().expect("TEXTCOLOR decodes");
        assert_eq!(colors.enabled().channels(), [1, 2, 3, 4]);
        assert_eq!(colors.enabled_border().channels(), [5, 6, 7, 8]);
        assert_eq!(colors.disabled().channels(), [9, 10, 11, 12]);
        assert_eq!(colors.disabled_border().channels(), [13, 14, 15, 16]);
        assert_eq!(colors.hilite().channels(), [17, 18, 19, 20]);
        assert_eq!(colors.hilite_border().channels(), [21, 22, 23, 24]);
    }

    #[test]
    fn types_a_draw_data_array_as_nine_entries_with_the_noimage_sentinel() {
        use std::fmt::Write as _;

        let mut value = String::from("  ENABLEDDRAWDATA =");
        for index in 0..9 {
            let image = if index == 0 { "Button-Top" } else { "NoImage" };
            write!(
                value,
                " IMAGE: {image}, COLOR: {index} 2 3 4, BORDERCOLOR: 5 6 7 {index}"
            )
            .expect("writing to a String cannot fail");
            value.push(if index == 8 { ';' } else { ',' });
        }
        let mut bytes = b"FILE_VERSION = 1;\nWINDOW\n  WINDOWTYPE = PUSHBUTTON;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n".to_vec();
        bytes.extend_from_slice(value.as_bytes());
        bytes.extend_from_slice(b"\nEND\n");

        let document = parse_wnd(&bytes, WndLimits::default()).expect("valid WND");
        let window = &document.windows()[0];
        assert!(document.diagnostics().is_empty());

        let data = window
            .draw_data_for(WndDrawDataSlot::Enabled)
            .expect("ENABLEDDRAWDATA decodes");
        assert_eq!(data.entries().len(), WND_DRAW_DATA_ENTRIES);
        assert_eq!(data.entries()[0].image(), Some("Button-Top"));
        assert_eq!(data.entries()[0].color().channels(), [0, 2, 3, 4]);
        assert_eq!(
            data.entries()[1].image(),
            None,
            "NoImage is the absent-image sentinel, not an image named NoImage"
        );
        assert_eq!(data.entries()[8].border_color().channels(), [5, 6, 7, 8]);
        assert_eq!(window.draw_data().len(), 1);
    }

    #[test]
    fn rejects_a_draw_data_array_that_is_not_exactly_nine_entries() {
        let bytes = b"FILE_VERSION = 1;\nWINDOW\n  WINDOWTYPE = PUSHBUTTON;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n  HILITEDRAWDATA = IMAGE: NoImage, COLOR: 1 2 3 4, BORDERCOLOR: 5 6 7 8;\nEND\n";
        let document = parse_wnd(bytes, WndLimits::default()).expect("not fatal");
        assert!(document.windows()[0].draw_data().is_empty());
        assert!(document.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic.kind(),
            WndDiagnosticKind::MalformedField { field, .. } if &**field == "HILITEDRAWDATA"
        )));
    }

    #[test]
    fn types_every_gadget_data_record() {
        let cases: [(&[u8], &str, WndGadgetData); 4] = [
            (
                b"  SLIDERDATA = MINVALUE: 15, MAXVALUE: 61;\n",
                "HORZSLIDER",
                WndGadgetData::Slider(super::WndSliderData {
                    minimum: 15,
                    maximum: 61,
                }),
            ),
            (
                b"  RADIOBUTTONDATA = GROUP: 2;\n",
                "RADIOBUTTON",
                WndGadgetData::RadioButtonGroup(2),
            ),
            (
                b"  STATICTEXTDATA = CENTERED: 1;\n",
                "STATICTEXT",
                WndGadgetData::StaticTextCentered(true),
            ),
            (
                b"  COMBOBOXDATA = ISEDITABLE: 0, MAXCHARS: 16, MAXDISPLAY: 5, ASCIIONLY: 0, LETTERSANDNUMBERS: 0;\n",
                "COMBOBOX",
                WndGadgetData::ComboBox(super::WndComboBoxData {
                    is_editable: false,
                    maximum_characters: 16,
                    maximum_display: 5,
                    ascii_only: false,
                    letters_and_numbers_only: false,
                }),
            ),
        ];
        for (record, window_type, expected) in cases {
            let mut bytes = format!(
                "FILE_VERSION = 1;\nWINDOW\n  WINDOWTYPE = {window_type};\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n"
            )
            .into_bytes();
            bytes.extend_from_slice(record);
            bytes.extend_from_slice(b"END\n");

            let document = parse_wnd(&bytes, WndLimits::default()).expect("valid WND");
            assert!(document.diagnostics().is_empty());
            assert_eq!(document.windows()[0].gadget_data(), Some(&expected));
        }
    }

    #[test]
    fn decodes_listboxdata_with_and_without_its_optional_scrollifatend_sub_record() {
        // The source peeks that label and only consumes a value when it matches, so five
        // retail records omit it entirely. A fixed-sequence decode would misread them.
        let with = b"FILE_VERSION = 1;\nWINDOW\n  WINDOWTYPE = SCROLLLISTBOX;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n  LISTBOXDATA = LENGTH: 100, AUTOSCROLL: 1, SCROLLIFATEND: 1, AUTOPURGE: 0, SCROLLBAR: 1, MULTISELECT: 0, COLUMNS: 2, COLUMNSWIDTH: 40, COLUMNSWIDTH: 60, FORCESELECT: 1;\nEND\n";
        let without = b"FILE_VERSION = 1;\nWINDOW\n  WINDOWTYPE = SCROLLLISTBOX;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n  LISTBOXDATA = LENGTH: 100, AUTOSCROLL: 1, AUTOPURGE: 0, SCROLLBAR: 1, MULTISELECT: 0, COLUMNS: 1, FORCESELECT: 0;\nEND\n";

        let document = parse_wnd(with, WndLimits::default()).expect("valid WND");
        assert!(document.diagnostics().is_empty());
        let Some(WndGadgetData::ListBox(list)) = document.windows()[0].gadget_data() else {
            panic!("expected list box data");
        };
        assert_eq!(list.scroll_if_at_end(), Some(true));
        assert_eq!(list.columns(), 2);
        assert_eq!(list.column_widths(), [40, 60]);
        assert!(list.force_select());

        let document = parse_wnd(without, WndLimits::default()).expect("valid WND");
        assert!(document.diagnostics().is_empty());
        let Some(WndGadgetData::ListBox(list)) = document.windows()[0].gadget_data() else {
            panic!("expected list box data");
        };
        assert_eq!(
            list.scroll_if_at_end(),
            None,
            "an omitted optional sub-record is absent, not false"
        );
        assert_eq!(list.columns(), 1);
        assert!(
            list.column_widths().is_empty(),
            "a single-column list declares no widths"
        );
        assert!(!list.force_select());
    }

    #[test]
    fn bounds_listboxdata_column_widths_against_an_attacker_controlled_count() {
        let bytes = b"FILE_VERSION = 1;\nWINDOW\n  WINDOWTYPE = SCROLLLISTBOX;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n  LISTBOXDATA = LENGTH: 1, AUTOSCROLL: 0, AUTOPURGE: 0, SCROLLBAR: 0, MULTISELECT: 0, COLUMNS: 100000, FORCESELECT: 0;\nEND\n";
        let document = parse_wnd(bytes, WndLimits::default()).expect("not fatal");
        assert!(document.windows()[0].gadget_data().is_none());
        assert!(document.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic.kind(),
            WndDiagnosticKind::MalformedField { field, reason }
                if &**field == "LISTBOXDATA" && reason.contains("column limit")
        )));
    }

    #[test]
    fn decodes_the_two_records_that_appear_only_in_source_and_never_in_retail_data() {
        // IMAGEOFFSET is two bare integers with no sub-labels; TABCONTROLDATA counts its own
        // trailing pane flags. Neither occurs in either retail edition, so these shapes rest
        // on the source alone.
        let bytes = b"FILE_VERSION = 1;\nWINDOW\n  WINDOWTYPE = TABCONTROL;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n  IMAGEOFFSET = -3 7;\n  TABCONTROLDATA = TABORIENTATION: 1, TABEDGE: 2, TABWIDTH: 30, TABHEIGHT: 40, TABCOUNT: 3, PANEBORDER: 5, PANEDISABLED: 3 0 1 0;\nEND\n";
        let document = parse_wnd(bytes, WndLimits::default()).expect("valid WND");
        assert!(document.diagnostics().is_empty());

        let window = &document.windows()[0];
        assert_eq!(window.image_offset(), Some((-3, 7)));
        let Some(WndGadgetData::TabControl(tabs)) = window.gadget_data() else {
            panic!("expected tab control data");
        };
        assert_eq!(tabs.tab_orientation(), 1);
        assert_eq!(tabs.tab_height(), 40);
        assert_eq!(tabs.pane_border(), 5);
        assert_eq!(tabs.pane_disabled(), [false, true, false]);
    }

    #[test]
    fn bounds_tab_control_pane_flags_where_the_source_would_overflow_its_array() {
        // The source reads PANEDISABLED's count from the file and writes that many entries
        // into a fixed NUM_TAB_PANES array without checking it.
        let bytes = b"FILE_VERSION = 1;\nWINDOW\n  WINDOWTYPE = TABCONTROL;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n  TABCONTROLDATA = TABORIENTATION: 1, TABEDGE: 2, TABWIDTH: 30, TABHEIGHT: 40, TABCOUNT: 3, PANEBORDER: 5, PANEDISABLED: 99999;\nEND\n";
        let document = parse_wnd(bytes, WndLimits::default()).expect("not fatal");
        assert!(document.windows()[0].gadget_data().is_none());
        assert!(document.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic.kind(),
            WndDiagnosticKind::MalformedField { field, reason }
                if &**field == "TABCONTROLDATA" && reason.contains("pane limit")
        )));

        // The bound is the source's own array width, not an arbitrary choice.
        let at_limit = format!(
            "FILE_VERSION = 1;\nWINDOW\n  WINDOWTYPE = TABCONTROL;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n  TABCONTROLDATA = TABORIENTATION: 1, TABEDGE: 2, TABWIDTH: 30, TABHEIGHT: 40, TABCOUNT: 3, PANEBORDER: 5, PANEDISABLED: {WND_TAB_PANES}{};\nEND\n",
            " 0".repeat(WND_TAB_PANES)
        );
        let document = parse_wnd(at_limit.as_bytes(), WndLimits::default()).expect("valid WND");
        assert!(document.diagnostics().is_empty());
        let Some(WndGadgetData::TabControl(tabs)) = document.windows()[0].gadget_data() else {
            panic!("expected tab control data");
        };
        assert_eq!(tabs.pane_disabled().len(), WND_TAB_PANES);
    }

    #[test]
    fn leaves_the_stubbed_tooltip_record_generic() {
        // The source's parseTooltip ignores its buffer and stores a placeholder, marked
        // @todo, so there is no grammar to decode. Retaining it generically is the honest
        // outcome; inventing a shape would be fabrication.
        let bytes = b"FILE_VERSION = 1;\nWINDOW\n  WINDOWTYPE = PUSHBUTTON;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n  TOOLTIP = whatever the author wrote here;\nEND\n";
        let document = parse_wnd(bytes, WndLimits::default()).expect("valid WND");
        let window = &document.windows()[0];
        assert_eq!(window.fields()[0].name(), "TOOLTIP");
        assert_eq!(
            window.fields()[0].raw_value(),
            "whatever the author wrote here"
        );
        assert!(
            document.diagnostics().is_empty(),
            "an untyped record is not a malformed one"
        );
    }

    #[test]
    fn does_not_validate_gadget_sub_label_spelling() {
        // The source strtoks past every label without comparing it, so a layout may spell
        // them anything and the legacy runtime still loads. Rejecting those would refuse
        // files the game itself accepts.
        let bytes = b"FILE_VERSION = 1;\nWINDOW\n  WINDOWTYPE = HORZSLIDER;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n  SLIDERDATA = LOWEST: 3, HIGHEST: 9;\nEND\n";
        let document = parse_wnd(bytes, WndLimits::default()).expect("valid WND");
        assert!(document.diagnostics().is_empty());
        assert_eq!(
            document.windows()[0].gadget_data(),
            Some(&WndGadgetData::Slider(super::WndSliderData {
                minimum: 3,
                maximum: 9,
            }))
        );
    }

    #[test]
    fn typing_a_field_never_removes_it_from_the_retained_field_list() {
        let document = parse_wnd(typed_fixture(), WndLimits::default()).expect("valid WND");
        let window = &document.windows()[0];
        let names = window
            .fields()
            .iter()
            .map(WndField::name)
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "NAME",
                "STATUS",
                "STYLE",
                "SYSTEMCALLBACK",
                "INPUTCALLBACK",
                "TOOLTIPCALLBACK",
                "DRAWCALLBACK",
                "FONT",
                "HEADERTEMPLATE",
                "TOOLTIPDELAY",
                "TEXT",
                "TOOLTIPTEXT",
                "TEXTCOLOR",
            ],
            "typed accessors are views; the generic record stays complete"
        );
    }

    #[test]
    fn a_malformed_typed_record_diagnoses_without_failing_the_document() {
        let cases: [(&[u8], &str); 4] = [
            (
                b"  FONT = NAME: \"Arial\", SIZE: notanumber, BOLD: 0;\n",
                "FONT",
            ),
            (b"  TOOLTIPDELAY = soon;\n", "TOOLTIPDELAY"),
            (
                b"  TEXTCOLOR = ENABLED: 1 2 3 4, ENABLEDBORDER: 5 6 7 8;\n",
                "TEXTCOLOR",
            ),
            (
                // 300 exceeds a color channel's range.
                b"  TEXTCOLOR = ENABLED: 300 2 3 4, ENABLEDBORDER: 5 6 7 8, DISABLED: 9 10 11 12, DISABLEDBORDER: 13 14 15 16, HILITE: 17 18 19 20, HILITEBORDER: 21 22 23 24;\n",
                "TEXTCOLOR",
            ),
        ];
        for (record, expected_field) in cases {
            let mut bytes = b"FILE_VERSION = 1;\nWINDOW\n  WINDOWTYPE = PUSHBUTTON;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n".to_vec();
            bytes.extend_from_slice(record);
            bytes.extend_from_slice(b"END\n");

            let document =
                parse_wnd(&bytes, WndLimits::default()).expect("a malformed record is not fatal");
            let window = &document.windows()[0];
            assert_eq!(window.fields().len(), 1, "the field is still retained");
            assert!(
                window.font().is_none()
                    && window.text_colors().is_none()
                    && window.tooltip_delay().is_none(),
                "the typed view stays absent"
            );
            assert!(
                document.diagnostics().iter().any(|diagnostic| matches!(
                    diagnostic.kind(),
                    WndDiagnosticKind::MalformedField { field, .. } if &**field == expected_field
                )),
                "{expected_field} must be diagnosed"
            );
        }
    }

    #[test]
    fn retains_quoting_and_punctuation_that_a_flattened_value_would_destroy() {
        // The whole point of the token view: `NAME: "Times New Roman", SIZE: 14` and the
        // same record without quotes are indistinguishable once flattened, so a font name
        // containing spaces cannot be delimited from the following sub-label.
        let document = parse_wnd(positive_fixture(), WndLimits::default()).expect("valid WND");
        let child = &document.windows()[0].children()[0];
        let font = child
            .fields()
            .iter()
            .find(|field| field.name() == "FONT")
            .expect("FONT field");

        assert_eq!(
            font.raw_value(),
            "NAME: \"Times New Roman\", SIZE: 14, BOLD: 0"
        );
        let tokens = font
            .tokens()
            .iter()
            .map(|token| (token.text(), token.kind()))
            .collect::<Vec<_>>();
        assert_eq!(
            tokens,
            vec![
                ("NAME", WndTokenKind::Word),
                (":", WndTokenKind::Punctuation),
                ("Times New Roman", WndTokenKind::Quoted),
                (",", WndTokenKind::Punctuation),
                ("SIZE", WndTokenKind::Word),
                (":", WndTokenKind::Punctuation),
                ("14", WndTokenKind::Word),
                (",", WndTokenKind::Punctuation),
                ("BOLD", WndTokenKind::Word),
                (":", WndTokenKind::Punctuation),
                ("0", WndTokenKind::Word),
            ]
        );
    }

    #[test]
    fn distinguishes_a_quoted_value_from_the_same_characters_written_bare() {
        let quoted = b"FILE_VERSION = 1;\nWINDOW\n  WINDOWTYPE = PUSHBUTTON;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n  TEXT = \"Times New Roman\";\nEND\n";
        let bare = b"FILE_VERSION = 1;\nWINDOW\n  WINDOWTYPE = PUSHBUTTON;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n  TEXT = Times New Roman;\nEND\n";

        let quoted_field = {
            let document = parse_wnd(quoted, WndLimits::default()).expect("valid WND");
            document.windows()[0].fields()[0].clone()
        };
        let bare_field = {
            let document = parse_wnd(bare, WndLimits::default()).expect("valid WND");
            document.windows()[0].fields()[0].clone()
        };

        assert_eq!(quoted_field.tokens().len(), 1);
        assert!(quoted_field.tokens()[0].is_quoted());
        assert_eq!(quoted_field.tokens()[0].text(), "Times New Roman");

        assert_eq!(bare_field.tokens().len(), 3);
        assert!(bare_field.tokens().iter().all(|token| !token.is_quoted()));

        assert_ne!(quoted_field.raw_value(), bare_field.raw_value());
    }

    #[test]
    fn splits_plus_separated_status_flags_and_diagnoses_unknown_names() {
        // `+` is the separator retail uses (`ENABLED+NOFOCUS+SEE_THRU`). ON_MOUSE_DOWN is a
        // Zero Hour-only name and must not be reported against a Zero Hour install.
        let bytes = b"FILE_VERSION = 1;\nWINDOW\n  WINDOWTYPE = PUSHBUTTON;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n  STATUS = ENABLED+ON_MOUSE_DOWN+NOT_A_REAL_FLAG+see_thru;\n  STYLE = PUSHBUTTON+MOUSETRACK+ALSO_NOT_REAL;\nEND\n";
        let document = parse_wnd(bytes, WndLimits::default()).expect("valid WND");

        let status = &document.windows()[0].fields()[0];
        assert_eq!(status.tokens().len(), 7);
        assert_eq!(
            status
                .tokens()
                .iter()
                .filter(|token| !token.is_punctuation())
                .count(),
            4
        );

        let unrecognized = document
            .diagnostics()
            .iter()
            .filter_map(|diagnostic| match diagnostic.kind() {
                WndDiagnosticKind::UnrecognizedValue { field, value } => {
                    Some((field.to_string(), value.to_string()))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            unrecognized,
            vec![
                ("STATUS".to_owned(), "NOT_A_REAL_FLAG".to_owned()),
                ("STYLE".to_owned(), "ALSO_NOT_REAL".to_owned()),
            ],
            "known names in either edition, in any case, must not be reported"
        );
    }

    #[test]
    fn diagnoses_a_repeated_control_name_without_failing_the_document() {
        let bytes = b"FILE_VERSION = 1;\nWINDOW\n  WINDOWTYPE = USER;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n  NAME = \"Menu.wnd:Repeated\";\n  CHILD\n    WINDOW\n      WINDOWTYPE = PUSHBUTTON;\n      SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n      NAME = \"Menu.wnd:Repeated\";\n    END\n  ENDALLCHILDREN\nEND\n";
        let document = parse_wnd(bytes, WndLimits::default()).expect("duplicates are not fatal");

        let duplicates = document
            .diagnostics()
            .iter()
            .filter(|diagnostic| {
                matches!(
                    diagnostic.kind(),
                    WndDiagnosticKind::DuplicateWindowName { .. }
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(duplicates.len(), 1);
        assert_eq!(
            duplicates[0].kind(),
            &WndDiagnosticKind::DuplicateWindowName {
                name: "Menu.wnd:Repeated".into(),
                first_window_id: 1,
            },
            "the inner window closes first, so it registers the name first"
        );
    }

    #[test]
    fn treats_an_empty_control_part_as_unnamed_rather_than_a_shared_name() {
        // 126 of 1,667 retail windows declare only the layout prefix. Counting those as a
        // shared name would report 31 retail layouts as having duplicates.
        let bytes = b"FILE_VERSION = 1;\nWINDOW\n  WINDOWTYPE = USER;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n  NAME = \"Menu.wnd:\";\n  CHILD\n    WINDOW\n      WINDOWTYPE = PUSHBUTTON;\n      SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n      NAME = \"Menu.wnd:\";\n    END\n  ENDALLCHILDREN\nEND\n";
        let document = parse_wnd(bytes, WndLimits::default()).expect("valid WND");

        let root = &document.windows()[0];
        assert_eq!(root.name(), Some("Menu.wnd:"));
        assert_eq!(root.control_name(), None);
        assert!(document.diagnostics().is_empty());
    }

    #[test]
    fn bounds_the_per_record_token_vector_independently_of_record_bytes() {
        let bytes = b"FILE_VERSION = 1;\nWINDOW\n  WINDOWTYPE = PUSHBUTTON;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n  DATA = a b c d e f g h;\nEND\n";
        assert!(matches!(
            parse_wnd(
                bytes,
                WndLimits {
                    maximum_record_tokens: 4,
                    ..WndLimits::default()
                }
            ),
            Err(WndError::TooManyRecordTokens { limit: 4, .. })
        ));
    }

    #[test]
    fn accepts_a_screenrect_written_with_comma_separated_sub_records() {
        // Retail authors SCREENRECT both with and without commas between sub-records.
        let bytes = b"FILE_VERSION = 1;\nWINDOW\n  WINDOWTYPE = PUSHBUTTON;\n  SCREENRECT = UPPERLEFT: 540 316,\n               BOTTOMRIGHT: 748 351,\n               CREATIONRESOLUTION: 800 600;\nEND\n";
        let document = parse_wnd(bytes, WndLimits::default()).expect("valid WND");
        let rect = document.windows()[0].rect();
        assert_eq!(rect.upper_left(), (540, 316));
        assert_eq!(rect.bottom_right(), (748, 351));
        assert_eq!(rect.creation_resolution(), (800, 600));
    }

    #[test]
    fn accepts_a_sibling_window_declared_without_its_own_child_keyword() {
        // The source child-list loop dispatches on WINDOW and has no CHILD case, so CHILD
        // is an optional marker once the list is open. Retail Zero Hour MainMenu.wnd
        // contains exactly one such sibling; rejecting it made that file undecodable.
        let bytes = b"FILE_VERSION = 1;\nWINDOW\n  WINDOWTYPE = USER;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n  CHILD\n    WINDOW\n      WINDOWTYPE = PUSHBUTTON;\n      SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n    END\n    WINDOW\n      WINDOWTYPE = STATICTEXT;\n      SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n    END\n  ENDALLCHILDREN\nEND\n";
        let document = parse_wnd(bytes, WndLimits::default()).expect("valid WND");
        let root = &document.windows()[0];
        assert_eq!(root.children().len(), 2);
        assert_eq!(root.children()[0].window_type(), "PUSHBUTTON");
        assert_eq!(root.children()[1].window_type(), "STATICTEXT");
        assert_eq!(root.children()[1].id(), 2);

        let diagnostics = document.diagnostics();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].kind(),
            &WndDiagnosticKind::MissingChildKeyword
        );
        assert_eq!(diagnostics[0].window_id(), Some(0));
        assert_eq!(diagnostics[0].line(), 10);
    }

    #[test]
    fn does_not_treat_window_as_a_child_before_any_child_list_is_open() {
        // Outside an open child list the source reads fields, not windows, so a bare
        // WINDOW here is a field name missing its '=' rather than an implicit child.
        let bytes = b"FILE_VERSION = 1;\nWINDOW\n  WINDOWTYPE = USER;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n  WINDOW\n    WINDOWTYPE = PUSHBUTTON;\n    SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n  END\nEND\n";
        assert!(matches!(
            parse_wnd(bytes, WndLimits::default()),
            Err(WndError::MissingEquals { line: 5 })
        ));
    }

    #[test]
    fn closes_the_child_list_at_endallchildren_so_a_later_window_is_not_a_child() {
        // ENDALLCHILDREN terminates the source loop, so the marker cannot leak across it.
        let bytes = b"FILE_VERSION = 1;\nWINDOW\n  WINDOWTYPE = USER;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n  CHILD\n    WINDOW\n      WINDOWTYPE = PUSHBUTTON;\n      SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n    END\n  ENDALLCHILDREN\n  WINDOW\n    WINDOWTYPE = STATICTEXT;\n    SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n  END\nEND\n";
        assert!(matches!(
            parse_wnd(bytes, WndLimits::default()),
            Err(WndError::MissingEquals { line: 11 })
        ));
    }

    #[test]
    fn rejects_children_without_a_closing_endallchildren() {
        let bytes = b"FILE_VERSION = 1;\nWINDOW\n  WINDOWTYPE = USER;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n  CHILD\n    WINDOW\n      WINDOWTYPE = PUSHBUTTON;\n      SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n    END\nEND\n";
        assert!(matches!(
            parse_wnd(bytes, WndLimits::default()),
            Err(WndError::MissingEndAllChildren { .. })
        ));
    }
}
