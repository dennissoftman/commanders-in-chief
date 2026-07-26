// Commanders in Chief
// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only
//
// Provenance: the `HeaderTemplate` block shape, its three fields, and the duplicate-name override
// behavior are derived from Electronic Arts' GPL-3.0 source release, GeneralsGameCode revision
// 9f7abb866f5afd446db14149979e744c7216baaf, specifically
// `Core/GameEngine/Source/GameClient/GUI/HeaderTemplate.cpp`
// (`HeaderTemplateManager::m_headerFieldParseTable`, `INI::parseHeaderTemplateDefinition`,
// `HeaderTemplate::HeaderTemplate`, `HeaderTemplateManager::init`) and
// `Core/GameEngine/Include/GameClient/HeaderTemplate.h`. This decoder is a bounded,
// project-authored implementation and contains no retail data.

use crate::ui_ini::{
    LineReader, Separators, Tokens, UiIniDiagnostic, UiIniDiagnosticKind, UiIniError, UiIniFormat,
    UiIniLimits, diagnostic_text, is_end_token, scan_bool, scan_int,
};

/// One immutable named header presentation template.
///
/// A WND control names a template through its `HEADERTEMPLATE` record; the template supplies the
/// font family, point size, and weight so localized layouts share one look without repeating font
/// records per control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderTemplate {
    name: Vec<u8>,
    line: usize,
    font_name: Vec<u8>,
    point: i32,
    bold: bool,
}

impl HeaderTemplate {
    /// Returns the template name exactly as spelled.
    ///
    /// The source compares template names with `AsciiString::compare`, so lookup is
    /// case-sensitive, unlike mapped images.
    #[must_use]
    pub fn name_bytes(&self) -> &[u8] {
        &self.name
    }

    /// Returns the one-based line the template opened on.
    #[must_use]
    pub const fn line(&self) -> usize {
        self.line
    }

    /// Returns the font family name, empty when the template declares none.
    #[must_use]
    pub fn font_name_bytes(&self) -> &[u8] {
        &self.font_name
    }

    /// Returns the point size, zero when the template declares none.
    #[must_use]
    pub const fn point(&self) -> i32 {
        self.point
    }

    /// Returns whether the template requests a bold face.
    #[must_use]
    pub const fn bold(&self) -> bool {
        self.bold
    }
}

/// A bounded, immutable set of header templates from one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderTemplateIni {
    templates: Vec<HeaderTemplate>,
    diagnostics: Vec<UiIniDiagnostic>,
}

impl HeaderTemplateIni {
    /// Returns every template in source order.
    #[must_use]
    pub fn templates(&self) -> &[HeaderTemplate] {
        &self.templates
    }

    /// Returns the template with this exact name.
    #[must_use]
    pub fn find(&self, name: &[u8]) -> Option<&HeaderTemplate> {
        self.templates.iter().find(|template| template.name == name)
    }

    /// Returns every non-fatal observation in encounter order.
    #[must_use]
    pub fn diagnostics(&self) -> &[UiIniDiagnostic] {
        &self.diagnostics
    }
}

const FORMAT: UiIniFormat = UiIniFormat::HeaderTemplate;

