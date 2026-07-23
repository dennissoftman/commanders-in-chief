// Commanders in Chief
// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only
//
// Provenance: the Terrain/Texture block shape and ordered override semantics are derived from
// Electronic Arts' GPL-3.0 source release, GeneralsGameCode revision
// 9f7abb866f5afd446db14149979e744c7216baaf, specifically INITerrain.cpp and TerrainTypes.cpp.
// This decoder is a bounded, project-authored implementation and contains no retail data.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

/// Explicit limits for the narrow Terrain INI decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerrainIniLimits {
    pub max_file_bytes: usize,
    pub max_lines: usize,
    pub max_line_bytes: usize,
    pub max_definitions: usize,
    pub max_name_bytes: usize,
    pub max_texture_bytes: usize,
}

impl Default for TerrainIniLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: 4 * 1_024 * 1_024,
            max_lines: 100_000,
            max_line_bytes: 4_096,
            max_definitions: 16_384,
            max_name_bytes: 255,
            max_texture_bytes: 1_024,
        }
    }
}

/// One ordered terrain declaration. A missing texture retains or inherits the current default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerrainDefinition {
    name: Vec<u8>,
    texture: Option<Vec<u8>>,
}

impl TerrainDefinition {
    #[must_use]
    pub fn name_bytes(&self) -> &[u8] {
        &self.name
    }

    #[must_use]
    pub fn texture_bytes(&self) -> Option<&[u8]> {
        self.texture.as_deref()
    }
}

/// A bounded, immutable sequence of Terrain INI declarations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerrainIni {
    definitions: Vec<TerrainDefinition>,
    diagnostics: Vec<TerrainIniDiagnostic>,
}

impl TerrainIni {
    #[must_use]
    pub fn definitions(&self) -> &[TerrainDefinition] {
        &self.definitions
    }

    /// Returns every field name inside a `Terrain` block that this narrow decoder does not
    /// recognize, in source order. These never fail parsing; they exist so an unsupported or
    /// missing field stays discoverable instead of disappearing silently.
    #[must_use]
    pub fn diagnostics(&self) -> &[TerrainIniDiagnostic] {
        &self.diagnostics
    }
}

/// One field name inside a `Terrain` block that this decoder does not specifically recognize.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerrainIniDiagnostic {
    line: usize,
    field: Vec<u8>,
}

impl TerrainIniDiagnostic {
    /// Returns the one-based source line the field appeared on.
    #[must_use]
    pub const fn line(&self) -> usize {
        self.line
    }

    /// Returns the unrecognized field name exactly as spelled in the source.
    #[must_use]
    pub fn field_bytes(&self) -> &[u8] {
        &self.field
    }
}

/// A structured Terrain INI decoding failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerrainIniError {
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
    TooManyDefinitions {
        line: usize,
        limit: usize,
    },
    MissingTerrainName {
        line: usize,
    },
    TerrainNameTooLong {
        line: usize,
        size: usize,
        limit: usize,
    },
    TextureMissingValue {
        line: usize,
    },
    TextureTooLong {
        line: usize,
        size: usize,
        limit: usize,
    },
    NestedTerrain {
        line: usize,
    },
    UnterminatedTerrain {
        line: usize,
    },
}

impl Display for TerrainIniError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::FileTooLarge { size, limit } => {
                write!(formatter, "Terrain INI is {size} bytes; limit is {limit}")
            }
            Self::TooManyLines { limit } => {
                write!(formatter, "Terrain INI exceeds the {limit}-line limit")
            }
            Self::LineTooLong { line, size, limit } => write!(
                formatter,
                "Terrain INI line {line} is {size} bytes; limit is {limit}"
            ),
            Self::TooManyDefinitions { line, limit } => write!(
                formatter,
                "Terrain INI exceeds the {limit}-definition limit at line {line}"
            ),
            Self::MissingTerrainName { line } => {
                write!(formatter, "Terrain INI line {line} has no terrain name")
            }
            Self::TerrainNameTooLong { line, size, limit } => write!(
                formatter,
                "Terrain name on line {line} is {size} bytes; limit is {limit}"
            ),
            Self::TextureMissingValue { line } => {
                write!(formatter, "Terrain Texture on line {line} has no value")
            }
            Self::TextureTooLong { line, size, limit } => write!(
                formatter,
                "Terrain texture on line {line} is {size} bytes; limit is {limit}"
            ),
            Self::NestedTerrain { line } => write!(
                formatter,
                "Terrain INI starts a new terrain before End at line {line}"
            ),
            Self::UnterminatedTerrain { line } => write!(
                formatter,
                "Terrain INI terrain opened on line {line} has no End"
            ),
        }
    }
}

impl Error for TerrainIniError {}

