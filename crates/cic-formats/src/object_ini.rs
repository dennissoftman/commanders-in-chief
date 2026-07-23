// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Bounded immutable decoder for renderer-facing `Object` INI model declarations.
//!
//! `Object`, `ObjectReskin`, `Draw`, `DefaultConditionState`, `Model`, and `Scale` field meanings
//! are derived from `W3DModelDraw.cpp`, `W3DModelDraw.h`, and `INI.cpp`; `W3DTreeDraw`'s flat
//! `ModelName` and `TextureName` fields are derived from `W3DTreeDraw.cpp` and `.h`. All named
//! sources are from `GeneralsGameCode` revision `9f7abb866f5afd446db14149979e744c7216baaf`,
//! licensed under GPL-3.0-or-later with Electronic Arts Section 7 terms. Full notices are recorded
//! in `docs/provenance/map.md`. The bounded non-executing extraction state machine and resource
//! limits are project-authored.

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
    pub max_texture_bytes: usize,
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
            max_texture_bytes: 1_024,
        }
    }
}

/// Presentation family selected by one supported W3D draw module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectDrawKind {
    Model,
    Tree,
}

/// One source-ordered initial model selected by a W3D draw module.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectModelDraw {
    module: Vec<u8>,
    model: Vec<u8>,
    texture: Option<Vec<u8>>,
    scale: f32,
    kind: ObjectDrawKind,
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
    pub fn texture_bytes(&self) -> Option<&[u8]> {
        self.texture.as_deref()
    }

    #[must_use]
    pub const fn scale(&self) -> f32 {
        self.scale
    }

    #[must_use]
    pub const fn kind(&self) -> ObjectDrawKind {
        self.kind
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
    diagnostics: Vec<ObjectIniDiagnostic>,
}

impl ObjectIni {
    #[must_use]
    pub fn definitions(&self) -> &[ObjectDefinition] {
        &self.definitions
    }

    /// Returns every field name inside a `Draw` module's body that this decoder does not
    /// recognize, in source order. These never fail parsing; they exist so an unsupported or
    /// missing renderer-facing field stays discoverable instead of disappearing silently. This
    /// does not cover gameplay modules other than `Draw` (`Behavior`, `Body`, and similar), which
    /// remain an intentional, documented architectural exclusion rather than a gap.
    #[must_use]
    pub fn diagnostics(&self) -> &[ObjectIniDiagnostic] {
        &self.diagnostics
    }
}

/// One field name inside a `Draw` module's body that this decoder does not specifically
/// recognize.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectIniDiagnostic {
    line: usize,
    field: Vec<u8>,
}

impl ObjectIniDiagnostic {
    /// Returns the one-based source line the field appeared on.
    #[must_use]
    pub const fn line(&self) -> usize {
        self.line
    }

