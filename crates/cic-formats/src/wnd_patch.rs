// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Bounded decoder and apply engine for project-owned WND patch overlays.
//!
//! Unlike every other decoder in this crate, this format is **not** derived from
//! `GeneralsGameCode`: it is original project design recorded in ADR 0010 and
//! `docs/formats/wnd.md`. It exists so modern controls and profile-specific adjustments can
//! be expressed as auditable data instead of being hardcoded in the parser, the renderer,
//! or a menu callback that searches for special window names.
//!
//! The pipeline is
//!
//! ```text
//! source WND bytes -> immutable WndDocument -> ordered patches -> patched WndDocument
//! ```
//!
//! The source document is never mutated: [`apply_wnd_patches`] clones it and returns a new
//! value, so the same parsed document can be patched differently per profile. Every applied
//! operation records provenance, so any field in the result can be traced to the patch and
//! line that produced it.
//!
//! Version 1 has no wildcards, no selectors, no destructive deletion, and no imperative
//! code. Operations name one exact decorated control name each.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

use crate::wnd::{
    WndDiagnostic, WndDocument, WndError, WndField, WndLimits, WndScreenRect, WndWindow,
};

/// Explicit input and allocation bounds for [`parse_wnd_patch`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WndPatchLimits {
    /// Maximum complete patch length.
    pub maximum_file_bytes: usize,
    /// Maximum physical lines.
    pub maximum_lines: usize,
    /// Maximum operations in one patch.
    pub maximum_operations: usize,
    /// Maximum bytes in one path, control name, field name, or value.
    pub maximum_argument_bytes: usize,
}

impl Default for WndPatchLimits {
    fn default() -> Self {
        Self {
            maximum_file_bytes: 1024 * 1024,
            maximum_lines: 16_384,
            maximum_operations: 4_096,
            maximum_argument_bytes: 4_096,
        }
    }
}

/// The only patch format version this decoder accepts.
pub const WND_PATCH_VERSION: u32 = 1;

/// One declarative patch operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WndPatchOperation {
    /// `require-window <control>` — the target must exist.
    RequireWindow {
        /// Exact decorated control name.
        control: Box<str>,
    },
    /// `require-field <control> <field> <value>` — the field must exist with this exact
    /// verbatim value.
    RequireField {
        /// Exact decorated control name.
        control: Box<str>,
        /// Field name.
        field: Box<str>,
        /// Expected verbatim value.
        value: Box<str>,
    },
    /// `set-field <control> <field> <value>` — replace an existing field's value.
    SetField {
        /// Exact decorated control name.
        control: Box<str>,
        /// Field name, which must already be present.
        field: Box<str>,
        /// Replacement value, tokenized exactly as an authored record would be.
        value: Box<str>,
    },
    /// `add-field <control> <field> <value>` — add a field that must not already exist.
    AddField {
        /// Exact decorated control name.
        control: Box<str>,
        /// Field name, which must be absent.
        field: Box<str>,
        /// Value, tokenized exactly as an authored record would be.
        value: Box<str>,
    },
    /// `set-rect <control> <x0> <y0> <x1> <y1> <width> <height>` — replace the stored
    /// creation rectangle and resolution.
    SetRect {
        /// Exact decorated control name.
        control: Box<str>,
        /// Replacement rectangle.
        rect: WndScreenRect,
    },
    /// `reorder <control> <index>` — move a window within its current parent's child list.
    Reorder {
        /// Exact decorated control name.
        control: Box<str>,
        /// Zero-based destination index among its siblings.
        index: usize,
    },
    /// `reparent <control> <parent> <index>` — move a window beneath a different parent.
    Reparent {
        /// Exact decorated control name of the window to move.
        control: Box<str>,
        /// Exact decorated control name of the destination parent.
        parent: Box<str>,
        /// Zero-based destination index among the new parent's children.
        index: usize,
    },
    /// `insert-window <parent> <index>` … `end-window` — insert a project-owned subtree.
    InsertWindow {
        /// Exact decorated control name of the destination parent.
        parent: Box<str>,
        /// Zero-based destination index among the parent's children.
        index: usize,
        /// The fragment's WND source text, parsed with the ordinary bounded decoder.
        fragment: Box<str>,
    },
}

/// One patch operation with the source line that declared it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WndPatchStep {
    operation: WndPatchOperation,
    line: usize,
}

impl WndPatchStep {
    /// Returns the operation.
    #[must_use]
    pub const fn operation(&self) -> &WndPatchOperation {
        &self.operation
    }

    /// Returns the one-based line the operation was declared on.
    #[must_use]
    pub const fn line(&self) -> usize {
        self.line
    }
}

/// One complete, immutable patch document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WndPatch {
    name: Box<str>,
    version: u32,
    target: Box<str>,
    steps: Vec<WndPatchStep>,
}

impl WndPatch {
    /// Returns the caller-supplied name identifying this patch in errors and provenance.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the declared format version.
    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }

    /// Returns the normalized WND virtual path this patch targets.
    #[must_use]
    pub fn target(&self) -> &str {
        &self.target
    }

    /// Returns the operations in file order.
    #[must_use]
    pub fn steps(&self) -> &[WndPatchStep] {
        &self.steps
    }
}

/// Where one field in a patched document came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WndPatchProvenance {
    control: Box<str>,
    field: Box<str>,
    patch: Box<str>,
    line: usize,
}

impl WndPatchProvenance {
    /// Returns the decorated control name the operation targeted.
    #[must_use]
    pub fn control(&self) -> &str {
        &self.control
    }

    /// Returns the field the operation wrote. `SCREENRECT` for a rectangle replacement.
    #[must_use]
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Returns the name of the patch that wrote it.
    #[must_use]
    pub fn patch(&self) -> &str {
        &self.patch
    }

    /// Returns the patch line that wrote it.
    #[must_use]
    pub const fn line(&self) -> usize {
        self.line
    }
}

/// A patched document plus the provenance of every field a patch wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchedWndDocument {
    document: WndDocument,
    provenance: Vec<WndPatchProvenance>,
}

impl PatchedWndDocument {
    /// Returns the patched document. The source document is unchanged.
    #[must_use]
    pub const fn document(&self) -> &WndDocument {
        &self.document
    }

    /// Returns one record per field a patch wrote, in application order.
    #[must_use]
    pub fn provenance(&self) -> &[WndPatchProvenance] {
        &self.provenance
    }
}

