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
//! This first slice covers the source-established file/layout header and the `WINDOW`/
//! `CHILD` hierarchy with `WINDOWTYPE`/`SCREENRECT` typed and every other field preserved
//! generically. Per-gadget typed field decode (fonts, state colors/borders, draw-data
//! arrays, header templates, gadget-specific `DATA`) is deliberately excluded here and
//! belongs to a later slice, alongside mapped-image/font/CSF resource resolution.

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

/// One generically retained `NAME = value;` field, preserved whether or not it is
/// recognized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WndField {
    name: Box<str>,
    raw_value: Box<str>,
    line: usize,
}

impl WndField {
    /// Returns the field name exactly as spelled in the source.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the field's value, whitespace-collapsed and trimmed, exactly as the source
    /// reader's semicolon-terminated record scan produces it.
    #[must_use]
    pub fn raw_value(&self) -> &str {
        &self.raw_value
    }

    /// Returns the one-based source line where the field name appeared.
    #[must_use]
    pub const fn line(&self) -> usize {
        self.line
    }
}

/// One immutable window/gadget declaration, with only `WINDOWTYPE` and `SCREENRECT` typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WndWindow {
    id: usize,
    window_type: Box<str>,
    rect: WndScreenRect,
    fields: Vec<WndField>,
    children: Vec<WndWindow>,
}

impl WndWindow {
    /// Returns a stable, source-order identifier (`0`-based) for this window.
    #[must_use]
    pub const fn id(&self) -> usize {
        self.id
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
    /// A token inside `CHILD`/`ENDALLCHILDREN` was not `WINDOW` or `ENDALLCHILDREN`.
    ExpectedChildWindow { line: usize },
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
                "WND CHILD at line {line} expected WINDOW or ENDALLCHILDREN"
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

struct Token<'a> {
    text: &'a [u8],
    line: usize,
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
                        return Ok(Token { text, line });
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
        if byte == b'=' || byte == b';' {
            self.advance();
            return Ok(Token {
                text: &self.bytes[self.pos - 1..self.pos],
                line,
            });
        }
        let start = self.pos;
        loop {
            match self.peek_byte() {
                Some(next) if !next.is_ascii_whitespace() && next != b'=' && next != b';' => {
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
            line,
        })
    }

    /// Reads tokens until a bare `;`, joining them with single spaces. This mirrors the
    /// source reader's "collapse whitespace runs, trim, stop at `;`" record scan. A missing
    /// semicolon is not specially detected here: like the source, subsequent structural
    /// tokens are folded into the value until a `;` is found or a limit/EOF error occurs.
    fn read_value_until_semicolon(&mut self, limits: WndLimits) -> Result<Box<str>, WndError> {
        let mut value = String::new();
        loop {
            let token = self.next_token(limits)?;
            if token.text == b";" {
                break;
            }
            let text = std::str::from_utf8(token.text)
                .map_err(|_| WndError::InvalidUtf8 { line: token.line })?;
            if !value.is_empty() {
                value.push(' ');
            }
            value.push_str(text);
            if value.len() > limits.maximum_record_bytes {
                return Err(WndError::RecordTooLong {
                    line: token.line,
                    size: value.len(),
                    limit: limits.maximum_record_bytes,
                });
            }
        }
        Ok(value.into_boxed_str())
    }
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
    let value = cursor.next_token(limits)?;
    let text = decode_token(&value)?;
    text.parse::<u32>()
        .map_err(|_| WndError::InvalidFileVersion { line: value.line })
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
        *slot = Some(cursor.read_value_until_semicolon(limits)?);
    }
    Ok(WndLayoutBlock {
        init,
        update,
        shutdown,
    })
}