    /// Returns the unrecognized field name (or bare token) exactly as spelled in the source.
    #[must_use]
    pub fn field_bytes(&self) -> &[u8] {
        &self.field
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
    module: Vec<u8>,
    model: Option<Vec<u8>>,
    texture: Option<Vec<u8>>,
    scale: f32,
    kind: ObjectDrawKind,
    in_condition_state: bool,
    select_condition_state: bool,
    saw_initial_condition_state: bool,
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
/// Object bodies contain heterogeneous modules with independent `End` tokens, and shipped draw
/// fields may use the same indentation as their `Draw` declaration. This narrow decoder therefore
/// tracks draw and condition-state `End` tokens, accepts either `DefaultConditionState` or the
/// source-equivalent first `ConditionState = NONE`, and ignores gameplay modules other than
/// `Draw` (`Behavior`, `Body`, and similar) as an intentional, documented architectural exclusion:
/// this decoder does not own gameplay/simulation semantics. Within a `Draw` module's own body,
/// field names outside the recognized renderer-facing vocabulary are not silently dropped; they
/// are retained as an [`ObjectIniDiagnostic`] on the returned value so an unsupported or missing
/// field stays discoverable. Provider overlay and reskin resolution remain caller policy.
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
    let mut diagnostics = Vec::new();
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
        if line.eq_ignore_ascii_case(b"End") && active_draw.is_some() {
            if let Some(draw) = active_draw.as_mut() {
                if draw.in_condition_state {
                    draw.in_condition_state = false;
                    draw.select_condition_state = false;
                } else {
                    finish_draw(&mut active_object, &mut active_draw);
                }
            }
            continue;
        }
        if indent == 0 && line.eq_ignore_ascii_case(b"End") {
            finish_object(
                &mut definitions,
                &mut active_object,
                line_number,
                limits.max_definitions,
            )?;
            continue;
        }
        if field_eq(line, b"Draw") {
            finish_draw(&mut active_object, &mut active_draw);
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
                    module: module.to_vec(),
                    model: None,
                    texture: None,
                    scale: 1.0,
                    kind: if module.eq_ignore_ascii_case(b"W3DTreeDraw") {
                        ObjectDrawKind::Tree
                    } else {
                        ObjectDrawKind::Model
                    },
                    in_condition_state: false,
                    select_condition_state: false,
                    saw_initial_condition_state: false,
                });
            }
            continue;
        }
        let Some(draw) = active_draw.as_mut() else {
            continue;
        };
        if token_eq(line, b"DefaultConditionState") {
            draw.in_condition_state = true;
            draw.select_condition_state = true;
            draw.saw_initial_condition_state = true;
            continue;
        }
        if field_eq(line, b"ConditionState") {
            let conditions = field_value(line).unwrap_or_default();
            draw.in_condition_state = true;
            draw.select_condition_state = !draw.saw_initial_condition_state
                && (conditions.is_empty() || conditions.eq_ignore_ascii_case(b"NONE"));
            draw.saw_initial_condition_state = true;
            continue;
        }
        if field_eq(line, b"AliasConditionState") || field_eq(line, b"TransitionState") {
            draw.in_condition_state = true;
            draw.select_condition_state = false;
            continue;
        }
        if draw.in_condition_state && draw.select_condition_state && field_eq(line, b"Model") {
            let value = field_value(line).unwrap_or_default();
            if !value.is_empty() && !value.eq_ignore_ascii_case(b"NONE") {
                enforce_value_limit(value, line_number, "Model", limits.max_model_bytes)?;
                draw.model = Some(value.to_vec());
            }
            continue;
        }
        if draw.kind == ObjectDrawKind::Tree
            && !draw.in_condition_state
            && field_eq(line, b"ModelName")
        {
            let value = field_value(line).unwrap_or_default();
            if !value.is_empty() && !value.eq_ignore_ascii_case(b"NONE") {
                enforce_value_limit(value, line_number, "ModelName", limits.max_model_bytes)?;
                draw.model = Some(value.to_vec());
            }
            continue;
        }
        if draw.kind == ObjectDrawKind::Tree
            && !draw.in_condition_state
            && field_eq(line, b"TextureName")
        {
            let value = field_value(line).unwrap_or_default();
            if !value.is_empty() && !value.eq_ignore_ascii_case(b"NONE") {
                enforce_value_limit(value, line_number, "TextureName", limits.max_texture_bytes)?;
                draw.texture = Some(value.to_vec());
            }
            continue;
        }
        if field_eq(line, b"Scale") && !draw.in_condition_state {
            let value = field_value(line).unwrap_or_default();
            let scale = std::str::from_utf8(value)
                .ok()
                .and_then(|value| value.parse::<f32>().ok())
                .filter(|value| value.is_finite() && *value > 0.0)
                .ok_or(ObjectIniError::InvalidScale { line: line_number })?;
            draw.scale = scale;
            continue;
        }
        // Nothing above matched. A known field name whose current context didn't apply it
        // (e.g. `Model` in a non-selected condition state) is not a gap and is not reported;
        // only a field name outside the known draw-body vocabulary is diagnosed.
        match field_name(line) {
            Some(field) if !is_known_draw_field(field) => {
                diagnostics.push(ObjectIniDiagnostic {
                    line: line_number,
                    field: field.to_vec(),
                });
            }
            None => {
                diagnostics.push(ObjectIniDiagnostic {
                    line: line_number,
                    field: line.to_vec(),
                });
            }
            _ => {}
        }
    }
    finish_draw(&mut active_object, &mut active_draw);
    finish_object(
        &mut definitions,
        &mut active_object,
        limits.max_lines,
        limits.max_definitions,
    )?;
    Ok(ObjectIni {
        definitions,
        diagnostics,
    })
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
            texture: draw.texture,
            scale: draw.scale,
            kind: draw.kind,
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

fn field_name(line: &[u8]) -> Option<&[u8]> {
    let equals = line.iter().position(|byte| *byte == b'=')?;
    Some(trim_ascii(&line[..equals]))
}