/// A structured patch decoding or application failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WndPatchError {
    /// The patch exceeds [`WndPatchLimits::maximum_file_bytes`].
    FileTooLarge { size: usize, limit: usize },
    /// The patch exceeds [`WndPatchLimits::maximum_lines`].
    TooManyLines { limit: usize },
    /// The patch exceeds [`WndPatchLimits::maximum_operations`].
    TooManyOperations { limit: usize },
    /// One argument exceeds [`WndPatchLimits::maximum_argument_bytes`].
    ArgumentTooLong {
        line: usize,
        size: usize,
        limit: usize,
    },
    /// The patch is not valid UTF-8.
    InvalidUtf8,
    /// A quoted argument was never closed.
    UnterminatedString { line: usize },
    /// The patch did not declare `version` before its first operation.
    MissingVersion,
    /// The patch did not declare `target` before its first operation.
    MissingTarget,
    /// The declared version is not [`WND_PATCH_VERSION`].
    UnsupportedVersion { line: usize, version: Box<str> },
    /// A directive or operation keyword was not recognized.
    UnknownDirective { line: usize, keyword: Box<str> },
    /// An operation had the wrong number of arguments.
    WrongArgumentCount {
        line: usize,
        keyword: Box<str>,
        expected: usize,
        found: usize,
    },
    /// A numeric argument was not a valid integer.
    InvalidInteger { line: usize, value: Box<str> },
    /// A replacement value could not be tokenized as a WND record.
    InvalidValue { line: usize, source: WndError },
    /// A patch targeted a different WND path than the document it was applied to.
    TargetMismatch {
        patch: Box<str>,
        expected: Box<str>,
        found: Box<str>,
    },
    /// A required control name is not present in the document.
    MissingControl {
        patch: Box<str>,
        line: usize,
        control: Box<str>,
    },
    /// A `require-field` precondition did not hold.
    PreconditionFailed {
        patch: Box<str>,
        line: usize,
        control: Box<str>,
        field: Box<str>,
        expected: Box<str>,
        found: Option<Box<str>>,
    },
    /// `set-field` named a field the control does not declare.
    MissingField {
        patch: Box<str>,
        line: usize,
        control: Box<str>,
        field: Box<str>,
    },
    /// `add-field` named a field the control already declares.
    DuplicateField {
        patch: Box<str>,
        line: usize,
        control: Box<str>,
        field: Box<str>,
    },
    /// A destination index is beyond the end of the child list it addresses.
    IndexOutOfRange {
        patch: Box<str>,
        line: usize,
        index: usize,
        length: usize,
    },
    /// A `reparent` would place a window beneath itself or one of its own descendants.
    ReparentCycle {
        patch: Box<str>,
        line: usize,
        control: Box<str>,
        parent: Box<str>,
    },
    /// An inserted subtree declares a decorated name the document already uses.
    DuplicateInsertedName {
        patch: Box<str>,
        line: usize,
        name: Box<str>,
    },
    /// An `insert-window` fragment is not a single valid `WINDOW` block.
    InvalidFragment { line: usize, source: WndError },
    /// An `insert-window` block was never closed with `end-window`.
    UnterminatedFragment { line: usize },
}

impl Display for WndPatchError {
    #[allow(clippy::too_many_lines)]
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::FileTooLarge { size, limit } => {
                write!(formatter, "WND patch is {size} bytes; limit is {limit}")
            }
            Self::TooManyLines { limit } => {
                write!(formatter, "WND patch exceeds {limit} lines")
            }
            Self::TooManyOperations { limit } => {
                write!(formatter, "WND patch exceeds {limit} operations")
            }
            Self::ArgumentTooLong { line, size, limit } => write!(
                formatter,
                "WND patch argument at line {line} is {size} bytes; limit is {limit}"
            ),
            Self::InvalidUtf8 => formatter.write_str("WND patch is not valid UTF-8"),
            Self::UnterminatedString { line } => write!(
                formatter,
                "WND patch quoted argument at line {line} was never closed"
            ),
            Self::MissingVersion => {
                formatter.write_str("WND patch does not declare a version directive")
            }
            Self::MissingTarget => {
                formatter.write_str("WND patch does not declare a target directive")
            }
            Self::UnsupportedVersion { line, version } => write!(
                formatter,
                "WND patch version '{version}' at line {line} is not supported; expected {WND_PATCH_VERSION}"
            ),
            Self::UnknownDirective { line, keyword } => write!(
                formatter,
                "WND patch line {line} has unknown directive '{keyword}'"
            ),
            Self::WrongArgumentCount {
                line,
                keyword,
                expected,
                found,
            } => write!(
                formatter,
                "WND patch '{keyword}' at line {line} takes {expected} arguments; found {found}"
            ),
            Self::InvalidInteger { line, value } => write!(
                formatter,
                "WND patch line {line} has non-integer value '{value}'"
            ),
            Self::InvalidValue { line, source } => write!(
                formatter,
                "WND patch value at line {line} is not a valid record: {source}"
            ),
            Self::TargetMismatch {
                patch,
                expected,
                found,
            } => write!(
                formatter,
                "WND patch '{patch}' targets '{expected}' but was applied to '{found}'"
            ),
            Self::MissingControl {
                patch,
                line,
                control,
            } => write!(
                formatter,
                "WND patch '{patch}' line {line} requires control '{control}', which the document does not declare"
            ),
            Self::PreconditionFailed {
                patch,
                line,
                control,
                field,
                expected,
                found,
            } => {
                let found = found.as_deref().unwrap_or("(absent)");
                write!(
                    formatter,
                    "WND patch '{patch}' line {line} expected {control}.{field} to be '{expected}' but found '{found}'"
                )
            }
            Self::MissingField {
                patch,
                line,
                control,
                field,
            } => write!(
                formatter,
                "WND patch '{patch}' line {line} sets {control}.{field}, which the control does not declare"
            ),
            Self::DuplicateField {
                patch,
                line,
                control,
                field,
            } => write!(
                formatter,
                "WND patch '{patch}' line {line} adds {control}.{field}, which the control already declares"
            ),
            Self::IndexOutOfRange {
                patch,
                line,
                index,
                length,
            } => write!(
                formatter,
                "WND patch '{patch}' line {line} names index {index}; the child list holds {length}"
            ),
            Self::ReparentCycle {
                patch,
                line,
                control,
                parent,
            } => write!(
                formatter,
                "WND patch '{patch}' line {line} would reparent '{control}' beneath '{parent}', which is inside its own subtree"
            ),
            Self::DuplicateInsertedName { patch, line, name } => write!(
                formatter,
                "WND patch '{patch}' line {line} inserts '{name}', which the document already declares"
            ),
            Self::InvalidFragment { line, source } => write!(
                formatter,
                "WND patch fragment at line {line} is not a valid WINDOW block: {source}"
            ),
            Self::UnterminatedFragment { line } => write!(
                formatter,
                "WND patch insert-window at line {line} was never closed with end-window"
            ),
        }
    }
}

