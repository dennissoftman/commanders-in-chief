// Commanders in Chief
// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only
//
// Provenance: the lexical rules implemented here are derived from Electronic Arts' GPL-3.0 source
// release, GeneralsGameCode revision 9f7abb866f5afd446db14149979e744c7216baaf, specifically
// `Core/GameEngine/Source/Common/INI/INI.cpp` (`INI::readLine`, `INI::initFromINIMulti`,
// `INI::getNextToken`, `INI::getNextTokenOrNull`, `INI::getNextSubToken`,
// `INI::getNextQuotedAsciiString`, `INI::getNextAsciiString`, `INI::scanInt`, `INI::scanReal`,
// `INI::scanBool`, `INI::scanIndexList`, `INI::parseBitString32`) and
// `Core/GameEngine/Include/Common/INI.h` (`INI_MAX_CHARS_PER_LINE`, `getSeps`, `getSepsColon`,
// `getSepsQuote`, `getEndToken`). This is a bounded, project-authored implementation and contains
// no retail data.
//
// The established lexical facts these decoders share:
//
// - A line ends at `\n`. A `;` starts a comment and terminates the line at that byte, so comment
//   text is never tokenized. Bytes below 32 become spaces, which makes `\r` insignificant.
// - The default separator set is `" \n\r\t="`, so `=` is only a separator: `Field = value`,
//   `Field=value`, and `Field value` are the same record. `:` joins the set for sub-tokens
//   (`Left:12`), and a quoted string is delimited by `"` and `=` alone.
// - Block keywords and field names are matched with `strcmp`, so both are case-sensitive. The
//   block terminator `End` is matched with `stricmp`, so it is case-insensitive.
// - An unknown field inside a block is skipped with a developer warning; the release client keeps
//   reading. This project retains it as a diagnostic instead of discarding it.
// - Reaching end of file with a block still open is a hard error.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

/// Which narrow UI definition format a bounded decode applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiIniFormat {
    /// `Data/INI/MappedImages/**/*.ini` named texture regions.
    MappedImage,
    /// `Data/<Language>/HeaderTemplate.ini` shared window header presentation.
    HeaderTemplate,
    /// `Data/<Language>/Language.ini` font families and text presentation policy.
    Language,
    /// `Data/INI/WindowTransitions.ini` named shell transition groups.
    WindowTransition,
}

impl UiIniFormat {
    /// Returns the block keyword this format's definitions open with.
    #[must_use]
    pub const fn block_keyword(self) -> &'static str {
        match self {
            Self::MappedImage => "MappedImage",
            Self::HeaderTemplate => "HeaderTemplate",
            Self::Language => "Language",
            Self::WindowTransition => "WindowTransition",
        }
    }
}

impl Display for UiIniFormat {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::MappedImage => "MappedImage INI",
            Self::HeaderTemplate => "HeaderTemplate INI",
            Self::Language => "Language INI",
            Self::WindowTransition => "WindowTransition INI",
        };
        formatter.write_str(name)
    }
}

/// Explicit limits shared by every narrow UI definition decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiIniLimits {
    /// Maximum accepted input size.
    pub max_file_bytes: usize,
    /// Maximum accepted line count.
    pub max_lines: usize,
    /// Maximum accepted bytes on one line.
    pub max_line_bytes: usize,
    /// Maximum accepted definitions in one file.
    pub max_definitions: usize,
    /// Maximum accepted bytes in a definition name.
    pub max_name_bytes: usize,
    /// Maximum accepted bytes in one field value.
    pub max_value_bytes: usize,
    /// Maximum accepted entries in a repeated field.
    pub max_list_entries: usize,
}