/// Decodes one `Data/<Language>/HeaderTemplate.ini` into immutable header templates.
///
/// # Errors
///
/// Returns a structured error when the input or any count, name, or value exceeds its explicit
/// limit, when a block opens without a name, or when a block is never closed by `End`.
pub fn parse_header_template_ini(
    bytes: &[u8],
    limits: UiIniLimits,
) -> Result<HeaderTemplateIni, UiIniError> {
    let mut reader = LineReader::new(bytes, FORMAT, limits)?;
    let mut templates: Vec<HeaderTemplate> = Vec::new();
    let mut diagnostics = Vec::new();

    while let Some((line, text)) = reader.next_line()? {
        let mut tokens = Tokens::new(text);
        let Some(keyword) = tokens.next(Separators::Default) else {
            continue;
        };
        if keyword != b"HeaderTemplate" {
            diagnostics.push(UiIniDiagnostic::new(
                line,
                UiIniDiagnosticKind::UnknownBlock {
                    keyword: diagnostic_text(keyword),
                },
            ));
            skip_block(&mut reader, line)?;
            continue;
        }
        let Some(name) = tokens.next(Separators::Default) else {
            return Err(UiIniError::MissingBlockName {
                format: FORMAT,
                line,
            });
        };
        if name.len() > limits.max_name_bytes {
            return Err(UiIniError::NameTooLong {
                format: FORMAT,
                line,
                size: name.len(),
                limit: limits.max_name_bytes,
            });
        }
        let existing = templates.iter().position(|template| template.name == name);
        if let Some(index) = existing {
            diagnostics.push(UiIniDiagnostic::new(
                line,
                UiIniDiagnosticKind::DuplicateDefinition {
                    name: diagnostic_text(name),
                    first_line: templates[index].line,
                },
            ));
        } else if templates.len() >= limits.max_definitions {
            return Err(UiIniError::TooManyDefinitions {
                format: FORMAT,
                line,
                limit: limits.max_definitions,
            });
        }
        let index = if let Some(index) = existing {
            index
        } else {
            templates.push(HeaderTemplate {
                name: name.to_vec(),
                line,
                font_name: Vec::new(),
                point: 0,
                bold: false,
            });
            templates.len() - 1
        };
        decode_block(
            &mut reader,
            &mut templates[index],
            &mut diagnostics,
            line,
            limits,
        )?;
    }

    Ok(HeaderTemplateIni {
        templates,
        diagnostics,
    })
}

fn decode_block(
    reader: &mut LineReader<'_>,
    template: &mut HeaderTemplate,
    diagnostics: &mut Vec<UiIniDiagnostic>,
    opened: usize,
    limits: UiIniLimits,
) -> Result<(), UiIniError> {
    loop {
        let Some((line, text)) = reader.next_line()? else {
            return Err(UiIniError::UnterminatedBlock {
                format: FORMAT,
                line: opened,
            });
        };
        let mut tokens = Tokens::new(text);
        let Some(field) = tokens.next(Separators::Default) else {
            continue;
        };
        if is_end_token(field) {
            return Ok(());
        }
        match field {
            b"Font" => {
                let Some(value) = tokens.next_quoted_string() else {
                    push_malformed(diagnostics, line, field, "expected a quoted font family");
                    continue;
                };
                if value.len() > limits.max_value_bytes {
                    return Err(UiIniError::ValueTooLong {
                        format: FORMAT,
                        line,
                        field: diagnostic_text(field),
                        size: value.len(),
                        limit: limits.max_value_bytes,
                    });
                }
                template.font_name = value;
            }
            b"Point" => match tokens.next(Separators::Default).and_then(scan_int) {
                Some(value) => template.point = value,
                None => push_malformed(diagnostics, line, field, "expected an integer"),
            },
            b"Bold" => match tokens.next(Separators::Default).and_then(scan_bool) {
                Some(value) => template.bold = value,
                None => push_malformed(diagnostics, line, field, "expected Yes or No"),
            },
            _ => diagnostics.push(UiIniDiagnostic::new(
                line,
                UiIniDiagnosticKind::UnknownField {
                    field: diagnostic_text(field),
                },
            )),
        }
    }
}

fn push_malformed(diagnostics: &mut Vec<UiIniDiagnostic>, line: usize, field: &[u8], reason: &str) {
    diagnostics.push(UiIniDiagnostic::new(
        line,
        UiIniDiagnosticKind::MalformedField {
            field: diagnostic_text(field),
            reason: reason.to_owned().into_boxed_str(),
        },
    ));
}