impl Error for WndPatchError {}

/// Splits one patch line into whitespace-separated arguments, honoring double quotes.
///
/// Inside a quoted argument, `\"` and `\\` are escapes. This is a deliberate divergence
/// from the WND grammar, which has no escape syntax: a patch has to be able to *express* a
/// WND value that itself contains quotes, such as `FONT = NAME: "Arial", SIZE: 10, BOLD: 0`,
/// and without escapes that value would be unwritable. A backslash before any other
/// character is literal. A `#` starting a bare token begins a comment that runs to end of
/// line; inside quotes it is ordinary text.
fn split_arguments(
    line: &str,
    line_number: usize,
    limits: WndPatchLimits,
) -> Result<Vec<Box<str>>, WndPatchError> {
    let mut arguments = Vec::new();
    let mut characters = line.chars().peekable();
    loop {
        while characters.peek().is_some_and(|c| c.is_whitespace()) {
            characters.next();
        }
        let Some(&first) = characters.peek() else {
            break;
        };
        if first == '#' {
            break;
        }
        let mut value = String::new();
        if first == '"' {
            characters.next();
            let mut closed = false;
            while let Some(character) = characters.next() {
                match character {
                    '"' => {
                        closed = true;
                        break;
                    }
                    '\\' if characters
                        .peek()
                        .is_some_and(|next| *next == '"' || *next == '\\') =>
                    {
                        value.push(characters.next().unwrap_or('\\'));
                    }
                    other => value.push(other),
                }
            }
            if !closed {
                return Err(WndPatchError::UnterminatedString { line: line_number });
            }
        } else {
            while let Some(&character) = characters.peek() {
                if character.is_whitespace() {
                    break;
                }
                value.push(character);
                characters.next();
            }
        }
        if value.len() > limits.maximum_argument_bytes {
            return Err(WndPatchError::ArgumentTooLong {
                line: line_number,
                size: value.len(),
                limit: limits.maximum_argument_bytes,
            });
        }
        arguments.push(value.into_boxed_str());
    }
    Ok(arguments)
}

fn expect_arguments(
    arguments: &[Box<str>],
    keyword: &str,
    expected: usize,
    line: usize,
) -> Result<(), WndPatchError> {
    if arguments.len() == expected {
        Ok(())
    } else {
        Err(WndPatchError::WrongArgumentCount {
            line,
            keyword: keyword.into(),
            expected,
            found: arguments.len(),
        })
    }
}

fn parse_integer(value: &str, line: usize) -> Result<i32, WndPatchError> {
    value
        .parse::<i32>()
        .map_err(|_| WndPatchError::InvalidInteger {
            line,
            value: value.into(),
        })
}

/// Decodes one operation line into its typed operation.
fn parse_operation(
    keyword: &str,
    rest: &[Box<str>],
    line: usize,
) -> Result<WndPatchOperation, WndPatchError> {
    Ok(match keyword {
        "require-window" => {
            expect_arguments(rest, "require-window", 1, line)?;
            WndPatchOperation::RequireWindow {
                control: rest[0].clone(),
            }
        }
        "require-field" => {
            expect_arguments(rest, "require-field", 3, line)?;
            WndPatchOperation::RequireField {
                control: rest[0].clone(),
                field: rest[1].clone(),
                value: rest[2].clone(),
            }
        }
        "set-field" => {
            expect_arguments(rest, "set-field", 3, line)?;
            WndPatchOperation::SetField {
                control: rest[0].clone(),
                field: rest[1].clone(),
                value: rest[2].clone(),
            }
        }
        "add-field" => {
            expect_arguments(rest, "add-field", 3, line)?;
            WndPatchOperation::AddField {
                control: rest[0].clone(),
                field: rest[1].clone(),
                value: rest[2].clone(),
            }
        }
        "set-rect" => {
            expect_arguments(rest, "set-rect", 7, line)?;
            let numbers = rest[1..]
                .iter()
                .map(|value| parse_integer(value, line))
                .collect::<Result<Vec<_>, _>>()?;
            WndPatchOperation::SetRect {
                control: rest[0].clone(),
                rect: WndScreenRect::new(
                    (numbers[0], numbers[1]),
                    (numbers[2], numbers[3]),
                    (numbers[4], numbers[5]),
                ),
            }
        }
        "reorder" => {
            expect_arguments(rest, "reorder", 2, line)?;
            WndPatchOperation::Reorder {
                control: rest[0].clone(),
                index: parse_index(&rest[1], line)?,
            }
        }
        "reparent" => {
            expect_arguments(rest, "reparent", 3, line)?;
            WndPatchOperation::Reparent {
                control: rest[0].clone(),
                parent: rest[1].clone(),
                index: parse_index(&rest[2], line)?,
            }
        }
        other => {
            return Err(WndPatchError::UnknownDirective {
                line,
                keyword: other.into(),
            });
        }
    })
}

fn parse_index(value: &str, line: usize) -> Result<usize, WndPatchError> {
    value
        .parse::<usize>()
        .map_err(|_| WndPatchError::InvalidInteger {
            line,
            value: value.into(),
        })
}

/// An `insert-window` body being collected line by line.
struct OpenFragment {
    parent: Box<str>,
    index: usize,
    start: usize,
    body: String,
}

/// Consumes one line of an open `insert-window` body.
///
/// Returns the finished step when the body closes with `end-window`, and `None` while it is
/// still open.
fn continue_fragment(
    open: &mut OpenFragment,
    line: &str,
    limits: WndPatchLimits,
) -> Result<Option<WndPatchStep>, WndPatchError> {
    if line.trim() == "end-window" {
        return Ok(Some(WndPatchStep {
            operation: WndPatchOperation::InsertWindow {
                parent: open.parent.clone(),
                index: open.index,
                fragment: std::mem::take(&mut open.body).into_boxed_str(),
            },
            line: open.start,
        }));
    }
    open.body.push_str(line);
    open.body.push('\n');
    if open.body.len() > limits.maximum_file_bytes {
        return Err(WndPatchError::FileTooLarge {
            size: open.body.len(),
            limit: limits.maximum_file_bytes,
        });
    }
    Ok(None)
}

