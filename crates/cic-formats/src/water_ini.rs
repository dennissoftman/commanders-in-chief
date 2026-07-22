// Commanders in Chief
// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only
//
// The narrow block and field names are established by user-owned Generals `Water.ini` resources.
// This bounded parser is original project code and contains no retail strings beyond identifiers
// required for compatible lookup.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

/// Explicit resource bounds for water-transparency INI inspection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaterIniLimits {
    pub max_file_bytes: usize,
    pub max_lines: usize,
    pub max_line_bytes: usize,
}

impl Default for WaterIniLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: 4 * 1_024 * 1_024,
            max_lines: 100_000,
            max_line_bytes: 4_096,
        }
    }
}

/// Last-definition-wins water-transparency values retained from one INI stream.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WaterTransparencyIni {
    minimum_opacity: Option<f32>,
    opaque_depth: Option<f32>,
}

impl WaterTransparencyIni {
    #[must_use]
    pub const fn minimum_opacity(self) -> Option<f32> {
        self.minimum_opacity
    }

    #[must_use]
    pub const fn opaque_depth(self) -> Option<f32> {
        self.opaque_depth
    }
}

/// A structured failure from bounded water-transparency decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaterIniError {
    FileTooLarge {
        size: usize,
        limit: usize,
    },
    TooManyLines {
        limit: usize,
    },
    LineTooLong {
        line: usize,
        size: usize,
        limit: usize,
    },
    NestedWaterTransparency {
        line: usize,
    },
    UnterminatedWaterTransparency {
        line: usize,
    },
    MissingValue {
        line: usize,
    },
    InvalidNumber {
        line: usize,
    },
    InvalidOpacity {
        line: usize,
    },
    InvalidDepth {
        line: usize,
    },
}

impl Display for WaterIniError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::FileTooLarge { size, limit } => {
                write!(formatter, "Water INI is {size} bytes; limit is {limit}")
            }
            Self::TooManyLines { limit } => write!(formatter, "Water INI exceeds {limit} lines"),
            Self::LineTooLong { line, size, limit } => write!(
                formatter,
                "Water INI line {line} is {size} bytes; limit is {limit}"
            ),
            Self::NestedWaterTransparency { line } => write!(
                formatter,
                "WaterTransparency begins before the prior block ends at line {line}"
            ),
            Self::UnterminatedWaterTransparency { line } => {
                write!(
                    formatter,
                    "WaterTransparency opened on line {line} has no End"
                )
            }
            Self::MissingValue { line } => {
                write!(
                    formatter,
                    "water transparency field has no value on line {line}"
                )
            }
            Self::InvalidNumber { line } => {
                write!(
                    formatter,
                    "water transparency value is not finite on line {line}"
                )
            }
            Self::InvalidOpacity { line } => {
                write!(
                    formatter,
                    "water opacity is outside zero through one on line {line}"
                )
            }
            Self::InvalidDepth { line } => {
                write!(
                    formatter,
                    "water opaque depth is outside valid bounds on line {line}"
                )
            }
        }
    }
}

impl Error for WaterIniError {}