fn skip_block(reader: &mut LineReader<'_>, opened: usize) -> Result<(), UiIniError> {
    loop {
        let Some((_, text)) = reader.next_line()? else {
            return Err(UiIniError::UnterminatedBlock {
                format: FORMAT,
                line: opened,
            });
        };
        let mut tokens = Tokens::new(text);
        if tokens.next(Separators::Default).is_some_and(is_end_token) {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_header_template_ini;
    use crate::ui_ini::{UiIniDiagnosticKind, UiIniError, UiIniFormat, UiIniLimits};

    #[test]
    fn decodes_named_templates_with_quoted_families() {
        let ini = parse_header_template_ini(
            b"; synthetic fixture\n\
              HeaderTemplate SynthTitle\n\
                Font = \"Synth Sans\"\n\
                Point = 14\n\
                Bold = Yes\n\
              End\n\
              HeaderTemplate SynthBody\n\
                Font = SynthMono\n\
                Point = 10\n\
                Bold = No\n\
              End\n",
            UiIniLimits::default(),
        )
        .expect("decode header templates");
        assert_eq!(ini.templates().len(), 2);
        assert_eq!(ini.templates()[0].name_bytes(), b"SynthTitle");
        assert_eq!(ini.templates()[0].font_name_bytes(), b"Synth Sans");
        assert_eq!(ini.templates()[0].point(), 14);
        assert!(ini.templates()[0].bold());
        assert_eq!(ini.templates()[1].font_name_bytes(), b"SynthMono");
        assert!(!ini.templates()[1].bold());
        assert!(ini.diagnostics().is_empty());

        // Template lookup is case-sensitive, matching `HeaderTemplateManager::findHeaderTemplate`.
        assert!(ini.find(b"SynthTitle").is_some());
        assert!(ini.find(b"synthtitle").is_none());
    }

    #[test]
    fn defaults_and_diagnostics_match_the_source_constructor() {
        let ini = parse_header_template_ini(
            b"HeaderTemplate Bare\nEnd\n\
              HeaderTemplate Odd\n  Weight = Heavy\n  Point = wide\nEnd\n",
            UiIniLimits::default(),
        )
        .expect("decode sparse templates");
        assert_eq!(ini.templates()[0].font_name_bytes(), b"");
        assert_eq!(ini.templates()[0].point(), 0);
        assert!(!ini.templates()[0].bold());
        assert_eq!(
            ini.diagnostics()[0].kind(),
            &UiIniDiagnosticKind::UnknownField {
                field: "Weight".to_owned().into_boxed_str(),
            }
        );
        assert!(matches!(
            ini.diagnostics()[1].kind(),
            UiIniDiagnosticKind::MalformedField { field, .. } if &**field == "Point"
        ));
    }

    #[test]
    fn a_duplicate_name_overwrites_and_reports() {
        let ini = parse_header_template_ini(
            b"HeaderTemplate Synth\n  Font = \"A\"\n  Point = 8\nEnd\n\
              HeaderTemplate Synth\n  Point = 12\nEnd\n",
            UiIniLimits::default(),
        )
        .expect("decode duplicates");
        assert_eq!(ini.templates().len(), 1);
        assert_eq!(ini.templates()[0].font_name_bytes(), b"A");
        assert_eq!(ini.templates()[0].point(), 12);
        assert_eq!(
            ini.diagnostics()[0].kind(),
            &UiIniDiagnosticKind::DuplicateDefinition {
                name: "Synth".to_owned().into_boxed_str(),
                first_line: 1,
            }
        );
    }

    #[test]
    fn rejects_structural_failures_and_truncation_never_panics() {
        assert_eq!(
            parse_header_template_ini(b"HeaderTemplate Synth\n", UiIniLimits::default()),
            Err(UiIniError::UnterminatedBlock {
                format: UiIniFormat::HeaderTemplate,
                line: 1,
            })
        );
        assert_eq!(
            parse_header_template_ini(b"HeaderTemplate\nEnd\n", UiIniLimits::default()),
            Err(UiIniError::MissingBlockName {
                format: UiIniFormat::HeaderTemplate,
                line: 1,
            })
        );
        let complete =
            b"HeaderTemplate Synth\n  Font = \"Synth Sans\"\n  Point = 14\n  Bold = Yes\nEnd\n";
        for length in 0..=complete.len() {
            let _ = parse_header_template_ini(&complete[..length], UiIniLimits::default());
        }
    }
}