impl Default for UiIniLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: 4 * 1_024 * 1_024,
            max_lines: 100_000,
            // The source line buffer is INI_MAX_CHARS_PER_LINE = 1028 and truncates beyond it.
            // This decoder refuses the file instead of silently reading a truncated record, and
            // allows a wider line than the legacy reader so a modded file is reportable.
            max_line_bytes: 4_096,
            max_definitions: 16_384,
            max_name_bytes: 255,
            max_value_bytes: 1_024,
            max_list_entries: 256,
        }
    }
}

/// A non-fatal observation from a narrow UI definition decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiIniDiagnosticKind {
    /// A top-level keyword this decoder does not own. The block is skipped to its `End`.
    ///
    /// The legacy loader rejects the whole file here, because it dispatches every block type in
    /// one table. These decoders each own one keyword, so an unrelated block is data this decoder
    /// is not responsible for rather than a defect.
    UnknownBlock {
        /// The keyword exactly as spelled.
        keyword: Box<str>,
    },
    /// A field name inside an owned block that this decoder does not recognize.
    UnknownField {
        /// The field name exactly as spelled.
        field: Box<str>,
    },
    /// A recognized field whose value did not match its established shape. The typed value is
    /// left at its default rather than guessed.
    MalformedField {
        /// The field name exactly as spelled.
        field: Box<str>,
        /// What the decoder expected.
        reason: Box<str>,
    },
    /// A second definition reused an earlier name. Later fields overwrite earlier ones, matching
    /// the legacy loader, which finds the existing definition and parses into it.
    DuplicateDefinition {
        /// The repeated name exactly as spelled.
        name: Box<str>,
        /// The line the first definition of that name opened on.
        first_line: usize,
    },
}

/// One non-fatal decode observation; never causes a decode to fail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiIniDiagnostic {
    line: usize,
    kind: UiIniDiagnosticKind,
}

impl UiIniDiagnostic {
    pub(crate) const fn new(line: usize, kind: UiIniDiagnosticKind) -> Self {
        Self { line, kind }
    }

    /// Returns the one-based source line the observation applies to.
    #[must_use]
    pub const fn line(&self) -> usize {
        self.line
    }

    /// Returns the observation detail.
    #[must_use]
    pub const fn kind(&self) -> &UiIniDiagnosticKind {
        &self.kind
    }
}

/// A structured, bounded UI definition decoding failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiIniError {
    /// The input exceeds [`UiIniLimits::max_file_bytes`].
    FileTooLarge {
        /// Which format was being decoded.
        format: UiIniFormat,
        /// Actual size.
        size: usize,
        /// Configured limit.
        limit: usize,
    },
    /// The input exceeds [`UiIniLimits::max_lines`].
    TooManyLines {
        /// Which format was being decoded.
        format: UiIniFormat,
        /// Configured limit.
        limit: usize,
    },
    /// One line exceeds [`UiIniLimits::max_line_bytes`].
    LineTooLong {
        /// Which format was being decoded.
        format: UiIniFormat,
        /// One-based line number.
        line: usize,
        /// Actual size.
        size: usize,
        /// Configured limit.
        limit: usize,
    },
    /// The file declares more definitions than [`UiIniLimits::max_definitions`].
    TooManyDefinitions {
        /// Which format was being decoded.
        format: UiIniFormat,
        /// One-based line number.
        line: usize,
        /// Configured limit.
        limit: usize,
    },
    /// A named block opened without its name token.
    MissingBlockName {
        /// Which format was being decoded.
        format: UiIniFormat,
        /// One-based line number.
        line: usize,
    },
    /// A definition name exceeds [`UiIniLimits::max_name_bytes`].
    NameTooLong {
        /// Which format was being decoded.
        format: UiIniFormat,
        /// One-based line number.
        line: usize,
        /// Actual size.
        size: usize,
        /// Configured limit.
        limit: usize,
    },
    /// A field value exceeds [`UiIniLimits::max_value_bytes`].
    ValueTooLong {
        /// Which format was being decoded.
        format: UiIniFormat,
        /// One-based line number.
        line: usize,
        /// Field name exactly as spelled.
        field: Box<str>,
        /// Actual size.
        size: usize,
        /// Configured limit.
        limit: usize,
    },
    /// A repeated field exceeds [`UiIniLimits::max_list_entries`].
    TooManyListEntries {
        /// Which format was being decoded.
        format: UiIniFormat,
        /// One-based line number.
        line: usize,
        /// Field name exactly as spelled.
        field: Box<str>,
        /// Configured limit.
        limit: usize,
    },
    /// A block opened but the file ended before its `End`.
    UnterminatedBlock {
        /// Which format was being decoded.
        format: UiIniFormat,
        /// One-based line the block opened on.
        line: usize,
    },
}