/// Parses a patch document.
///
/// `name` identifies the patch in errors and provenance; callers normally pass its virtual
/// path.
///
/// # Errors
///
/// Returns [`WndPatchError`] for a malformed line, an unknown directive, an unsupported
/// version, a missing required directive, or any [`WndPatchLimits`] excess.
pub fn parse_wnd_patch(
    name: &str,
    bytes: &[u8],
    limits: WndPatchLimits,
) -> Result<WndPatch, WndPatchError> {
    if bytes.len() > limits.maximum_file_bytes {
        return Err(WndPatchError::FileTooLarge {
            size: bytes.len(),
            limit: limits.maximum_file_bytes,
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| WndPatchError::InvalidUtf8)?;

    let mut version = None;
    let mut target = None;
    let mut steps: Vec<WndPatchStep> = Vec::new();
    // Set while collecting an `insert-window` body; every line until `end-window` is
    // verbatim WND source rather than a patch directive.
    let mut fragment: Option<OpenFragment> = None;

    for (index, line) in text.lines().enumerate() {
        let line_number = index + 1;
        if line_number > limits.maximum_lines {
            return Err(WndPatchError::TooManyLines {
                limit: limits.maximum_lines,
            });
        }

        if let Some(open) = fragment.as_mut() {
            if let Some(step) = continue_fragment(open, line, limits)? {
                fragment = None;
                if steps.len() >= limits.maximum_operations {
                    return Err(WndPatchError::TooManyOperations {
                        limit: limits.maximum_operations,
                    });
                }
                steps.push(step);
            }
            continue;
        }

        let arguments = split_arguments(line, line_number, limits)?;
        let Some(keyword) = arguments.first() else {
            continue;
        };
        let rest = &arguments[1..];

        match &**keyword {
            "version" => {
                expect_arguments(rest, "version", 1, line_number)?;
                if rest[0].parse::<u32>() != Ok(WND_PATCH_VERSION) {
                    return Err(WndPatchError::UnsupportedVersion {
                        line: line_number,
                        version: rest[0].clone(),
                    });
                }
                version = Some(WND_PATCH_VERSION);
                continue;
            }
            "target" => {
                expect_arguments(rest, "target", 1, line_number)?;
                target = Some(rest[0].clone());
                continue;
            }
            _ => {}
        }

        if version.is_none() {
            return Err(WndPatchError::MissingVersion);
        }
        if target.is_none() {
            return Err(WndPatchError::MissingTarget);
        }
        if steps.len() >= limits.maximum_operations {
            return Err(WndPatchError::TooManyOperations {
                limit: limits.maximum_operations,
            });
        }

        if &**keyword == "insert-window" {
            expect_arguments(rest, "insert-window", 2, line_number)?;
            let index = parse_index(&rest[1], line_number)?;
            fragment = Some(OpenFragment {
                parent: rest[0].clone(),
                index,
                start: line_number,
                body: String::new(),
            });
            continue;
        }

        steps.push(WndPatchStep {
            operation: parse_operation(keyword, rest, line_number)?,
            line: line_number,
        });
    }

    if let Some(open) = fragment {
        return Err(WndPatchError::UnterminatedFragment { line: open.start });
    }

    Ok(WndPatch {
        name: name.into(),
        version: version.ok_or(WndPatchError::MissingVersion)?,
        target: target.ok_or(WndPatchError::MissingTarget)?,
        steps,
    })
}

fn normalize_target(path: &str) -> String {
    path.replace('\\', "/").to_ascii_lowercase()
}

/// Applies ordered patches to a parsed document, returning a new patched value.
///
/// `document_path` is the virtual path the document was read from; every patch's `target`
/// must match it, compared case-insensitively with `\` normalized to `/`. Patches apply in
/// slice order and operations within a patch apply in file order, so a later patch observes
/// an earlier one's result.
///
/// The source `document` is never modified.
///
/// # Errors
///
/// Returns [`WndPatchError`] for a target mismatch, a missing required control, a failed
/// precondition, a `set-field` on an absent field, an `add-field` on a present one, or a
/// replacement value that is not a valid record.
pub fn apply_wnd_patches(
    document: &WndDocument,
    document_path: &str,
    patches: &[WndPatch],
    limits: WndLimits,
) -> Result<PatchedWndDocument, WndPatchError> {
    let mut result = document.clone();
    let mut provenance = Vec::new();
    let mut diagnostics = Vec::new();
    let normalized_document = normalize_target(document_path);

    for patch in patches {
        if normalize_target(patch.target()) != normalized_document {
            return Err(WndPatchError::TargetMismatch {
                patch: patch.name.clone(),
                expected: patch.target.clone(),
                found: document_path.into(),
            });
        }
        for step in patch.steps() {
            apply_step(
                &mut result,
                patch,
                step,
                limits,
                &mut provenance,
                &mut diagnostics,
            )?;
        }
    }

    result.push_diagnostics(diagnostics);
    Ok(PatchedWndDocument {
        document: result,
        provenance,
    })
}

fn find_control<'a>(document: &'a mut WndDocument, control: &str) -> Option<&'a mut WndWindow> {
    document
        .windows_mut()
        .iter_mut()
        .find_map(|window| window.find_by_decorated_name_mut(control))
}

fn matches_control(window: &WndWindow, control: &str) -> bool {
    window.control_name().is_some() && window.name() == Some(control)
}

/// Returns the child-index path from the forest root to the named window.
///
/// Structural operations resolve a path first and navigate mutably afterwards, rather than
/// searching mutably, so one traversal never holds a borrow across the mutation.
fn find_path(forest: &[WndWindow], control: &str) -> Option<Vec<usize>> {
    for (index, window) in forest.iter().enumerate() {
        if matches_control(window, control) {
            return Some(vec![index]);
        }
        if let Some(sub) = find_path(window.children(), control) {
            let mut path = Vec::with_capacity(sub.len() + 1);
            path.push(index);
            path.extend(sub);
            return Some(path);
        }
    }
    None
}

/// Returns the child list that directly holds the window at `path`.
fn sibling_list<'a>(document: &'a mut WndDocument, path: &[usize]) -> &'a mut Vec<WndWindow> {
    let mut list = document.windows_mut();
    for &index in &path[..path.len().saturating_sub(1)] {
        list = list[index].children_mut();
    }
    list
}

/// Returns the child list beneath the named parent.
fn children_of<'a>(document: &'a mut WndDocument, parent_path: &[usize]) -> &'a mut Vec<WndWindow> {
    let mut list = document.windows_mut();
    for &index in parent_path {
        list = list[index].children_mut();
    }
    list
}

/// What every operation handler needs to report a failure against its own source line.
struct StepContext<'a> {
    patch: &'a WndPatch,
    line: usize,
    limits: WndLimits,
}

impl StepContext<'_> {
    fn check_index(&self, index: usize, length: usize) -> Result<(), WndPatchError> {
        if index <= length {
            Ok(())
        } else {
            Err(WndPatchError::IndexOutOfRange {
                patch: self.patch.name.clone(),
                line: self.line,
                index,
                length,
            })
        }
    }

    fn missing_control(&self, control: &str) -> WndPatchError {
        WndPatchError::MissingControl {
            patch: self.patch.name.clone(),
            line: self.line,
            control: control.into(),
        }
    }

    fn provenance(&self, control: &str, field: &str) -> WndPatchProvenance {
        WndPatchProvenance {
            control: control.into(),
            field: field.into(),
            patch: self.patch.name.clone(),
            line: self.line,
        }
    }
}