fn split_screen_rect_tokens(raw: &str) -> impl Iterator<Item = &str> {
    raw.split(|character: char| {
        character.is_ascii_whitespace() || matches!(character, ',' | ':' | '=')
    })
    .filter(|token| !token.is_empty())
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

fn parse_screen_rect(raw: &str, line: usize) -> Result<WndScreenRect, WndError> {
    let mut tokens = split_screen_rect_tokens(raw);
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
    let window_type = cursor.read_value_until_semicolon(limits)?;
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
    let mut fields = Vec::new();
    let mut children = Vec::new();

    loop {
        let token = cursor.next_token(limits)?;
        if token.text == b"END" {
            break;
        }
        if token.text == b"CHILD" {
            loop {
                let child_token = cursor.next_token(limits)?;
                if child_token.text == b"ENDALLCHILDREN" {
                    break;
                }
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
            }
            continue;
        }
        if token.text == b"SCREENRECT" {
            expect_equals(cursor, limits, token.line)?;
            let raw = cursor.read_value_until_semicolon(limits)?;
            rect = Some(parse_screen_rect(&raw, token.line)?);
            continue;
        }
        let name = decode_token(&token)?;
        expect_equals(cursor, limits, token.line)?;
        let raw_value = cursor.read_value_until_semicolon(limits)?;
        fields.push(WndField {
            name: name.into(),
            raw_value,
            line: token.line,
        });
    }

    let rect = rect.ok_or(WndError::MissingScreenRect { line: window_line })?;

    Ok(WndWindow {
        id,
        window_type,
        rect,
        fields,
        children,
    })
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
    let mut state = ParseState { windows_seen: 0 };

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
        let raw_value = cursor.read_value_until_semicolon(limits)?;
        if !is_known_top_level_field(name) {
            diagnostics.push(WndDiagnostic {
                line: keyword.line,
                window_id: None,
                kind: WndDiagnosticKind::UnknownField { name: name.into() },
            });
        }
        top_level_fields.push(WndField {
            name: name.into(),
            raw_value,
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
    use super::{WndDiagnosticKind, WndError, WndLimits, parse_wnd};

    fn positive_fixture() -> &'static [u8] {
        b"FILE_VERSION = 2\n\
STARTLAYOUTBLOCK\n\
  LAYOUTINIT = SyntheticMenuInit;\n\
  LAYOUTUPDATE = SyntheticMenuUpdate;\n\
  LAYOUTSHUTDOWN = SyntheticMenuShutdown;\n\
ENDLAYOUTBLOCK\n\
WINDOW\n\
  WINDOWTYPE = PUSHBUTTON;\n\
  SCREENRECT = UPPERLEFT: 10 20 BOTTOMRIGHT: 210 70\n\
               CREATIONRESOLUTION: 800 600;\n\
  STATUS = ACTIVE ENABLED;\n\
  CHILD\n\
    WINDOW\n\
      WINDOWTYPE = STATICTEXT;\n\
      SCREENRECT = UPPERLEFT: 20 30 BOTTOMRIGHT: 200 50\n\
                   CREATIONRESOLUTION: 800 600;\n\
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
        assert_eq!(root.fields().len(), 1);
        assert_eq!(root.fields()[0].name(), "STATUS");
        assert_eq!(root.fields()[0].raw_value(), "ACTIVE ENABLED");
        assert_eq!(root.children().len(), 1);

        let child = &root.children()[0];
        assert_eq!(child.id(), 1);
        assert_eq!(child.window_type(), "STATICTEXT");
        assert_eq!(child.rect().upper_left(), (20, 30));
        assert!(document.diagnostics().is_empty());
    }

    #[test]
    fn version_one_defaults_every_layout_callback_to_the_source_none_literal() {
        let bytes = b"FILE_VERSION = 1\nWINDOW\n  WINDOWTYPE = PUSHBUTTON;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\nEND\n";
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
        let bytes = b"FILE_VERSION = 2\nSTARTLAYOUTBLOCK\nENDLAYOUTBLOCK\n";
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
                b"FILE_VERSION = 1\n",
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
                b"FILE_VERSION = 1\nWINDOW\n  WINDOWTYPE = PUSHBUTTON;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n  DATA = one two three four five six seven eight nine ten eleven twelve;\nEND\n",
                WndLimits {
                    maximum_record_bytes: 8,
                    ..default
                },
            ),
            (
                b"FILE_VERSION = 1\nWINDOW\n  WINDOWTYPE = ASuperLongWindowTypeNameThatExceedsTheField;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\nEND\n",
                WndLimits {
                    maximum_field_bytes: 4,
                    ..default
                },
            ),
            (
                b"FILE_VERSION = 1\nWINDOW\n  WINDOWTYPE = PUSHBUTTON;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\nEND\n",
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

        let nested = b"FILE_VERSION = 1\nWINDOW\n  WINDOWTYPE = PUSHBUTTON;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n  CHILD\n    WINDOW\n      WINDOWTYPE = STATICTEXT;\n      SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n    END\n  ENDALLCHILDREN\nEND\n";
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
        let bytes = b"FILE_VERSION = 1\nSOMEUNKNOWNTOPLEVEL = 1;\nWINDOW\n  WINDOWTYPE = PUSHBUTTON;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n  SOMEUNKNOWNWINDOWFIELD = value;\nEND\n";
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
        let bytes = b"FILE_VERSION = 1\nENABLEDCOLOR = 255 255 255 255;\nFONT = Arial 10 0;\nWINDOW\n  WINDOWTYPE = PUSHBUTTON;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\nEND\n";
        let document = parse_wnd(bytes, WndLimits::default()).expect("valid WND");
        assert_eq!(document.top_level_fields().len(), 2);
        assert!(document.diagnostics().is_empty());
    }
}
