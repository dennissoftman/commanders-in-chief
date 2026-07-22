// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Bounded immutable decoder for renderer-facing `Object` INI model declarations.
//!
//! `Object`, `ObjectReskin`, `Draw`, `DefaultConditionState`, `Model`, and `Scale` field meanings
//! are derived from `W3DModelDraw.cpp`, `W3DModelDraw.h`, and `INI.cpp` in `GeneralsGameCode`
//! revision `9f7abb866f5afd446db14149979e744c7216baaf`, licensed under GPL-3.0-or-later
//! with Electronic Arts Section 7 terms. Full notices are recorded in
//! `docs/provenance/map.md`. Indentation-based extraction and resource limits are project-authored.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

/// Explicit input and allocation bounds for [`parse_object_ini`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectIniLimits {
    pub max_file_bytes: usize,
    pub max_lines: usize,
    pub max_line_bytes: usize,
    pub max_definitions: usize,
    pub max_draws: usize,
    pub max_name_bytes: usize,
    pub max_module_bytes: usize,
    pub max_model_bytes: usize,
}

impl Default for ObjectIniLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: 8 * 1_024 * 1_024,
            max_lines: 250_000,
            max_line_bytes: 8_192,
            max_definitions: 32_768,
            max_draws: 131_072,
            max_name_bytes: 255,
            max_module_bytes: 255,
            max_model_bytes: 1_024,
        }
    }
}

/// One source-ordered initial model selected by a W3D draw module.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectModelDraw {
    module: Vec<u8>,
    model: Vec<u8>,
    scale: f32,
}

impl ObjectModelDraw {
    #[must_use]
    pub fn module_bytes(&self) -> &[u8] {
        &self.module
    }

    #[must_use]
    pub fn model_bytes(&self) -> &[u8] {
        &self.model
    }

    #[must_use]
    pub const fn scale(&self) -> f32 {
        self.scale
    }
}

/// One immutable object template and its renderer-facing default models.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectDefinition {
    name: Vec<u8>,
    reskin_of: Option<Vec<u8>>,
    draws: Vec<ObjectModelDraw>,
}

impl ObjectDefinition {
    #[must_use]
    pub fn name_bytes(&self) -> &[u8] {
        &self.name
    }

    #[must_use]
    pub fn reskin_of_bytes(&self) -> Option<&[u8]> {
        self.reskin_of.as_deref()
    }

    #[must_use]
    pub fn draws(&self) -> &[ObjectModelDraw] {
        &self.draws
    }
}

/// Immutable source-order object declarations from one INI provider.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectIni {
    definitions: Vec<ObjectDefinition>,
}

impl ObjectIni {
    #[must_use]
    pub fn definitions(&self) -> &[ObjectDefinition] {
        &self.definitions
    }
}

#[derive(Debug)]
struct ActiveObject {
    name: Vec<u8>,
    reskin_of: Option<Vec<u8>>,
    draws: Vec<ObjectModelDraw>,
}

#[derive(Debug)]
struct ActiveDraw {
    indent: usize,
    module: Vec<u8>,
    model: Option<Vec<u8>>,
    scale: f32,
    default_condition_indent: Option<usize>,
}

/// Structured object-definition decoding failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectIniError {
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
    MissingObjectName {
        line: usize,
    },
    MissingReskinBase {
        line: usize,
    },
    ValueTooLong {
        line: usize,
        field: &'static str,
        size: usize,
        limit: usize,
    },
    TooManyDefinitions {
        line: usize,
        limit: usize,
    },
    TooManyDraws {
        line: usize,
        limit: usize,
    },
    InvalidScale {
        line: usize,
    },
}