/// Moves a window within its current sibling list.
fn apply_reorder(
    document: &mut WndDocument,
    context: &StepContext<'_>,
    control: &str,
    index: usize,
    provenance: &mut Vec<WndPatchProvenance>,
) -> Result<(), WndPatchError> {
    let from_path =
        find_path(document.windows(), control).ok_or_else(|| context.missing_control(control))?;
    let current = *from_path.last().expect("a path always names a child");
    let list = sibling_list(document, &from_path);
    // The window stays in this list, so the last valid destination is len - 1.
    context.check_index(index, list.len().saturating_sub(1))?;
    let window = list.remove(current);
    list.insert(index, window);
    provenance.push(context.provenance(control, "(order)"));
    Ok(())
}

/// Moves a window beneath a different parent.
///
/// The window is detached before its destination is resolved, because removing it can shift
/// the parent's path. Every failure after that removal therefore puts it back rather than
/// dropping the subtree.
fn apply_reparent(
    document: &mut WndDocument,
    context: &StepContext<'_>,
    control: &str,
    parent: &str,
    index: usize,
    provenance: &mut Vec<WndPatchProvenance>,
) -> Result<(), WndPatchError> {
    let from_path =
        find_path(document.windows(), control).ok_or_else(|| context.missing_control(control))?;
    let current = *from_path.last().expect("a path always names a child");

    // Refuse before detaching: a window may not land inside its own subtree.
    if sibling_list(document, &from_path)[current].subtree_contains(parent) {
        return Err(WndPatchError::ReparentCycle {
            patch: context.patch.name.clone(),
            line: context.line,
            control: control.into(),
            parent: parent.into(),
        });
    }

    let detached = sibling_list(document, &from_path).remove(current);
    let Some(parent_path) = find_path(document.windows(), parent) else {
        sibling_list(document, &from_path).insert(current, detached);
        return Err(context.missing_control(parent));
    };
    let length = children_of(document, &parent_path).len();
    if let Err(error) = context.check_index(index, length) {
        sibling_list(document, &from_path).insert(current, detached);
        return Err(error);
    }

    children_of(document, &parent_path).insert(index, detached);
    provenance.push(context.provenance(control, "(parent)"));
    Ok(())
}

/// Inserts a project-owned subtree beneath an exact parent.
fn apply_insert_window(
    document: &mut WndDocument,
    context: &StepContext<'_>,
    parent: &str,
    index: usize,
    fragment: &str,
    provenance: &mut Vec<WndPatchProvenance>,
) -> Result<(), WndPatchError> {
    let mut inserted = parse_fragment(fragment, context.line, context.limits)?;

    let mut existing = Vec::new();
    for window in document.windows() {
        window.collect_decorated_names(&mut existing);
    }
    let mut added = Vec::new();
    inserted.collect_decorated_names(&mut added);
    if let Some(clash) = added.iter().find(|name| existing.contains(name)) {
        return Err(WndPatchError::DuplicateInsertedName {
            patch: context.patch.name.clone(),
            line: context.line,
            name: clash.clone(),
        });
    }

    let parent_path =
        find_path(document.windows(), parent).ok_or_else(|| context.missing_control(parent))?;
    let length = children_of(document, &parent_path).len();
    context.check_index(index, length)?;

    // Ids are source-order and must stay unique, so continue past the highest one the
    // document already holds.
    let mut next = document
        .windows()
        .iter()
        .map(WndWindow::maximum_id)
        .max()
        .map_or(0, |highest| highest + 1);
    inserted.renumber_from(&mut next);
    for name in added {
        provenance.push(context.provenance(&name, "(inserted)"));
    }
    children_of(document, &parent_path).insert(index, inserted);
    Ok(())
}

/// Routes the structural operations to their handlers.
fn apply_structural_step(
    document: &mut WndDocument,
    context: &StepContext<'_>,
    operation: &WndPatchOperation,
    provenance: &mut Vec<WndPatchProvenance>,
) -> Result<(), WndPatchError> {
    match operation {
        WndPatchOperation::Reorder { control, index } => {
            apply_reorder(document, context, control, *index, provenance)
        }
        WndPatchOperation::Reparent {
            control,
            parent,
            index,
        } => apply_reparent(document, context, control, parent, *index, provenance),
        WndPatchOperation::InsertWindow {
            parent,
            index,
            fragment,
        } => apply_insert_window(document, context, parent, *index, fragment, provenance),
        _ => unreachable!("apply_step routes only structural operations here"),
    }
}

/// Parses an `insert-window` body as one `WINDOW` block.
///
/// The fragment is wrapped in a minimal version-1 document and handed to the ordinary
/// bounded decoder, so an inserted subtree is held to exactly the same grammar, limits, and
/// typed-field rules as authored source.
fn parse_fragment(
    fragment: &str,
    line: usize,
    limits: WndLimits,
) -> Result<WndWindow, WndPatchError> {
    let document = format!("FILE_VERSION = 1;\n{fragment}");
    let parsed = crate::wnd::parse_wnd(document.as_bytes(), limits)
        .map_err(|source| WndPatchError::InvalidFragment { line, source })?;
    if parsed.windows().len() != 1 {
        return Err(WndPatchError::InvalidFragment {
            line,
            source: WndError::NoWindows,
        });
    }
    Ok(parsed.windows()[0].clone())
}

fn apply_step(
    document: &mut WndDocument,
    patch: &WndPatch,
    step: &WndPatchStep,
    limits: WndLimits,
    provenance: &mut Vec<WndPatchProvenance>,
    diagnostics: &mut Vec<WndDiagnostic>,
) -> Result<(), WndPatchError> {
    let context = StepContext {
        patch,
        line: step.line(),
        limits,
    };
    let control_name = match step.operation() {
        WndPatchOperation::Reorder { .. }
        | WndPatchOperation::Reparent { .. }
        | WndPatchOperation::InsertWindow { .. } => {
            return apply_structural_step(document, &context, step.operation(), provenance);
        }
        WndPatchOperation::RequireWindow { control }
        | WndPatchOperation::RequireField { control, .. }
        | WndPatchOperation::SetField { control, .. }
        | WndPatchOperation::AddField { control, .. }
        | WndPatchOperation::SetRect { control, .. } => control.clone(),
    };
    let window = find_control(document, &control_name)
        .ok_or_else(|| context.missing_control(&control_name))?;

    match step.operation() {
        WndPatchOperation::RequireWindow { .. } => Ok(()),
        WndPatchOperation::RequireField {
            control,
            field,
            value,
        } => check_precondition(window, &context, control, field, value),
        WndPatchOperation::SetField {
            control,
            field,
            value,
        } => apply_set_field(
            window,
            &context,
            control,
            field,
            value,
            provenance,
            diagnostics,
        ),
        WndPatchOperation::AddField {
            control,
            field,
            value,
        } => apply_add_field(
            window,
            &context,
            control,
            field,
            value,
            provenance,
            diagnostics,
        ),
        WndPatchOperation::SetRect { control, rect } => {
            window.set_rect(*rect);
            provenance.push(context.provenance(control, "SCREENRECT"));
            Ok(())
        }
        WndPatchOperation::Reorder { .. }
        | WndPatchOperation::Reparent { .. }
        | WndPatchOperation::InsertWindow { .. } => {
            unreachable!("structural operations return above")
        }
    }
}