impl Display for UiIniError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::FileTooLarge {
                format,
                size,
                limit,
            } => write!(formatter, "{format} is {size} bytes; limit is {limit}"),
            Self::TooManyLines { format, limit } => {
                write!(formatter, "{format} exceeds the {limit}-line limit")
            }
            Self::LineTooLong {
                format,
                line,
                size,
                limit,
            } => write!(
                formatter,
                "{format} line {line} is {size} bytes; limit is {limit}"
            ),
            Self::TooManyDefinitions {
                format,
                line,
                limit,
            } => write!(
                formatter,
                "{format} exceeds the {limit}-definition limit at line {line}"
            ),
            Self::MissingBlockName { format, line } => write!(
                formatter,
                "{format} block on line {line} has no {} name",
                format.block_keyword()
            ),
            Self::NameTooLong {
                format,
                line,
                size,
                limit,
            } => write!(
                formatter,
                "{format} name on line {line} is {size} bytes; limit is {limit}"
            ),
            Self::ValueTooLong {
                format,
                line,
                field,
                size,
                limit,
            } => write!(
                formatter,
                "{format} field {field:?} on line {line} is {size} bytes; limit is {limit}"
            ),
            Self::TooManyListEntries {
                format,
                line,
                field,
                limit,
            } => write!(
                formatter,
                "{format} field {field:?} on line {line} exceeds the {limit}-entry limit"
            ),
            Self::UnterminatedBlock { format, line } => {
                write!(formatter, "{format} block opened on line {line} has no End")
            }
        }
    }
}

impl Error for UiIniError {}

/// Separator sets. Every byte below 33 is whitespace because the source line reader rewrites
/// control bytes as spaces before tokenizing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Separators {
    /// `" \n\r\t="`.
    Default,
    /// `" \n\r\t=:"`, used by `Left:12`-style sub-tokens.
    Colon,
    /// `"\"\n="`, used inside a quoted string, where spaces are content.
    Quote,
}

impl Separators {
    const fn is_separator(self, byte: u8) -> bool {
        match self {
            Self::Default => byte <= b' ' || byte == b'=',
            Self::Colon => byte <= b' ' || byte == b'=' || byte == b':',
            Self::Quote => byte == b'"' || byte == b'=',
        }
    }
}

/// A bounded line reader over one UI definition file.
///
/// Yields cleaned lines: comment text removed, terminated at the first `;` or NUL byte, with the
/// trailing newline dropped. Whitespace is left in place for the tokenizer.
pub(crate) struct LineReader<'a> {
    bytes: &'a [u8],
    next: usize,
    line: usize,
    format: UiIniFormat,
    limits: UiIniLimits,
}

impl<'a> LineReader<'a> {
    /// Creates a reader, rejecting an oversized input up front.
    pub(crate) fn new(
        bytes: &'a [u8],
        format: UiIniFormat,
        limits: UiIniLimits,
    ) -> Result<Self, UiIniError> {
        if bytes.len() > limits.max_file_bytes {
            return Err(UiIniError::FileTooLarge {
                format,
                size: bytes.len(),
                limit: limits.max_file_bytes,
            });
        }
        Ok(Self {
            bytes,
            next: 0,
            line: 0,
            format,
            limits,
        })
    }