impl Display for ObjectIniError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::FileTooLarge { size, limit } => {
                write!(formatter, "Object INI is {size} bytes; limit is {limit}")
            }
            Self::TooManyLines { limit } => write!(formatter, "Object INI exceeds {limit} lines"),
            Self::LineTooLong { line, size, limit } => write!(
                formatter,
                "Object INI line {line} is {size} bytes; limit is {limit}"
            ),
            Self::MissingObjectName { line } => {
                write!(formatter, "Object INI line {line} has no object name")
            }
            Self::MissingReskinBase { line } => {
                write!(
                    formatter,
                    "ObjectReskin on line {line} has no base template"
                )
            }
            Self::ValueTooLong {
                line,
                field,
                size,
                limit,
            } => write!(
                formatter,
                "Object INI {field} on line {line} is {size} bytes; limit is {limit}"
            ),
            Self::TooManyDefinitions { line, limit } => write!(
                formatter,
                "Object INI exceeds {limit} definitions at line {line}"
            ),
            Self::TooManyDraws { line, limit } => {
                write!(
                    formatter,
                    "Object INI exceeds {limit} model draws at line {line}"
                )
            }
            Self::InvalidScale { line } => {
                write!(
                    formatter,
                    "Object INI Scale on line {line} is not positive and finite"
                )
            }
        }
    }
}

impl Error for ObjectIniError {}

/// Extracts default W3D model presentation from object definitions without executing behavior.
///
/// Object bodies contain heterogeneous modules with independent `End` tokens. The source INI
/// convention uses indentation for those nested declarations, so this narrow decoder uses bounded
/// indentation columns to isolate W3D draw/default-state fields and ignores all gameplay fields.
/// Provider overlay and reskin resolution remain caller policy.
///
/// # Errors
///
/// Returns a structured error for malformed declarations, invalid scales, or an explicit limit.
#[allow(clippy::too_many_lines)]
pub fn parse_object_ini(
    bytes: &[u8],
    limits: ObjectIniLimits,
) -> Result<ObjectIni, ObjectIniError> {
    if bytes.len() > limits.max_file_bytes {
        return Err(ObjectIniError::FileTooLarge {
            size: bytes.len(),
            limit: limits.max_file_bytes,
        });
    }
    let mut definitions = Vec::new();
    let mut active_object = None;
    let mut active_draw = None;
    let mut draw_count = 0_usize;
    for (zero_based_line, raw_line) in bytes.split(|byte| *byte == b'\n').enumerate() {
        let line_number = zero_based_line
            .checked_add(1)
            .ok_or(ObjectIniError::TooManyLines {
                limit: limits.max_lines,
            })?;
        if line_number > limits.max_lines {
            return Err(ObjectIniError::TooManyLines {
                limit: limits.max_lines,
            });
        }
        if raw_line.len() > limits.max_line_bytes {
            return Err(ObjectIniError::LineTooLong {
                line: line_number,
                size: raw_line.len(),
                limit: limits.max_line_bytes,
            });
        }
        let uncommented = strip_comment(raw_line);
        let indent = indentation_columns(uncommented);
        let line = trim_ascii(uncommented);
        if line.is_empty() {
            continue;
        }
        if indent == 0 && (token_eq(line, b"Object") || token_eq(line, b"ObjectReskin")) {
            finish_draw(&mut active_object, &mut active_draw);
            finish_object(
                &mut definitions,
                &mut active_object,
                line_number,
                limits.max_definitions,
            )?;
            active_object = Some(parse_object_header(line, line_number, limits)?);
            continue;
        }
        if active_object.is_none() {
            continue;
        }
        if indent == 0 && line.eq_ignore_ascii_case(b"End") {
            finish_draw(&mut active_object, &mut active_draw);
            finish_object(
                &mut definitions,
                &mut active_object,
                line_number,
                limits.max_definitions,
            )?;
            continue;
        }
        if active_draw
            .as_ref()
            .is_some_and(|draw| indent <= draw.indent)
        {
            finish_draw(&mut active_object, &mut active_draw);
        }
        if field_eq(line, b"Draw") {
            let value = field_value(line).unwrap_or_default();
            let module = value
                .split(u8::is_ascii_whitespace)
                .find(|token| !token.is_empty())
                .unwrap_or_default();
            if is_w3d_draw_module(module) {
                enforce_value_limit(module, line_number, "Draw module", limits.max_module_bytes)?;
                draw_count = draw_count
                    .checked_add(1)
                    .ok_or(ObjectIniError::TooManyDraws {
                        line: line_number,
                        limit: limits.max_draws,
                    })?;
                if draw_count > limits.max_draws {
                    return Err(ObjectIniError::TooManyDraws {
                        line: line_number,
                        limit: limits.max_draws,
                    });
                }
                active_draw = Some(ActiveDraw {
                    indent,
                    module: module.to_vec(),
                    model: None,
                    scale: 1.0,
                    default_condition_indent: None,
                });
            }
            continue;
        }
        let Some(draw) = active_draw.as_mut() else {
            continue;
        };
        if draw
            .default_condition_indent
            .is_some_and(|condition_indent| indent <= condition_indent)
        {
            draw.default_condition_indent = None;
        }
        if token_eq(line, b"DefaultConditionState") && indent > draw.indent {
            draw.default_condition_indent = Some(indent);
            continue;
        }
        if draw.default_condition_indent.is_some() && field_eq(line, b"Model") {
            let value = field_value(line).unwrap_or_default();
            if !value.is_empty() && !value.eq_ignore_ascii_case(b"NONE") {
                enforce_value_limit(value, line_number, "Model", limits.max_model_bytes)?;
                draw.model = Some(value.to_vec());
            }
            continue;
        }
        if field_eq(line, b"Scale") && indent > draw.indent {
            let value = field_value(line).unwrap_or_default();
            let scale = std::str::from_utf8(value)
                .ok()
                .and_then(|value| value.parse::<f32>().ok())
                .filter(|value| value.is_finite() && *value > 0.0)
                .ok_or(ObjectIniError::InvalidScale { line: line_number })?;
            draw.scale = scale;
        }
    }
    finish_draw(&mut active_object, &mut active_draw);
    finish_object(
        &mut definitions,
        &mut active_object,
        limits.max_lines,
        limits.max_definitions,
    )?;
    Ok(ObjectIni { definitions })
}