/// Verifies a `require-field` precondition against the document as it currently stands.
fn check_precondition(
    window: &mut WndWindow,
    context: &StepContext<'_>,
    control: &str,
    field: &str,
    value: &str,
) -> Result<(), WndPatchError> {
    let found: Option<Box<str>> = window
        .field_mut(field)
        .map(|entry| entry.raw_value().into());
    if found.as_deref() == Some(value) {
        return Ok(());
    }
    Err(WndPatchError::PreconditionFailed {
        patch: context.patch.name.clone(),
        line: context.line,
        control: control.into(),
        field: field.into(),
        expected: value.into(),
        found,
    })
}

/// Replaces an existing field's value, then re-types the window from its new fields.
fn apply_set_field(
    window: &mut WndWindow,
    context: &StepContext<'_>,
    control: &str,
    field: &str,
    value: &str,
    provenance: &mut Vec<WndPatchProvenance>,
    diagnostics: &mut Vec<WndDiagnostic>,
) -> Result<(), WndPatchError> {
    let replacement = WndField::from_patch_value(field, value, context.line, context.limits)
        .map_err(|source| WndPatchError::InvalidValue {
            line: context.line,
            source,
        })?;
    let existing = window
        .field_mut(field)
        .ok_or_else(|| WndPatchError::MissingField {
            patch: context.patch.name.clone(),
            line: context.line,
            control: control.into(),
            field: field.into(),
        })?;
    *existing = replacement;
    window.retype(diagnostics);
    provenance.push(context.provenance(control, field));
    Ok(())
}

/// Adds a field the control does not already declare, then re-types the window.
fn apply_add_field(
    window: &mut WndWindow,
    context: &StepContext<'_>,
    control: &str,
    field: &str,
    value: &str,
    provenance: &mut Vec<WndPatchProvenance>,
    diagnostics: &mut Vec<WndDiagnostic>,
) -> Result<(), WndPatchError> {
    if window.field_mut(field).is_some() {
        return Err(WndPatchError::DuplicateField {
            patch: context.patch.name.clone(),
            line: context.line,
            control: control.into(),
            field: field.into(),
        });
    }
    let added = WndField::from_patch_value(field, value, context.line, context.limits).map_err(
        |source| WndPatchError::InvalidValue {
            line: context.line,
            source,
        },
    )?;
    window.push_field(added);
    window.retype(diagnostics);
    provenance.push(context.provenance(control, field));
    Ok(())
}

#[cfg(test)]
mod tests {
    /// Predicate identifying the expected error variant for one case.
    type Check = fn(&WndPatchError) -> bool;

    use std::fmt::Write as _;

    use super::{
        WND_PATCH_VERSION, WndPatchError, WndPatchLimits, WndPatchOperation, apply_wnd_patches,
        parse_wnd_patch,
    };
    use crate::wnd::{WndLimits, parse_wnd};

    const TARGET: &str = "Menus/Synthetic.wnd";

    fn document() -> crate::wnd::WndDocument {
        let bytes = b"FILE_VERSION = 1;\nWINDOW\n  WINDOWTYPE = USER;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 100 100 CREATIONRESOLUTION: 800 600;\n  NAME = \"Synthetic.wnd:Root\";\n  CHILD\n    WINDOW\n      WINDOWTYPE = COMBOBOX;\n      SCREENRECT = UPPERLEFT: 10 10 BOTTOMRIGHT: 90 30 CREATIONRESOLUTION: 800 600;\n      NAME = \"Synthetic.wnd:ComboBoxResolution\";\n      STATUS = ENABLED;\n    END\n  ENDALLCHILDREN\nEND\n";
        parse_wnd(bytes, WndLimits::default()).expect("valid WND")
    }

    fn patch(body: &str) -> super::WndPatch {
        let text = format!("version {WND_PATCH_VERSION}\ntarget {TARGET}\n{body}");
        parse_wnd_patch(
            "test.cic-wnd-patch",
            text.as_bytes(),
            WndPatchLimits::default(),
        )
        .expect("valid patch")
    }

    #[test]
    fn parses_directives_operations_and_comments() {
        let parsed = patch(
            "# a comment line\nrequire-window \"Synthetic.wnd:ComboBoxResolution\"\nset-field \"Synthetic.wnd:ComboBoxResolution\" STATUS \"ENABLED+HIDDEN\"  # trailing comment\n",
        );
        assert_eq!(parsed.version(), WND_PATCH_VERSION);
        assert_eq!(parsed.target(), TARGET);
        assert_eq!(parsed.steps().len(), 2);
        assert_eq!(
            parsed.steps()[0].operation(),
            &WndPatchOperation::RequireWindow {
                control: "Synthetic.wnd:ComboBoxResolution".into()
            }
        );
        assert_eq!(parsed.steps()[1].line(), 5);
    }

    #[test]
    fn applies_field_edits_and_leaves_the_source_document_unchanged() {
        let source = document();
        let before = source.clone();
        let patched = apply_wnd_patches(
            &source,
            TARGET,
            &[patch(
                "set-field \"Synthetic.wnd:ComboBoxResolution\" STATUS \"ENABLED+HIDDEN\"\nadd-field \"Synthetic.wnd:ComboBoxResolution\" TOOLTIPDELAY \"250\"\nset-rect \"Synthetic.wnd:ComboBoxResolution\" 5 6 7 8 1024 768\n",
            )],
            WndLimits::default(),
        )
        .expect("patch applies");

        assert_eq!(source, before, "the source document must not be mutated");

        let child = &patched.document().windows()[0].children()[0];
        assert_eq!(
            child
                .status()
                .iter()
                .map(crate::wnd::WndFlag::name)
                .collect::<Vec<_>>(),
            vec!["ENABLED", "HIDDEN"],
            "typed views are recomputed from the patched field"
        );
        assert_eq!(child.tooltip_delay(), Some(250));
        assert_eq!(child.rect().upper_left(), (5, 6));
        assert_eq!(child.rect().creation_resolution(), (1024, 768));

        let provenance = patched.provenance();
        assert_eq!(provenance.len(), 3);
        assert_eq!(provenance[0].field(), "STATUS");
        assert_eq!(provenance[0].control(), "Synthetic.wnd:ComboBoxResolution");
        assert_eq!(provenance[0].patch(), "test.cic-wnd-patch");
        assert_eq!(provenance[2].field(), "SCREENRECT");
    }