    /// Returns the next cleaned line and its one-based number, or `None` at end of input.
    pub(crate) fn next_line(&mut self) -> Result<Option<(usize, &'a [u8])>, UiIniError> {
        if self.next >= self.bytes.len() {
            return Ok(None);
        }
        let remainder = &self.bytes[self.next..];
        let raw_length = remainder
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(remainder.len(), |index| index + 1);
        let raw = &remainder[..raw_length];
        self.next += raw_length;
        self.line += 1;
        if self.line > self.limits.max_lines {
            return Err(UiIniError::TooManyLines {
                format: self.format,
                limit: self.limits.max_lines,
            });
        }
        if raw.len() > self.limits.max_line_bytes {
            return Err(UiIniError::LineTooLong {
                format: self.format,
                line: self.line,
                size: raw.len(),
                limit: self.limits.max_line_bytes,
            });
        }
        let body = raw.strip_suffix(b"\n").unwrap_or(raw);
        let cleaned = body
            .iter()
            .position(|byte| *byte == b';' || *byte == 0)
            .map_or(body, |index| &body[..index]);
        Ok(Some((self.line, cleaned)))
    }
}

/// A `strtok`-shaped tokenizer over one cleaned line.
///
/// Each call consumes leading separators, then returns bytes up to the next separator. The
/// separator set may change between calls, exactly as the source alternates `getSeps`,
/// `getSepsColon`, and `getSepsQuote` while reading one record.
#[derive(Debug, Clone)]
pub(crate) struct Tokens<'a> {
    rest: &'a [u8],
}

impl<'a> Tokens<'a> {
    pub(crate) const fn new(line: &'a [u8]) -> Self {
        Self { rest: line }
    }

    /// Returns the next token under `separators`, or `None` when the line is exhausted.
    pub(crate) fn next(&mut self, separators: Separators) -> Option<&'a [u8]> {
        let start = self
            .rest
            .iter()
            .position(|byte| !separators.is_separator(*byte))?;
        let tail = &self.rest[start..];
        let end = tail
            .iter()
            .position(|byte| separators.is_separator(*byte))
            .unwrap_or(tail.len());
        let (token, remainder) = tail.split_at(end);
        // Consume the delimiter itself, matching strtok's replacement of it with a terminator.
        self.rest = remainder.get(1..).unwrap_or(&[]);
        Some(token)
    }

    /// Returns the value of a labelled sub-token such as `Left:12`.
    ///
    /// The label is compared case-insensitively; the source uses `stricmp` here even though field
    /// names are case-sensitive.
    pub(crate) fn next_sub_token(&mut self, expected: &str) -> Option<&'a [u8]> {
        let label = self.next(Separators::Colon)?;
        if !label.eq_ignore_ascii_case(expected.as_bytes()) {
            return None;
        }
        self.next(Separators::Colon)
    }

    /// Returns the next possibly-quoted string.
    ///
    /// Mirrors `INI::getNextQuotedAsciiString`: an unquoted token is returned as-is, a token fully
    /// enclosed in quotes has them stripped, and a quoted run spanning separators is rejoined with
    /// a single space before its closing quote. `None` means the line ended inside the quote, which
    /// the source treats as invalid data.
    ///
    /// Two source quirks are reproduced rather than corrected, because a definition written against
    /// the original reader must resolve to the same value here. A quoted string is rejoined from at
    /// most two tokens, so `"a b c"` yields `a b c` only because the second token runs to the
    /// closing quote — but a continuation of exactly one character is dropped, so `"Synth 0"`
    /// yields `Synth`. Runs of separators inside the quote also collapse to one space.
    pub(crate) fn next_quoted_string(&mut self) -> Option<Vec<u8>> {
        let token = self.next(Separators::Default)?;
        if token.first() != Some(&b'"') {
            return Some(token.to_vec());
        }
        let mut value = Vec::from(&token[1..]);
        if !value.is_empty() && value.last() == Some(&b'"') {
            value.pop();
            return Some(value);
        }
        let remainder = self.next(Separators::Quote)?;
        if remainder.len() > 1 && remainder.get(1) != Some(&b'\t') {
            value.push(b' ');
            value.extend_from_slice(remainder);
        } else if value.last() == Some(&b'"') {
            value.pop();
        }
        Some(value)
    }

    /// Returns the next possibly-quoted string, tolerating a missing closing quote.
    ///
    /// Mirrors `INI::getNextAsciiString`, which reads the continuation with the non-throwing
    /// token accessor and strips a dangling quote instead of failing.
    pub(crate) fn next_ascii_string(&mut self) -> Option<Vec<u8>> {
        let token = self.next(Separators::Default)?;
        if token.first() != Some(&b'"') {
            return Some(token.to_vec());
        }
        let mut value = Vec::from(&token[1..]);
        match self.next(Separators::Quote) {
            Some(remainder) => {
                if remainder.len() > 1 && remainder.get(1) != Some(&b'\t') {
                    value.push(b' ');
                }
                value.extend_from_slice(remainder);
            }
            None => {
                if value.last() == Some(&b'"') {
                    value.pop();
                }
            }
        }
        Some(value)
    }
}