/// Decodes the Terrain blocks needed to resolve MAP texture classes.
///
/// A `Terrain` block's simulation-only fields (for example `Class`) are outside the
/// renderer-facing texture catalog this decoder builds, so they are not applied to
/// [`TerrainDefinition`] — but they are never silently discarded either. Every field name this
/// decoder does not specifically recognize is retained as a [`TerrainIniDiagnostic`] so an
/// unsupported or genuinely missing field stays discoverable instead of disappearing silently.
///
/// # Errors
///
/// Returns a structured error when input or any count/string exceeds its explicit limit, or when
/// Terrain block structure is malformed.
pub fn parse_terrain_ini(
    bytes: &[u8],
    limits: TerrainIniLimits,
) -> Result<TerrainIni, TerrainIniError> {
    if bytes.len() > limits.max_file_bytes {
        return Err(TerrainIniError::FileTooLarge {
            size: bytes.len(),
            limit: limits.max_file_bytes,
        });
    }

    let mut definitions = Vec::new();
    let mut diagnostics = Vec::new();
    let mut active: Option<(usize, Vec<u8>, Option<Vec<u8>>)> = None;
    for (zero_based_line, raw_line) in bytes.split(|byte| *byte == b'\n').enumerate() {
        let line_number = zero_based_line
            .checked_add(1)
            .ok_or(TerrainIniError::TooManyLines {
                limit: limits.max_lines,
            })?;
        if line_number > limits.max_lines {
            return Err(TerrainIniError::TooManyLines {
                limit: limits.max_lines,
            });
        }
        if raw_line.len() > limits.max_line_bytes {
            return Err(TerrainIniError::LineTooLong {
                line: line_number,
                size: raw_line.len(),
                limit: limits.max_line_bytes,
            });
        }
        let line = trim_ascii(strip_comment(raw_line));
        if line.is_empty() {
            continue;
        }

        if line.eq_ignore_ascii_case(b"Terrain") {
            return Err(TerrainIniError::MissingTerrainName { line: line_number });
        }
        if token_eq(line, b"Terrain") {
            if active.is_some() {
                return Err(TerrainIniError::NestedTerrain { line: line_number });
            }
            let name = trim_ascii(&line[b"Terrain".len()..]);
            if name.is_empty() {
                return Err(TerrainIniError::MissingTerrainName { line: line_number });
            }
            if name.len() > limits.max_name_bytes {
                return Err(TerrainIniError::TerrainNameTooLong {
                    line: line_number,
                    size: name.len(),
                    limit: limits.max_name_bytes,
                });
            }
            active = Some((line_number, name.to_vec(), None));
            continue;
        }

        let Some((_, _, texture)) = active.as_mut() else {
            continue;
        };
        if line.eq_ignore_ascii_case(b"End") {
            let Some((_, name, texture)) = active.take() else {
                continue;
            };
            if definitions.len() >= limits.max_definitions {
                return Err(TerrainIniError::TooManyDefinitions {
                    line: line_number,
                    limit: limits.max_definitions,
                });
            }
            definitions.push(TerrainDefinition { name, texture });
            continue;
        }
        let Some((field, value)) = split_assignment(line) else {
            continue;
        };
        if field.eq_ignore_ascii_case(b"Texture") {
            let value = unquote(trim_ascii(value));
            if value.is_empty() {
                return Err(TerrainIniError::TextureMissingValue { line: line_number });
            }
            if value.len() > limits.max_texture_bytes {
                return Err(TerrainIniError::TextureTooLong {
                    line: line_number,
                    size: value.len(),
                    limit: limits.max_texture_bytes,
                });
            }
            *texture = Some(value.to_vec());
        } else {
            diagnostics.push(TerrainIniDiagnostic {
                line: line_number,
                field: field.to_vec(),
            });
        }
    }

    if let Some((line, _, _)) = active {
        return Err(TerrainIniError::UnterminatedTerrain { line });
    }
    Ok(TerrainIni {
        definitions,
        diagnostics,
    })
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

fn token_eq(line: &[u8], token: &[u8]) -> bool {
    line.get(..token.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(token))
        && line.get(token.len()).is_some_and(u8::is_ascii_whitespace)
}

fn split_assignment(line: &[u8]) -> Option<(&[u8], &[u8])> {
    let equals = line.iter().position(|byte| *byte == b'=')?;
    Some((trim_ascii(&line[..equals]), trim_ascii(&line[equals + 1..])))
}

fn unquote(value: &[u8]) -> &[u8] {
    if value.len() >= 2 && value.first() == Some(&b'"') && value.last() == Some(&b'"') {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::{TerrainIniError, TerrainIniLimits, parse_terrain_ini};

    #[test]
    fn decodes_ordered_texture_declarations_and_diagnoses_other_fields() {
        let ini = parse_terrain_ini(
            b"; synthetic fixture\r\nTerrain DefaultTerrain\r\n  Texture = Base.tga\r\n  Class = ROCK\r\nEnd\r\n\r\nTerrain Cliff\n  BlendEdges = Yes\nEnd\nTerrain Cliff\n  Texture = \"Override.dds\"\nEnd\n",
            TerrainIniLimits::default(),
        )
        .expect("decode terrain INI");
        assert_eq!(ini.definitions().len(), 3);
        assert_eq!(ini.definitions()[0].name_bytes(), b"DefaultTerrain");
        assert_eq!(
            ini.definitions()[0].texture_bytes(),
            Some(b"Base.tga".as_slice())
        );
        assert_eq!(ini.definitions()[1].name_bytes(), b"Cliff");
        assert_eq!(ini.definitions()[1].texture_bytes(), None);
        assert_eq!(
            ini.definitions()[2].texture_bytes(),
            Some(b"Override.dds".as_slice())
        );

        assert_eq!(ini.diagnostics().len(), 2);
        assert_eq!(ini.diagnostics()[0].line(), 4);
        assert_eq!(ini.diagnostics()[0].field_bytes(), b"Class");
        assert_eq!(ini.diagnostics()[1].line(), 8);
        assert_eq!(ini.diagnostics()[1].field_bytes(), b"BlendEdges");
    }

    #[test]
    fn rejects_unterminated_and_oversized_input() {
        assert_eq!(
            parse_terrain_ini(b"Terrain Broken\n", TerrainIniLimits::default()),
            Err(TerrainIniError::UnterminatedTerrain { line: 1 })
        );
        let limits = TerrainIniLimits {
            max_file_bytes: 3,
            ..TerrainIniLimits::default()
        };
        assert_eq!(
            parse_terrain_ini(b"four", limits),
            Err(TerrainIniError::FileTooLarge { size: 4, limit: 3 })
        );
    }
}