fn parse_object_header(
    line: &[u8],
    line_number: usize,
    limits: ObjectIniLimits,
) -> Result<ActiveObject, ObjectIniError> {
    let reskin = token_eq(line, b"ObjectReskin");
    let keyword_length = if reskin {
        b"ObjectReskin".len()
    } else {
        b"Object".len()
    };
    let mut tokens = trim_ascii(&line[keyword_length..])
        .split(u8::is_ascii_whitespace)
        .filter(|token| !token.is_empty());
    let name = tokens
        .next()
        .ok_or(ObjectIniError::MissingObjectName { line: line_number })?;
    enforce_value_limit(name, line_number, "object name", limits.max_name_bytes)?;
    let reskin_of = if reskin {
        let base = tokens
            .next()
            .ok_or(ObjectIniError::MissingReskinBase { line: line_number })?;
        enforce_value_limit(base, line_number, "reskin base", limits.max_name_bytes)?;
        Some(base.to_vec())
    } else {
        None
    };
    Ok(ActiveObject {
        name: name.to_vec(),
        reskin_of,
        draws: Vec::new(),
    })
}

fn finish_draw(object: &mut Option<ActiveObject>, draw: &mut Option<ActiveDraw>) {
    let Some(draw) = draw.take() else {
        return;
    };
    let Some(model) = draw.model else {
        return;
    };
    if let Some(object) = object.as_mut() {
        object.draws.push(ObjectModelDraw {
            module: draw.module,
            model,
            scale: draw.scale,
        });
    }
}

fn finish_object(
    definitions: &mut Vec<ObjectDefinition>,
    active: &mut Option<ActiveObject>,
    line: usize,
    limit: usize,
) -> Result<(), ObjectIniError> {
    let Some(object) = active.take() else {
        return Ok(());
    };
    if definitions.len() >= limit {
        return Err(ObjectIniError::TooManyDefinitions { line, limit });
    }
    definitions.push(ObjectDefinition {
        name: object.name,
        reskin_of: object.reskin_of,
        draws: object.draws,
    });
    Ok(())
}

fn enforce_value_limit(
    value: &[u8],
    line: usize,
    field: &'static str,
    limit: usize,
) -> Result<(), ObjectIniError> {
    if value.len() > limit {
        Err(ObjectIniError::ValueTooLong {
            line,
            field,
            size: value.len(),
            limit,
        })
    } else {
        Ok(())
    }
}