    #[test]
    fn later_patches_observe_earlier_results_in_order() {
        let source = document();
        let patched = apply_wnd_patches(
            &source,
            TARGET,
            &[
                patch("set-field \"Synthetic.wnd:Root\" NAME \"Synthetic.wnd:First\"\n"),
                patch("set-field \"Synthetic.wnd:First\" NAME \"Synthetic.wnd:Second\"\n"),
            ],
            WndLimits::default(),
        )
        .expect("ordered patches apply");
        assert_eq!(
            patched.document().windows()[0].name(),
            Some("Synthetic.wnd:Second")
        );
    }

    #[test]
    fn every_failure_mode_is_a_structured_error() {
        let source = document();
        let cases: [(&str, Check); 5] = [
            ("require-window \"Synthetic.wnd:Absent\"\n", |error| {
                matches!(error, WndPatchError::MissingControl { .. })
            }),
            (
                "require-field \"Synthetic.wnd:ComboBoxResolution\" STATUS \"DISABLED\"\n",
                |error| matches!(error, WndPatchError::PreconditionFailed { .. }),
            ),
            (
                "set-field \"Synthetic.wnd:ComboBoxResolution\" FONT \"NAME: \\\"Arial\\\"\"\n",
                |error| matches!(error, WndPatchError::MissingField { .. }),
            ),
            (
                "add-field \"Synthetic.wnd:ComboBoxResolution\" STATUS \"ENABLED\"\n",
                |error| matches!(error, WndPatchError::DuplicateField { .. }),
            ),
            (
                "set-rect \"Synthetic.wnd:ComboBoxResolution\" 1 2 3 4 5 notanumber\n",
                |error| matches!(error, WndPatchError::InvalidInteger { .. }),
            ),
        ];
        for (body, matches) in cases {
            let text = format!("version {WND_PATCH_VERSION}\ntarget {TARGET}\n{body}");
            let parsed = parse_wnd_patch("t", text.as_bytes(), WndPatchLimits::default());
            let error = match parsed {
                Ok(parsed) => apply_wnd_patches(&source, TARGET, &[parsed], WndLimits::default())
                    .expect_err("must fail"),
                Err(error) => error,
            };
            assert!(matches(&error), "unexpected error for {body:?}: {error}");
        }
    }

    #[test]
    fn can_express_a_wnd_value_that_itself_contains_quotes() {
        // FONT's name is quoted in the source grammar, so without escapes a patch could
        // never write a well-formed FONT record.
        let source = document();
        let patched = apply_wnd_patches(
            &source,
            TARGET,
            &[patch(
                "add-field \"Synthetic.wnd:ComboBoxResolution\" FONT \"NAME: \\\"Times New Roman\\\", SIZE: 14, BOLD: 0\"\n",
            )],
            WndLimits::default(),
        )
        .expect("patch applies");

        let child = &patched.document().windows()[0].children()[0];
        let font = child.font().expect("the patched FONT record types");
        assert_eq!(font.name(), "Times New Roman");
        assert_eq!(font.size(), 14);
        assert!(!font.bold());
        assert_eq!(
            child
                .fields()
                .iter()
                .find(|field| field.name() == "FONT")
                .expect("retained generically too")
                .raw_value(),
            "NAME: \"Times New Roman\", SIZE: 14, BOLD: 0"
        );
    }

    /// A root with three named children, for structural operations.
    fn forest() -> crate::wnd::WndDocument {
        let mut bytes = String::from(
            "FILE_VERSION = 1;\nWINDOW\n  WINDOWTYPE = USER;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 100 100 CREATIONRESOLUTION: 800 600;\n  NAME = \"Synthetic.wnd:Root\";\n",
        );
        for child in ["Alpha", "Beta", "Gamma"] {
            write!(
                bytes,
                "  CHILD\n    WINDOW\n      WINDOWTYPE = PUSHBUTTON;\n      SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n      NAME = \"Synthetic.wnd:{child}\";\n    END\n"
            )
            .expect("writing to a String cannot fail");
        }
        bytes.push_str("  ENDALLCHILDREN\nEND\n");
        parse_wnd(bytes.as_bytes(), WndLimits::default()).expect("valid WND")
    }

    fn child_names(document: &crate::wnd::WndDocument) -> Vec<&str> {
        document.windows()[0]
            .children()
            .iter()
            .map(|child| child.control_name().unwrap_or("-"))
            .collect()
    }

    #[test]
    fn reorders_a_window_within_its_sibling_list() {
        let source = forest();
        let patched = apply_wnd_patches(
            &source,
            TARGET,
            &[patch("reorder \"Synthetic.wnd:Gamma\" 0\n")],
            WndLimits::default(),
        )
        .expect("reorder applies");
        assert_eq!(child_names(patched.document()), ["Gamma", "Alpha", "Beta"]);
        assert_eq!(child_names(&source), ["Alpha", "Beta", "Gamma"]);
    }

    #[test]
    fn reparents_a_window_beneath_a_different_parent() {
        let source = forest();
        let patched = apply_wnd_patches(
            &source,
            TARGET,
            &[patch(
                "reparent \"Synthetic.wnd:Gamma\" \"Synthetic.wnd:Alpha\" 0\n",
            )],
            WndLimits::default(),
        )
        .expect("reparent applies");
        let root = &patched.document().windows()[0];
        assert_eq!(child_names(patched.document()), ["Alpha", "Beta"]);
        assert_eq!(
            root.children()[0].children()[0].control_name(),
            Some("Gamma")
        );
    }

    #[test]
    fn refuses_to_reparent_a_window_into_its_own_subtree() {
        let source = forest();
        let error = apply_wnd_patches(
            &source,
            TARGET,
            &[patch(
                "reparent \"Synthetic.wnd:Root\" \"Synthetic.wnd:Alpha\" 0\n",
            )],
            WndLimits::default(),
        )
        .expect_err("a cycle must be refused");
        assert!(matches!(error, WndPatchError::ReparentCycle { .. }));
    }