/// Returns whether a token is the case-insensitive block terminator.
pub(crate) fn is_end_token(token: &[u8]) -> bool {
    token.eq_ignore_ascii_case(b"End")
}

/// Parses a signed decimal integer the way `INI::scanInt` does.
///
/// `sscanf("%d")` and `std::from_chars` both accept a leading sign and stop at the first byte that
/// cannot extend the number, ignoring whatever follows, so `12abc` is twelve. A token with no
/// digits at all, or one that does not fit in the source's 32-bit `Int`, is rejected.
pub(crate) fn scan_int(token: &[u8]) -> Option<i32> {
    let (negative, digits) = match token.first() {
        Some(b'-') => (true, &token[1..]),
        Some(b'+') => (false, &token[1..]),
        _ => (false, token),
    };
    let count = digits
        .iter()
        .position(|byte| !byte.is_ascii_digit())
        .unwrap_or(digits.len());
    if count == 0 {
        return None;
    }
    let mut value: i64 = 0;
    for byte in &digits[..count] {
        value = value.checked_mul(10)?.checked_add(i64::from(byte - b'0'))?;
        if value > i64::from(i32::MAX) + 1 {
            return None;
        }
    }
    let signed = if negative { -value } else { value };
    i32::try_from(signed).ok()
}

/// Parses a real the way `INI::scanReal` does, accepting the longest numeric prefix.
pub(crate) fn scan_real(token: &[u8]) -> Option<f32> {
    let mut end = 0;
    let mut seen_digit = false;
    let mut seen_point = false;
    let mut seen_exponent = false;
    for (index, byte) in token.iter().enumerate() {
        let accepted = match byte {
            b'+' | b'-' => index == 0 || matches!(token.get(index - 1), Some(b'e' | b'E')),
            b'0'..=b'9' => {
                seen_digit = true;
                true
            }
            b'.' if !seen_point && !seen_exponent => {
                seen_point = true;
                true
            }
            b'e' | b'E' if seen_digit && !seen_exponent => {
                seen_exponent = true;
                true
            }
            _ => false,
        };
        if !accepted {
            break;
        }
        end = index + 1;
    }
    if !seen_digit {
        return None;
    }
    let text = std::str::from_utf8(&token[..end]).ok()?;
    // Drop a trailing exponent marker or sign that no digits followed; `%f` would not have
    // consumed it either.
    text.trim_end_matches(['e', 'E', '+', '-'])
        .parse::<f32>()
        .ok()
}

/// Parses the source's only boolean spelling: `Yes` or `No`, case-insensitively.
pub(crate) fn scan_bool(token: &[u8]) -> Option<bool> {
    if token.eq_ignore_ascii_case(b"yes") {
        Some(true)
    } else if token.eq_ignore_ascii_case(b"no") {
        Some(false)
    } else {
        None
    }
}