/// Decodes only `WaterTransparency` opacity and depth fields, ignoring unrelated INI blocks.
///
/// Repeated fields and blocks use stable file-order last-definition-wins semantics.
///
/// # Errors
///
/// Returns a structured error for resource-limit excess, malformed block closure, non-finite
/// numbers, opacity outside `[0, 1]`, or depth outside `(0, 10_000]`.
pub fn parse_water_transparency_ini(
    bytes: &[u8],
    limits: WaterIniLimits,
) -> Result<WaterTransparencyIni, WaterIniError> {
    if bytes.len() > limits.max_file_bytes {
        return Err(WaterIniError::FileTooLarge {
            size: bytes.len(),
            limit: limits.max_file_bytes,
        });
    }
    let mut result = WaterTransparencyIni::default();
    let mut water_opened = None;
    let mut other_block = false;
    for (zero_based_line, raw_line) in bytes.split(|byte| *byte == b'\n').enumerate() {
        let line_number = zero_based_line
            .checked_add(1)
            .ok_or(WaterIniError::TooManyLines {
                limit: limits.max_lines,
            })?;
        if line_number > limits.max_lines {
            return Err(WaterIniError::TooManyLines {
                limit: limits.max_lines,
            });
        }
        if raw_line.len() > limits.max_line_bytes {
            return Err(WaterIniError::LineTooLong {
                line: line_number,
                size: raw_line.len(),
                limit: limits.max_line_bytes,
            });
        }
        let line = trim_ascii(strip_comment(raw_line));
        if line.is_empty() {
            continue;
        }
        if line.eq_ignore_ascii_case(b"End") {
            water_opened = None;
            other_block = false;
            continue;
        }
        if line.eq_ignore_ascii_case(b"WaterTransparency") {
            if water_opened.is_some() {
                return Err(WaterIniError::NestedWaterTransparency { line: line_number });
            }
            if !other_block {
                water_opened = Some(line_number);
            }
            continue;
        }
        if water_opened.is_none() {
            if !other_block && !line.contains(&b'=') {
                other_block = true;
            }
            continue;
        }
        let Some((field, value)) = split_assignment(line) else {
            continue;
        };
        if field.eq_ignore_ascii_case(b"TransparentWaterMinOpacity") {
            let value = parse_number(value, line_number)?;
            if !(0.0..=1.0).contains(&value) {
                return Err(WaterIniError::InvalidOpacity { line: line_number });
            }
            result.minimum_opacity = Some(value);
        } else if field.eq_ignore_ascii_case(b"TransparentWaterDepth") {
            let value = parse_number(value, line_number)?;
            if value <= 0.0 || value > 10_000.0 {
                return Err(WaterIniError::InvalidDepth { line: line_number });
            }
            result.opaque_depth = Some(value);
        }
    }
    if let Some(line) = water_opened {
        return Err(WaterIniError::UnterminatedWaterTransparency { line });
    }
    Ok(result)
}

fn parse_number(value: &[u8], line: usize) -> Result<f32, WaterIniError> {
    let value = trim_ascii(value);
    if value.is_empty() {
        return Err(WaterIniError::MissingValue { line });
    }
    let value = std::str::from_utf8(value)
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|value| value.is_finite())
        .ok_or(WaterIniError::InvalidNumber { line })?;
    Ok(value)
}

fn strip_comment(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    line.iter()
        .position(|byte| *byte == b';')
        .map_or(line, |index| &line[..index])
}

fn trim_ascii(mut bytes: &[u8]) -> &[u8] {
    while bytes.first().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[1..];
    }
    while bytes.last().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

fn split_assignment(line: &[u8]) -> Option<(&[u8], &[u8])> {
    let equals = line.iter().position(|byte| *byte == b'=')?;
    Some((trim_ascii(&line[..equals]), trim_ascii(&line[equals + 1..])))
}

#[cfg(test)]
mod tests {
    use super::{WaterIniError, WaterIniLimits, parse_water_transparency_ini};

    #[test]
    fn retains_last_bounded_water_values_and_ignores_water_sets() {
        let parsed = parse_water_transparency_ini(
            b"WaterSet MORNING\n TransparentDiffuseColor = R:1 G:2 B:3 A:128\nEnd\n\
              WaterTransparency\n TransparentWaterMinOpacity = 0.8\n TransparentWaterDepth = 2.0\nEnd\n\
              WaterTransparency\n TransparentWaterMinOpacity = 1.0 ; final\nEnd\n",
            WaterIniLimits::default(),
        )
        .expect("water transparency");
        assert_eq!(
            parsed.minimum_opacity().map(f32::to_bits),
            Some(1.0_f32.to_bits())
        );
        assert_eq!(
            parsed.opaque_depth().map(f32::to_bits),
            Some(2.0_f32.to_bits())
        );
    }

    #[test]
    fn rejects_unterminated_invalid_and_oversized_values() {
        assert_eq!(
            parse_water_transparency_ini(
                b"WaterTransparency\n TransparentWaterMinOpacity = 2\nEnd\n",
                WaterIniLimits::default(),
            ),
            Err(WaterIniError::InvalidOpacity { line: 2 })
        );
        assert_eq!(
            parse_water_transparency_ini(b"WaterTransparency\n", WaterIniLimits::default()),
            Err(WaterIniError::UnterminatedWaterTransparency { line: 1 })
        );
        let limits = WaterIniLimits {
            max_file_bytes: 3,
            ..WaterIniLimits::default()
        };
        assert_eq!(
            parse_water_transparency_ini(b"four", limits),
            Err(WaterIniError::FileTooLarge { size: 4, limit: 3 })
        );
    }
}