    #[test]
    fn leaves_the_tree_intact_when_a_reparent_target_is_missing() {
        // The window is detached before its destination is resolved, so a failure here has
        // to put it back rather than drop it.
        let source = forest();
        let error = apply_wnd_patches(
            &source,
            TARGET,
            &[patch(
                "reparent \"Synthetic.wnd:Beta\" \"Synthetic.wnd:Absent\" 0\nreorder \"Synthetic.wnd:Beta\" 0\n",
            )],
            WndLimits::default(),
        )
        .expect_err("missing destination");
        assert!(matches!(error, WndPatchError::MissingControl { .. }));
        assert_eq!(child_names(&source), ["Alpha", "Beta", "Gamma"]);
    }

    #[test]
    fn inserts_a_project_owned_subtree_parsed_by_the_ordinary_decoder() {
        let source = forest();
        let patched = apply_wnd_patches(
            &source,
            TARGET,
            &[patch(
                "insert-window \"Synthetic.wnd:Root\" 1\nWINDOW\n  WINDOWTYPE = COMBOBOX;\n  SCREENRECT = UPPERLEFT: 2 3 BOTTOMRIGHT: 4 5 CREATIONRESOLUTION: 800 600;\n  NAME = \"Synthetic.wnd:ComboRefreshRate\";\n  STATUS = ENABLED;\nEND\nend-window\n",
            )],
            WndLimits::default(),
        )
        .expect("insert applies");

        assert_eq!(
            child_names(patched.document()),
            ["Alpha", "ComboRefreshRate", "Beta", "Gamma"]
        );
        let inserted = &patched.document().windows()[0].children()[1];
        assert_eq!(inserted.window_type(), "COMBOBOX");
        assert_eq!(inserted.rect().upper_left(), (2, 3));
        assert_eq!(
            inserted.status().len(),
            1,
            "an inserted subtree is typed like authored source"
        );

        let ids: Vec<usize> = patched.document().windows()[0]
            .children()
            .iter()
            .map(crate::wnd::WndWindow::id)
            .collect();
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "inserted ids must not collide");

        assert_eq!(
            patched.provenance()[0].control(),
            "Synthetic.wnd:ComboRefreshRate"
        );
        assert_eq!(patched.provenance()[0].field(), "(inserted)");
    }

    #[test]
    fn refuses_an_inserted_subtree_that_reuses_an_existing_name() {
        let source = forest();
        let error = apply_wnd_patches(
            &source,
            TARGET,
            &[patch(
                "insert-window \"Synthetic.wnd:Root\" 0\nWINDOW\n  WINDOWTYPE = PUSHBUTTON;\n  SCREENRECT = UPPERLEFT: 0 0 BOTTOMRIGHT: 1 1 CREATIONRESOLUTION: 800 600;\n  NAME = \"Synthetic.wnd:Beta\";\nEND\nend-window\n",
            )],
            WndLimits::default(),
        )
        .expect_err("duplicate inserted name");
        assert!(matches!(error, WndPatchError::DuplicateInsertedName { .. }));
    }

    #[test]
    fn rejects_out_of_range_indices_and_malformed_or_unterminated_fragments() {
        let source = forest();
        let out_of_range = apply_wnd_patches(
            &source,
            TARGET,
            &[patch("reorder \"Synthetic.wnd:Alpha\" 9\n")],
            WndLimits::default(),
        )
        .expect_err("index beyond the sibling list");
        assert!(matches!(
            out_of_range,
            WndPatchError::IndexOutOfRange { .. }
        ));

        let malformed = apply_wnd_patches(
            &source,
            TARGET,
            &[patch("insert-window \"Synthetic.wnd:Root\" 0\nWINDOW\n  NOTAWINDOWTYPE = 1;\nEND\nend-window\n")],
            WndLimits::default(),
        )
        .expect_err("fragment must satisfy the WND grammar");
        assert!(matches!(malformed, WndPatchError::InvalidFragment { .. }));

        let text = format!(
            "version {WND_PATCH_VERSION}\ntarget {TARGET}\ninsert-window \"Synthetic.wnd:Root\" 0\nWINDOW\n"
        );
        assert!(matches!(
            parse_wnd_patch("t", text.as_bytes(), WndPatchLimits::default()),
            Err(WndPatchError::UnterminatedFragment { .. })
        ));
    }

    #[test]
    fn rejects_a_patch_aimed_at_a_different_layout() {
        let source = document();
        let error = apply_wnd_patches(
            &source,
            "Menus/Other.wnd",
            &[patch("require-window \"Synthetic.wnd:Root\"\n")],
            WndLimits::default(),
        )
        .expect_err("target mismatch");
        assert!(matches!(error, WndPatchError::TargetMismatch { .. }));
    }

    #[test]
    fn matches_the_target_path_case_insensitively_with_normalized_separators() {
        let source = document();
        apply_wnd_patches(
            &source,
            "menus\\synthetic.wnd",
            &[patch("require-window \"Synthetic.wnd:Root\"\n")],
            WndLimits::default(),
        )
        .expect("VFS paths differ in case and separator");
    }

    #[test]
    fn rejects_unsupported_versions_and_missing_directives() {
        let unsupported = format!("version 2\ntarget {TARGET}\n");
        assert!(matches!(
            parse_wnd_patch("t", unsupported.as_bytes(), WndPatchLimits::default()),
            Err(WndPatchError::UnsupportedVersion { .. })
        ));
        assert!(matches!(
            parse_wnd_patch("t", b"target x.wnd\n", WndPatchLimits::default()),
            Err(WndPatchError::MissingVersion)
        ));
        assert!(matches!(
            parse_wnd_patch("t", b"version 1\n", WndPatchLimits::default()),
            Err(WndPatchError::MissingTarget)
        ));
        assert!(matches!(
            parse_wnd_patch(
                "t",
                b"version 1\ntarget x.wnd\ndelete-window \"a\"\n",
                WndPatchLimits::default()
            ),
            Err(WndPatchError::UnknownDirective { .. })
        ));
    }

    #[test]
    fn enforces_every_patch_limit() {
        let default = WndPatchLimits::default();
        let body = format!("version 1\ntarget {TARGET}\nrequire-window \"a\"\n");
        let cases: [WndPatchLimits; 4] = [
            WndPatchLimits {
                maximum_file_bytes: 4,
                ..default
            },
            WndPatchLimits {
                maximum_lines: 1,
                ..default
            },
            WndPatchLimits {
                maximum_operations: 0,
                ..default
            },
            WndPatchLimits {
                maximum_argument_bytes: 2,
                ..default
            },
        ];
        for (index, limits) in cases.into_iter().enumerate() {
            assert!(
                parse_wnd_patch("t", body.as_bytes(), limits).is_err(),
                "case {index} unexpectedly accepted"
            );
        }
    }
}