/// Returns a name's index in an ordered vocabulary, compared case-insensitively like
/// `INI::scanIndexList`.
pub(crate) fn index_of_name(token: &[u8], names: &[&str]) -> Option<usize> {
    names
        .iter()
        .position(|name| token.eq_ignore_ascii_case(name.as_bytes()))
}

/// Parses a `parseBitString32`-shaped flag list into a bit set.
///
/// `NONE` clears the set and ends the list. A leading `+` or `-` adds or removes one flag from the
/// current value; bare names replace the value. The source refuses to mix bare names with `+`/`-`
/// in one list, and refuses any name outside the vocabulary.
pub(crate) fn scan_bit_string(
    tokens: &mut Tokens<'_>,
    names: &[&str],
    current: u32,
) -> Result<u32, BitStringError> {
    let mut bits = current;
    let mut saw_bare = false;
    let mut saw_signed = false;
    while let Some(token) = tokens.next(Separators::Default) {
        if token.eq_ignore_ascii_case(b"NONE") {
            if saw_bare || saw_signed {
                return Err(BitStringError::MixedOperators);
            }
            return Ok(0);
        }
        let (sign, name) = match token.first() {
            Some(b'+') => (Some(true), &token[1..]),
            Some(b'-') => (Some(false), &token[1..]),
            _ => (None, token),
        };
        let index = index_of_name(name, names).ok_or_else(|| BitStringError::UnknownName {
            name: diagnostic_text(name),
        })?;
        let shift = u32::try_from(index).map_err(|_| BitStringError::FlagOutOfRange)?;
        let bit = 1_u32
            .checked_shl(shift)
            .ok_or(BitStringError::FlagOutOfRange)?;
        match sign {
            Some(true) => {
                if saw_bare {
                    return Err(BitStringError::MixedOperators);
                }
                saw_signed = true;
                bits |= bit;
            }
            Some(false) => {
                if saw_bare {
                    return Err(BitStringError::MixedOperators);
                }
                saw_signed = true;
                bits &= !bit;
            }
            None => {
                if saw_signed {
                    return Err(BitStringError::MixedOperators);
                }
                if !saw_bare {
                    bits = 0;
                }
                saw_bare = true;
                bits |= bit;
            }
        }
    }
    Ok(bits)
}

/// Why a flag list could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BitStringError {
    /// A name outside the established vocabulary.
    UnknownName {
        /// The name exactly as spelled.
        name: Box<str>,
    },
    /// Bare names mixed with `+`/`-` operators in one list.
    MixedOperators,
    /// A vocabulary entry past bit 31, which the source's 32-bit flag word cannot represent.
    FlagOutOfRange,
}

impl BitStringError {
    /// Returns a stable diagnostic reason for this failure.
    pub(crate) fn reason(&self) -> Box<str> {
        match self {
            Self::UnknownName { name } => format!("unknown flag name {name:?}").into_boxed_str(),
            Self::MixedOperators => "flag list mixes bare names with + or - operators"
                .to_owned()
                .into_boxed_str(),
            Self::FlagOutOfRange => "flag name is past the 32-bit flag word"
                .to_owned()
                .into_boxed_str(),
        }
    }
}

/// Converts raw bytes into a lossless-if-possible owned string for diagnostics.
pub(crate) fn diagnostic_text(bytes: &[u8]) -> Box<str> {
    String::from_utf8_lossy(bytes).into_owned().into_boxed_str()
}

#[cfg(test)]
mod tests {
    use super::{BitStringError, LineReader, UiIniError};
    use super::{
        Separators, Tokens, UiIniFormat, UiIniLimits, is_end_token, scan_bit_string, scan_bool,
        scan_int, scan_real,
    };