/// Draw-body field names this decoder specifically recognizes (whether or not the current draw
/// kind or condition-state context happens to apply them). Anything else inside a `Draw` body is
/// reported as an [`ObjectIniDiagnostic`] instead of being silently dropped.
fn is_known_draw_field(field: &[u8]) -> bool {
    [
        b"ConditionState".as_slice(),
        b"AliasConditionState".as_slice(),
        b"TransitionState".as_slice(),
        b"Model".as_slice(),
        b"ModelName".as_slice(),
        b"TextureName".as_slice(),
        b"Scale".as_slice(),
    ]
    .iter()
    .any(|candidate| field.eq_ignore_ascii_case(candidate))
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
    use super::{ObjectDrawKind, ObjectIniError, ObjectIniLimits, parse_object_ini};

    #[test]
    fn constructor_defaults_and_implicit_draw_scale_are_exact() {
        assert_eq!(
            ObjectIniLimits::default(),
            ObjectIniLimits {
                max_file_bytes: 8 * 1_024 * 1_024,
                max_lines: 250_000,
                max_line_bytes: 8_192,
                max_definitions: 32_768,
                max_draws: 131_072,
                max_name_bytes: 255,
                max_module_bytes: 255,
                max_model_bytes: 1_024,
                max_texture_bytes: 1_024,
            }
        );
        let parsed = parse_object_ini(
            b"Object Defaulted\nDraw = W3DModelDraw Tag\nDefaultConditionState\nModel = DefaultModel\nEnd\nEnd\nEnd\n",
            ObjectIniLimits::default(),
        )
        .expect("default scale");
        assert_eq!(
            parsed.definitions()[0].draws()[0].scale().to_bits(),
            1.0_f32.to_bits()
        );
    }

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
        assert_eq!(object.draws()[1].kind(), ObjectDrawKind::Tree);
        assert_eq!(object.draws()[1].scale().to_bits(), 1.25_f32.to_bits());
        assert_eq!(
            parsed.definitions()[1].reskin_of_bytes(),
            Some(b"SyntheticBuilding".as_slice())
        );
        // The `Behavior` module (and its nested `Value` field) is an intentional, documented
        // architectural exclusion — this decoder does not own gameplay modules at all, so it
        // must not be reported as an unrecognized *field* the way an unsupported Draw-body field
        // would be.
        assert!(parsed.diagnostics().is_empty());
    }

    #[test]
    fn extracts_bounded_flat_tree_resources_without_executing_topple_fields() {
        let bytes = b"\
Object SyntheticTree
 Draw = W3DTreeDraw Tag
  ModelName = SyntheticTreeModel
  TextureName = SyntheticTreeTexture.tga
  MoveOutwardTime = 3
  DoTopple = Yes
 End
End
";
        let parsed = parse_object_ini(bytes, ObjectIniLimits::default()).expect("tree draw");
        let draw = &parsed.definitions()[0].draws()[0];
        assert_eq!(draw.kind(), ObjectDrawKind::Tree);
        assert_eq!(draw.model_bytes(), b"SyntheticTreeModel");
        assert_eq!(
            draw.texture_bytes(),
            Some(b"SyntheticTreeTexture.tga".as_slice())
        );
        // MoveOutwardTime/DoTopple are real W3DTreeDraw fields this decoder doesn't apply yet;
        // they must be retained as diagnostics, not silently dropped.
        assert_eq!(parsed.diagnostics().len(), 2);
        assert_eq!(parsed.diagnostics()[0].line(), 5);
        assert_eq!(parsed.diagnostics()[0].field_bytes(), b"MoveOutwardTime");
        assert_eq!(parsed.diagnostics()[1].line(), 6);
        assert_eq!(parsed.diagnostics()[1].field_bytes(), b"DoTopple");

        let limits = ObjectIniLimits {
            max_texture_bytes: 3,
            ..ObjectIniLimits::default()
        };
        assert!(matches!(
            parse_object_ini(bytes, limits),
            Err(ObjectIniError::ValueTooLong {
                field: "TextureName",
                ..
            })
        ));
    }

    #[test]
    fn accepts_end_delimited_flat_draw_fields_and_initial_none_condition() {
        let bytes = b"Object SyntheticSupply\n  Draw = W3DSupplyDraw ModuleTag_Visual\n  Scale = 1.5\n  ConditionState = NONE\n    Model = SyntheticSupplyFull\n  End\n  ConditionState = DAMAGED\n    Model = SyntheticSupplyDamaged\n  End\n  End\nEnd\n";
        let parsed = parse_object_ini(bytes, ObjectIniLimits::default()).expect("object INI");
        let draws = parsed.definitions()[0].draws();
        assert_eq!(draws.len(), 1);
        assert_eq!(draws[0].model_bytes(), b"SyntheticSupplyFull");
        assert_eq!(draws[0].scale().to_bits(), 1.5_f32.to_bits());
    }

    #[test]
    fn condition_state_inputs_select_only_source_default_presentation() {
        let bytes = b"\
Object ExplicitDefault
 Draw = W3DModelDraw Tag
  DefaultConditionState
   Model = Explicit
  End
  ConditionState = NONE
   Model = LaterNone
  End
  AliasConditionState = DAMAGED
   Model = Alias
  End
  TransitionState = IDLE DAMAGED
   Model = Transition
  End
 End
End
Object InitialEmpty
 Draw = W3DTreeDraw Tag
  ConditionState =
   Model = EmptyConditions
  End
 End
End
Object NonDefaultFirst
 Draw = W3DSupplyDraw Tag
  ConditionState = DAMAGED
   Model = Damaged
  End
  ConditionState = NONE
   Model = TooLate
  End
 End
End
Object NoneModel
 Draw = W3DModelDraw Tag
  DefaultConditionState
   Model = NONE
  End
 End
End
Object IgnoredModule
 Draw = SomeGameplayModule Tag
  DefaultConditionState
   Model = NotRendererFacing
  End
 End
End
";
        let parsed = parse_object_ini(bytes, ObjectIniLimits::default()).expect("condition states");
        assert_eq!(parsed.definitions().len(), 5);
        assert_eq!(
            parsed.definitions()[0].draws()[0].model_bytes(),
            b"Explicit"
        );
        assert_eq!(
            parsed.definitions()[1].draws()[0].model_bytes(),
            b"EmptyConditions"
        );
        assert!(parsed.definitions()[2].draws().is_empty());
        assert!(parsed.definitions()[3].draws().is_empty());
        assert!(parsed.definitions()[4].draws().is_empty());
    }

    #[test]
    fn rejects_nonfinite_or_nonpositive_draw_scale() {
        for value in ["NaN", "inf", "-inf", "0", "-1", "not-a-number"] {
            let bytes =
                format!("Object Synthetic\n  Draw = W3DModelDraw Tag\n    Scale = {value}\nEnd\n");
            assert_eq!(
                parse_object_ini(bytes.as_bytes(), ObjectIniLimits::default()),
                Err(ObjectIniError::InvalidScale { line: 3 }),
                "{value}"
            );
        }
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

    #[test]
    fn enforces_every_object_structure_limit_before_retention() {
        let default = ObjectIniLimits::default();
        let cases = [
            (
                b"Object A\nEnd\n".as_slice(),
                ObjectIniLimits {
                    max_file_bytes: 12,
                    ..default
                },
                "file",
            ),
            (
                b"Object A\nEnd\n".as_slice(),
                ObjectIniLimits {
                    max_lines: 1,
                    ..default
                },
                "lines",
            ),
            (
                b"Object Long\nEnd\n".as_slice(),
                ObjectIniLimits {
                    max_line_bytes: 6,
                    ..default
                },
                "line bytes",
            ),
            (
                b"Object A\nEnd\n".as_slice(),
                ObjectIniLimits {
                    max_definitions: 0,
                    ..default
                },
                "definitions",
            ),
            (
                b"Object A\nDraw = W3DModelDraw Tag\nEnd\nEnd\n".as_slice(),
                ObjectIniLimits {
                    max_draws: 0,
                    ..default
                },
                "draws",
            ),
            (
                b"Object Long\nEnd\n".as_slice(),
                ObjectIniLimits {
                    max_name_bytes: 3,
                    ..default
                },
                "name",
            ),
            (
                b"Object A\nDraw = W3DModelDraw Tag\nEnd\nEnd\n".as_slice(),
                ObjectIniLimits {
                    max_module_bytes: 3,
                    ..default
                },
                "module",
            ),
        ];
        for (bytes, limits, label) in cases {
            assert!(
                parse_object_ini(bytes, limits).is_err(),
                "{label} limit unexpectedly accepted"
            );
        }
        assert_eq!(
            parse_object_ini(b"Object\n", default),
            Err(ObjectIniError::MissingObjectName { line: 1 })
        );
        assert_eq!(
            parse_object_ini(b"ObjectReskin Variant\n", default),
            Err(ObjectIniError::MissingReskinBase { line: 1 })
        );
    }
}