fn field_eq(line: &[u8], expected: &[u8]) -> bool {
    line.iter()
        .position(|byte| *byte == b'=')
        .is_some_and(|equals| trim_ascii(&line[..equals]).eq_ignore_ascii_case(expected))
}

fn field_value(line: &[u8]) -> Option<&[u8]> {
    let equals = line.iter().position(|byte| *byte == b'=')?;
    Some(trim_ascii(&line[equals + 1..]))
}

fn token_eq(line: &[u8], expected: &[u8]) -> bool {
    line.len() >= expected.len()
        && line[..expected.len()].eq_ignore_ascii_case(expected)
        && line
            .get(expected.len())
            .is_none_or(|byte| byte.is_ascii_whitespace() || *byte == b'=')
}

fn is_w3d_draw_module(module: &[u8]) -> bool {
    module.len() >= 7
        && module[..3].eq_ignore_ascii_case(b"W3D")
        && module
            .windows(4)
            .any(|window| window.eq_ignore_ascii_case(b"Draw"))
}

fn strip_comment(line: &[u8]) -> &[u8] {
    let length = line
        .iter()
        .position(|byte| *byte == b';')
        .unwrap_or(line.len());
    &line[..length]
}

fn indentation_columns(line: &[u8]) -> usize {
    line.iter()
        .take_while(|byte| byte.is_ascii_whitespace())
        .fold(0_usize, |columns, byte| {
            columns.saturating_add(if *byte == b'\t' { 4 } else { 1 })
        })
}

fn trim_ascii(mut value: &[u8]) -> &[u8] {
    while value.first().is_some_and(u8::is_ascii_whitespace) {
        value = &value[1..];
    }
    while value.last().is_some_and(u8::is_ascii_whitespace) {
        value = &value[..value.len() - 1];
    }
    value
}

#[cfg(test)]
mod tests {
    use super::{ObjectIniError, ObjectIniLimits, parse_object_ini};

    #[test]
    fn extracts_multiple_default_models_and_reskins_without_behavior_execution() {
        let bytes = b"Object SyntheticBuilding\n  Behavior = SomeGameplayModule Tag\n    Value = Ignored\n  End\n  Draw = W3DModelDraw MainDraw\n    DefaultConditionState\n      Model = SyntheticHouse\n    End\n    ConditionState = DAMAGED\n      Model = SyntheticHouseDamaged\n    End\n  End\n  Draw = W3DTreeDraw Crown\n    Scale = 1.25\n    DefaultConditionState\n      Model = SyntheticCrown\n    End\n  End\nEnd\nObjectReskin SyntheticVariant SyntheticBuilding\nEnd\n";
        let parsed = parse_object_ini(bytes, ObjectIniLimits::default()).expect("object INI");
        assert_eq!(parsed.definitions().len(), 2);
        let object = &parsed.definitions()[0];
        assert_eq!(object.name_bytes(), b"SyntheticBuilding");
        assert_eq!(object.draws().len(), 2);
        assert_eq!(object.draws()[0].model_bytes(), b"SyntheticHouse");
        assert_eq!(object.draws()[1].model_bytes(), b"SyntheticCrown");
        assert_eq!(object.draws()[1].scale().to_bits(), 1.25_f32.to_bits());
        assert_eq!(
            parsed.definitions()[1].reskin_of_bytes(),
            Some(b"SyntheticBuilding".as_slice())
        );
    }

    #[test]
    fn rejects_nonfinite_or_nonpositive_draw_scale() {
        let bytes = b"Object Synthetic\n  Draw = W3DModelDraw Tag\n    Scale = NaN\nEnd\n";
        assert_eq!(
            parse_object_ini(bytes, ObjectIniLimits::default()),
            Err(ObjectIniError::InvalidScale { line: 3 })
        );
    }

    #[test]
    fn enforces_line_and_model_limits() {
        let limits = ObjectIniLimits {
            max_model_bytes: 3,
            ..ObjectIniLimits::default()
        };
        let bytes = b"Object S\n  Draw = W3DModelDraw T\n    DefaultConditionState\n      Model = Four\nEnd\n";
        assert!(matches!(
            parse_object_ini(bytes, limits),
            Err(ObjectIniError::ValueTooLong { field: "Model", .. })
        ));
    }
}