    #[test]
    fn lines_drop_comments_and_count_from_one() {
        let mut reader = LineReader::new(
            b"MappedImage Name ; trailing comment\r\n  Texture = a.tga\n; whole line\nEnd",
            UiIniFormat::MappedImage,
            UiIniLimits::default(),
        )
        .expect("create reader");
        assert_eq!(
            reader.next_line().expect("line 1"),
            Some((1, b"MappedImage Name ".as_slice()))
        );
        assert_eq!(
            reader.next_line().expect("line 2"),
            Some((2, b"  Texture = a.tga".as_slice()))
        );
        assert_eq!(
            reader.next_line().expect("line 3"),
            Some((3, b"".as_slice()))
        );
        assert_eq!(
            reader.next_line().expect("line 4"),
            Some((4, b"End".as_slice()))
        );
        assert_eq!(reader.next_line().expect("end of input"), None);
    }

    #[test]
    fn line_limits_are_explicit() {
        let limits = UiIniLimits {
            max_file_bytes: 4,
            ..UiIniLimits::default()
        };
        assert_eq!(
            LineReader::new(b"12345", UiIniFormat::Language, limits).err(),
            Some(UiIniError::FileTooLarge {
                format: UiIniFormat::Language,
                size: 5,
                limit: 4,
            })
        );
        let limits = UiIniLimits {
            max_lines: 1,
            ..UiIniLimits::default()
        };
        let mut reader =
            LineReader::new(b"a\nb\n", UiIniFormat::Language, limits).expect("create reader");
        assert!(reader.next_line().is_ok());
        assert_eq!(
            reader.next_line().err(),
            Some(UiIniError::TooManyLines {
                format: UiIniFormat::Language,
                limit: 1,
            })
        );
        let limits = UiIniLimits {
            max_line_bytes: 2,
            ..UiIniLimits::default()
        };
        let mut reader =
            LineReader::new(b"abc\n", UiIniFormat::Language, limits).expect("create reader");
        assert_eq!(
            reader.next_line().err(),
            Some(UiIniError::LineTooLong {
                format: UiIniFormat::Language,
                line: 1,
                size: 4,
                limit: 2,
            })
        );
    }

    #[test]
    fn equals_and_colons_are_separators() {
        let mut tokens = Tokens::new(b"  Coords = Left:12 Top:34  ");
        assert_eq!(tokens.next(Separators::Default), Some(b"Coords".as_slice()));
        assert_eq!(tokens.next_sub_token("left"), Some(b"12".as_slice()));
        assert_eq!(tokens.next_sub_token("Top"), Some(b"34".as_slice()));
        assert_eq!(tokens.next(Separators::Default), None);
    }

    #[test]
    fn quoted_strings_rejoin_across_separators() {
        let mut tokens = Tokens::new(b"Font = \"Arial\" 12 Yes");
        assert_eq!(tokens.next(Separators::Default), Some(b"Font".as_slice()));
        assert_eq!(tokens.next_quoted_string(), Some(b"Arial".to_vec()));
        assert_eq!(tokens.next(Separators::Default), Some(b"12".as_slice()));
        assert_eq!(tokens.next(Separators::Default), Some(b"Yes".as_slice()));

        let mut tokens = Tokens::new(b"CopyrightFont = \"Arial Unicode MS\" 14 No");
        assert_eq!(
            tokens.next(Separators::Default),
            Some(b"CopyrightFont".as_slice())
        );
        assert_eq!(
            tokens.next_quoted_string(),
            Some(b"Arial Unicode MS".to_vec())
        );
        assert_eq!(tokens.next(Separators::Default), Some(b"14".as_slice()));
        assert_eq!(tokens.next(Separators::Default), Some(b"No".as_slice()));

        // An unquoted value is returned verbatim, as the source does.
        let mut tokens = Tokens::new(b"Font = Arial");
        assert_eq!(tokens.next(Separators::Default), Some(b"Font".as_slice()));
        assert_eq!(tokens.next_quoted_string(), Some(b"Arial".to_vec()));

        // A quote that never closes is invalid data rather than a silent empty value.
        let mut tokens = Tokens::new(b"Font = \"Arial");
        assert_eq!(tokens.next(Separators::Default), Some(b"Font".as_slice()));
        assert_eq!(tokens.next_quoted_string(), None);

        // Reproduced quirk: a one-character continuation is dropped by the source reader.
        let mut tokens = Tokens::new(b"Font = \"Synth 0\" 5 No");
        assert_eq!(tokens.next(Separators::Default), Some(b"Font".as_slice()));
        assert_eq!(tokens.next_quoted_string(), Some(b"Synth".to_vec()));

        // An unquoted multi-word value keeps only its first token, as `getNextAsciiString` does.
        let mut tokens = Tokens::new(b"UnicodeFontName = Synth Unicode");
        assert_eq!(
            tokens.next(Separators::Default),
            Some(b"UnicodeFontName".as_slice())
        );
        assert_eq!(tokens.next_ascii_string(), Some(b"Synth".to_vec()));

        let mut tokens = Tokens::new(b"UnicodeFontName = \"Synth Unicode\"");
        assert_eq!(
            tokens.next(Separators::Default),
            Some(b"UnicodeFontName".as_slice())
        );
        assert_eq!(tokens.next_ascii_string(), Some(b"Synth Unicode".to_vec()));
    }

    #[test]
    fn scanners_match_the_source_shapes() {
        assert_eq!(scan_int(b"12"), Some(12));
        assert_eq!(scan_int(b"-7"), Some(-7));
        assert_eq!(scan_int(b"+7"), Some(7));
        assert_eq!(scan_int(b"12abc"), Some(12));
        assert_eq!(scan_int(b"abc"), None);
        assert_eq!(scan_int(b""), None);
        assert_eq!(scan_int(b"2147483648"), None);
        assert_eq!(scan_int(b"-2147483648"), Some(i32::MIN));

        assert_eq!(scan_real(b"0.7"), Some(0.7));
        assert_eq!(scan_real(b"-1"), Some(-1.0));
        assert_eq!(scan_real(b"1.5e2"), Some(150.0));
        assert_eq!(scan_real(b"1.5units"), Some(1.5));
        assert_eq!(scan_real(b"units"), None);

        assert_eq!(scan_bool(b"yes"), Some(true));
        assert_eq!(scan_bool(b"NO"), Some(false));
        assert_eq!(scan_bool(b"true"), None);

        assert!(is_end_token(b"end"));
        assert!(is_end_token(b"END"));
        assert!(!is_end_token(b"Ended"));
    }

    #[test]
    fn flag_lists_follow_the_none_and_operator_rules() {
        let names = ["ROTATED_90_CLOCKWISE", "RAW_TEXTURE"];
        let mut tokens = Tokens::new(b"ROTATED_90_CLOCKWISE RAW_TEXTURE");
        assert_eq!(scan_bit_string(&mut tokens, &names, 0), Ok(0b11));

        let mut tokens = Tokens::new(b"NONE");
        assert_eq!(scan_bit_string(&mut tokens, &names, 0b11), Ok(0));

        let mut tokens = Tokens::new(b"-RAW_TEXTURE");
        assert_eq!(scan_bit_string(&mut tokens, &names, 0b11), Ok(0b01));

        let mut tokens = Tokens::new(b"RAW_TEXTURE +ROTATED_90_CLOCKWISE");
        assert_eq!(
            scan_bit_string(&mut tokens, &names, 0),
            Err(BitStringError::MixedOperators)
        );

        let mut tokens = Tokens::new(b"SIDEWAYS");
        assert_eq!(
            scan_bit_string(&mut tokens, &names, 0),
            Err(BitStringError::UnknownName {
                name: "SIDEWAYS".to_owned().into_boxed_str(),
            })
        );
    }
}
