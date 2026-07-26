// Commanders in Chief
// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only
//
// Provenance: the `Language` block, its 25 field names, the font-descriptor shape, the repeated
// `LocalFontFile` list order, the resolution font-size policy vocabulary, and every constructor
// default are derived from Electronic Arts' GPL-3.0 source release, GeneralsGameCode revision
// 9f7abb866f5afd446db14149979e744c7216baaf, specifically
// `Core/GameEngine/Source/GameClient/GlobalLanguage.cpp`
// (`TheGlobalLanguageDataFieldParseTable`, `ResolutionFontSizeMethodNames`,
// `INI::parseLanguageDefinition`, `GlobalLanguage::GlobalLanguage`, `GlobalLanguage::parseFontDesc`,
// `GlobalLanguage::parseFontFileName`, `GlobalLanguage::init`),
// `Core/GameEngine/Include/GameClient/GlobalLanguage.h` (`ResolutionFontSizeMethod`), and
// `Generals/Code/GameEngine/Include/GameClient/FontDesc.h` with
// `GlobalLanguage.cpp`'s `FontDesc::FontDesc`. This decoder is a bounded, project-authored
// implementation and contains no retail data.

use crate::ui_ini::{
    LineReader, Separators, Tokens, UiIniDiagnostic, UiIniDiagnosticKind, UiIniError, UiIniFormat,
    UiIniLimits, diagnostic_text, index_of_name, is_end_token, scan_bool, scan_int, scan_real,
};

/// How a font's point size follows the presentation resolution.
///
/// `CLASSIC` is the original behavior. The other three are policies the pinned source release adds;
/// they are decoded because a modded or updated `Language.ini` may select them, and the declared
/// default at that revision is `CLASSIC_NO_CEILING`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionFontSizeMethod {
    /// Scale by width against the 800x600 reference, clamped at 2x.
    Classic,
    /// Scale by width against the 800x600 reference with no clamp.
    ClassicNoCeiling,
    /// Scale by the smaller of the width and height ratios.
    Strict,
    /// Scale by the evenly weighted width and height ratios, with the aspect clamped.
    Balanced,
}

impl ResolutionFontSizeMethod {
    /// The vocabulary in declaration order, which is also the stored index order.
    pub const NAMES: [&'static str; 4] = ["CLASSIC", "CLASSIC_NO_CEILING", "STRICT", "BALANCED"];

    /// Returns the established spelling of this policy.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Classic => "CLASSIC",
            Self::ClassicNoCeiling => "CLASSIC_NO_CEILING",
            Self::Strict => "STRICT",
            Self::Balanced => "BALANCED",
        }
    }

    const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Classic),
            1 => Some(Self::ClassicNoCeiling),
            2 => Some(Self::Strict),
            3 => Some(Self::Balanced),
            _ => None,
        }
    }
}

impl Default for ResolutionFontSizeMethod {
    /// `ResolutionFontSizeMethod_Default` at the pinned revision.
    fn default() -> Self {
        Self::ClassicNoCeiling
    }
}

/// Every font role a `Language.ini` can describe, in source table order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguageFontRole {
    /// `CopyrightFont`.
    Copyright,
    /// `MessageFont`.
    Message,
    /// `MilitaryCaptionTitleFont`.
    MilitaryCaptionTitle,
    /// `MilitaryCaptionFont`.
    MilitaryCaption,
    /// `SuperweaponCountdownNormalFont`.
    SuperweaponCountdownNormal,
    /// `SuperweaponCountdownReadyFont`.
    SuperweaponCountdownReady,
    /// `NamedTimerCountdownNormalFont`.
    NamedTimerCountdownNormal,
    /// `NamedTimerCountdownReadyFont`.
    NamedTimerCountdownReady,
    /// `DrawableCaptionFont`.
    DrawableCaption,
    /// `DefaultWindowFont`, the fallback for a WND control with no `FONT` record.
    DefaultWindow,
    /// `DefaultDisplayStringFont`.
    DefaultDisplayString,
    /// `TooltipFontName`.
    Tooltip,
    /// `NativeDebugDisplay`.
    NativeDebugDisplay,
    /// `DrawGroupInfoFont`.
    DrawGroupInfo,
    /// `CreditsTitleFont`.
    CreditsTitle,
    /// `CreditsMinorTitleFont`.
    CreditsMinorTitle,
    /// `CreditsNormalFont`.
    CreditsNormal,
}

/// The number of established font roles.
pub const LANGUAGE_FONT_ROLES: usize = 17;

impl LanguageFontRole {
    /// Every role in source table order.
    pub const ALL: [Self; LANGUAGE_FONT_ROLES] = [
        Self::Copyright,
        Self::Message,
        Self::MilitaryCaptionTitle,
        Self::MilitaryCaption,
        Self::SuperweaponCountdownNormal,
        Self::SuperweaponCountdownReady,
        Self::NamedTimerCountdownNormal,
        Self::NamedTimerCountdownReady,
        Self::DrawableCaption,
        Self::DefaultWindow,
        Self::DefaultDisplayString,
        Self::Tooltip,
        Self::NativeDebugDisplay,
        Self::DrawGroupInfo,
        Self::CreditsTitle,
        Self::CreditsMinorTitle,
        Self::CreditsNormal,
    ];

    /// Returns the INI field name that declares this role.
    #[must_use]
    pub const fn field_name(self) -> &'static str {
        match self {
            Self::Copyright => "CopyrightFont",
            Self::Message => "MessageFont",
            Self::MilitaryCaptionTitle => "MilitaryCaptionTitleFont",
            Self::MilitaryCaption => "MilitaryCaptionFont",
            Self::SuperweaponCountdownNormal => "SuperweaponCountdownNormalFont",
            Self::SuperweaponCountdownReady => "SuperweaponCountdownReadyFont",
            Self::NamedTimerCountdownNormal => "NamedTimerCountdownNormalFont",
            Self::NamedTimerCountdownReady => "NamedTimerCountdownReadyFont",
            Self::DrawableCaption => "DrawableCaptionFont",
            Self::DefaultWindow => "DefaultWindowFont",
            Self::DefaultDisplayString => "DefaultDisplayStringFont",
            Self::Tooltip => "TooltipFontName",
            Self::NativeDebugDisplay => "NativeDebugDisplay",
            Self::DrawGroupInfo => "DrawGroupInfoFont",
            Self::CreditsTitle => "CreditsTitleFont",
            Self::CreditsMinorTitle => "CreditsMinorTitleFont",
            Self::CreditsNormal => "CreditsNormalFont",
        }
    }

    const fn index(self) -> usize {
        match self {
            Self::Copyright => 0,
            Self::Message => 1,
            Self::MilitaryCaptionTitle => 2,
            Self::MilitaryCaption => 3,
            Self::SuperweaponCountdownNormal => 4,
            Self::SuperweaponCountdownReady => 5,
            Self::NamedTimerCountdownNormal => 6,
            Self::NamedTimerCountdownReady => 7,
            Self::DrawableCaption => 8,
            Self::DefaultWindow => 9,
            Self::DefaultDisplayString => 10,
            Self::Tooltip => 11,
            Self::NativeDebugDisplay => 12,
            Self::DrawGroupInfo => 13,
            Self::CreditsTitle => 14,
            Self::CreditsMinorTitle => 15,
            Self::CreditsNormal => 16,
        }
    }

    fn from_field(field: &[u8]) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|role| field == role.field_name().as_bytes())
    }
}

/// One `"family" size bold` font description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageFontDesc {
    name: Vec<u8>,
    size: i32,
    bold: bool,
    declared: bool,
}

impl LanguageFontDesc {
    /// Returns the font family name.
    #[must_use]
    pub fn name_bytes(&self) -> &[u8] {
        &self.name
    }

    /// Returns the point size.
    #[must_use]
    pub const fn size(&self) -> i32 {
        self.size
    }

    /// Returns whether a bold face is requested.
    #[must_use]
    pub const fn bold(&self) -> bool {
        self.bold
    }

    /// Returns whether the file declared this role, as opposed to leaving the source's
    /// `FontDesc` constructor default of `Arial Unicode MS` at 12 points.
    #[must_use]
    pub const fn is_declared(&self) -> bool {
        self.declared
    }
}

impl Default for LanguageFontDesc {
    /// `FontDesc::FontDesc` at the pinned revision.
    fn default() -> Self {
        Self {
            name: b"Arial Unicode MS".to_vec(),
            size: 12,
            bold: false,
            declared: false,
        }
    }
}

/// The immutable text presentation policy from one `Data/<Language>/Language.ini`.
///
/// This is the localized side of UI resource resolution: it names the font families a layout's
/// `FONT` and `HEADERTEMPLATE` records resolve against, and fixes the resolution font-scaling
/// policy the presentation gates apply.
#[derive(Debug, Clone, PartialEq)]
pub struct LanguageIni {
    unicode_font_name: Vec<u8>,
    local_font_files: Vec<Vec<u8>>,
    military_caption_speed: i32,
    military_caption_delay_ms: i32,
    use_hard_word_wrap: bool,
    resolution_font_adjustment: f32,
    resolution_font_size_method: ResolutionFontSizeMethod,
    fonts: Vec<LanguageFontDesc>,
    declared: bool,
    diagnostics: Vec<UiIniDiagnostic>,
}

impl LanguageIni {
    /// Returns the Unicode font family name, empty when undeclared.
    #[must_use]
    pub fn unicode_font_name_bytes(&self) -> &[u8] {
        &self.unicode_font_name
    }

    /// Returns every `LocalFontFile` in application order.
    ///
    /// The source pushes each name onto the front of its list, so the last file the INI declares is
    /// registered first. This preserves that order rather than file order, because a font file
    /// installed earlier wins when two supply the same family.
    #[must_use]
    pub fn local_font_files(&self) -> &[Vec<u8>] {
        &self.local_font_files
    }

    /// Returns the military caption reveal speed.
    #[must_use]
    pub const fn military_caption_speed(&self) -> i32 {
        self.military_caption_speed
    }

    /// Returns the military caption delay in milliseconds; the source default is 750.
    #[must_use]
    pub const fn military_caption_delay_ms(&self) -> i32 {
        self.military_caption_delay_ms
    }

    /// Returns whether hard word wrapping is requested.
    #[must_use]
    pub const fn use_hard_word_wrap(&self) -> bool {
        self.use_hard_word_wrap
    }

    /// Returns the resolution font-size scaler; the source default is 0.7, so a font grows at 70%
    /// of the resolution increase.
    #[must_use]
    pub const fn resolution_font_adjustment(&self) -> f32 {
        self.resolution_font_adjustment
    }

    /// Returns the selected resolution font-size policy.
    #[must_use]
    pub const fn resolution_font_size_method(&self) -> ResolutionFontSizeMethod {
        self.resolution_font_size_method
    }

    /// Returns one role's font description, declared or defaulted.
    #[must_use]
    pub fn font(&self, role: LanguageFontRole) -> &LanguageFontDesc {
        &self.fonts[role.index()]
    }

    /// Returns whether the file actually contained a `Language` block.
    #[must_use]
    pub const fn is_declared(&self) -> bool {
        self.declared
    }

    /// Returns every non-fatal observation in encounter order.
    #[must_use]
    pub fn diagnostics(&self) -> &[UiIniDiagnostic] {
        &self.diagnostics
    }
}

impl Default for LanguageIni {
    /// Every value the source's `GlobalLanguage` constructor establishes before any file is read.
    fn default() -> Self {
        Self {
            unicode_font_name: Vec::new(),
            local_font_files: Vec::new(),
            military_caption_speed: 0,
            military_caption_delay_ms: 750,
            use_hard_word_wrap: false,
            resolution_font_adjustment: 0.7,
            resolution_font_size_method: ResolutionFontSizeMethod::default(),
            fonts: vec![LanguageFontDesc::default(); LANGUAGE_FONT_ROLES],
            declared: false,
            diagnostics: Vec::new(),
        }
    }
}

const FORMAT: UiIniFormat = UiIniFormat::Language;

/// Decodes one `Data/<Language>/Language.ini` into the immutable text presentation policy.
///
/// The `Language` block is unnamed because the source parses it into a singleton. A file with no
/// such block yields the constructor defaults with [`LanguageIni::is_declared`] false, which is how
/// a missing localization archive stays visible instead of silently becoming defaults.
///
/// # Errors
///
/// Returns a structured error when the input or any count or value exceeds its explicit limit, or
/// when a block is never closed by `End`.
pub fn parse_language_ini(bytes: &[u8], limits: UiIniLimits) -> Result<LanguageIni, UiIniError> {
    let mut reader = LineReader::new(bytes, FORMAT, limits)?;
    let mut language = LanguageIni::default();

    while let Some((line, text)) = reader.next_line()? {
        let mut tokens = Tokens::new(text);
        let Some(keyword) = tokens.next(Separators::Default) else {
            continue;
        };
        if keyword != b"Language" {
            language.diagnostics.push(UiIniDiagnostic::new(
                line,
                UiIniDiagnosticKind::UnknownBlock {
                    keyword: diagnostic_text(keyword),
                },
            ));
            skip_block(&mut reader, line)?;
            continue;
        }
        if language.declared {
            language.diagnostics.push(UiIniDiagnostic::new(
                line,
                UiIniDiagnosticKind::DuplicateDefinition {
                    name: "Language".to_owned().into_boxed_str(),
                    first_line: 0,
                },
            ));
        }
        language.declared = true;
        decode_block(&mut reader, &mut language, line, limits)?;
    }

    Ok(language)
}

fn decode_block(
    reader: &mut LineReader<'_>,
    language: &mut LanguageIni,
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
        if let Some(role) = LanguageFontRole::from_field(field) {
            match decode_font_desc(&mut tokens) {
                Some(font) => {
                    check_value_length(field, &font.name, line, limits)?;
                    language.fonts[role.index()] = font;
                }
                None => push_malformed(
                    language,
                    line,
                    field,
                    "expected \"<family>\" <point size> <Yes|No>",
                ),
            }
            continue;
        }
        match field {
            b"UnicodeFontName" => {
                let Some(value) = tokens.next_ascii_string() else {
                    push_malformed(language, line, field, "expected a font family");
                    continue;
                };
                check_value_length(field, &value, line, limits)?;
                language.unicode_font_name = value;
            }
            b"LocalFontFile" => {
                let Some(value) = tokens.next_ascii_string() else {
                    push_malformed(language, line, field, "expected a font file name");
                    continue;
                };
                check_value_length(field, &value, line, limits)?;
                if language.local_font_files.len() >= limits.max_list_entries {
                    return Err(UiIniError::TooManyListEntries {
                        format: FORMAT,
                        line,
                        field: diagnostic_text(field),
                        limit: limits.max_list_entries,
                    });
                }
                // `parseFontFileName` pushes onto the front of the list.
                language.local_font_files.insert(0, value);
            }
            b"MilitaryCaptionSpeed" => match tokens.next(Separators::Default).and_then(scan_int) {
                Some(value) => language.military_caption_speed = value,
                None => push_malformed(language, line, field, "expected an integer"),
            },
            b"MilitaryCaptionDelayMS" => {
                match tokens.next(Separators::Default).and_then(scan_int) {
                    Some(value) => language.military_caption_delay_ms = value,
                    None => push_malformed(language, line, field, "expected an integer"),
                }
            }
            b"UseHardWordWrap" => match tokens.next(Separators::Default).and_then(scan_bool) {
                Some(value) => language.use_hard_word_wrap = value,
                None => push_malformed(language, line, field, "expected Yes or No"),
            },
            b"ResolutionFontAdjustment" => {
                match tokens.next(Separators::Default).and_then(scan_real) {
                    Some(value) => language.resolution_font_adjustment = value,
                    None => push_malformed(language, line, field, "expected a real"),
                }
            }
            b"ResolutionFontSizeMethod" => {
                let method = tokens
                    .next(Separators::Default)
                    .and_then(|token| index_of_name(token, &ResolutionFontSizeMethod::NAMES))
                    .and_then(ResolutionFontSizeMethod::from_index);
                match method {
                    Some(value) => language.resolution_font_size_method = value,
                    None => push_malformed(
                        language,
                        line,
                        field,
                        "expected CLASSIC, CLASSIC_NO_CEILING, STRICT, or BALANCED",
                    ),
                }
            }
            _ => language.diagnostics.push(UiIniDiagnostic::new(
                line,
                UiIniDiagnosticKind::UnknownField {
                    field: diagnostic_text(field),
                },
            )),
        }
    }
}

fn decode_font_desc(tokens: &mut Tokens<'_>) -> Option<LanguageFontDesc> {
    let name = tokens.next_quoted_string()?;
    let size = scan_int(tokens.next(Separators::Default)?)?;
    let bold = scan_bool(tokens.next(Separators::Default)?)?;
    Some(LanguageFontDesc {
        name,
        size,
        bold,
        declared: true,
    })
}

fn check_value_length(
    field: &[u8],
    value: &[u8],
    line: usize,
    limits: UiIniLimits,
) -> Result<(), UiIniError> {
    if value.len() > limits.max_value_bytes {
        return Err(UiIniError::ValueTooLong {
            format: FORMAT,
            line,
            field: diagnostic_text(field),
            size: value.len(),
            limit: limits.max_value_bytes,
        });
    }
    Ok(())
}

fn push_malformed(language: &mut LanguageIni, line: usize, field: &[u8], reason: &str) {
    language.diagnostics.push(UiIniDiagnostic::new(
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
    use super::{LanguageFontRole, LanguageIni, ResolutionFontSizeMethod, parse_language_ini};
    use crate::ui_ini::{UiIniDiagnosticKind, UiIniError, UiIniFormat, UiIniLimits};

    #[test]
    fn decodes_font_roles_policy_and_font_file_order() {
        let ini = parse_language_ini(
            b"; synthetic fixture\n\
              Language\n\
                UnicodeFontName = \"Synth Unicode\"\n\
                LocalFontFile = first.ttf\n\
                LocalFontFile = second.ttf\n\
                MilitaryCaptionSpeed = 3\n\
                MilitaryCaptionDelayMS = 500\n\
                UseHardWordWrap = Yes\n\
                ResolutionFontAdjustment = 0.5\n\
                ResolutionFontSizeMethod = STRICT\n\
                DefaultWindowFont = \"Synth Sans\" 11 No\n\
                CopyrightFont = SynthMono 9 Yes\n\
              End\n",
            UiIniLimits::default(),
        )
        .expect("decode language");
        assert!(ini.is_declared());
        assert_eq!(ini.unicode_font_name_bytes(), b"Synth Unicode");
        // `parseFontFileName` pushes to the front, so the last declaration is applied first.
        assert_eq!(
            ini.local_font_files(),
            [b"second.ttf".to_vec(), b"first.ttf".to_vec()]
        );
        assert_eq!(ini.military_caption_speed(), 3);
        assert_eq!(ini.military_caption_delay_ms(), 500);
        assert!(ini.use_hard_word_wrap());
        assert!((ini.resolution_font_adjustment() - 0.5).abs() < f32::EPSILON);
        assert_eq!(
            ini.resolution_font_size_method(),
            ResolutionFontSizeMethod::Strict
        );

        let window = ini.font(LanguageFontRole::DefaultWindow);
        assert_eq!(window.name_bytes(), b"Synth Sans");
        assert_eq!(window.size(), 11);
        assert!(!window.bold());
        assert!(window.is_declared());

        let copyright = ini.font(LanguageFontRole::Copyright);
        assert_eq!(copyright.name_bytes(), b"SynthMono");
        assert!(copyright.bold());

        // An undeclared role keeps the source constructor default and says so.
        let credits = ini.font(LanguageFontRole::CreditsTitle);
        assert_eq!(credits.name_bytes(), b"Arial Unicode MS");
        assert_eq!(credits.size(), 12);
        assert!(!credits.is_declared());
        assert!(ini.diagnostics().is_empty());
    }

    #[test]
    fn every_established_role_is_reachable_by_its_field_name() {
        let mut source = Vec::from(b"Language\n".as_slice());
        for (index, role) in LanguageFontRole::ALL.into_iter().enumerate() {
            source.extend_from_slice(role.field_name().as_bytes());
            source.extend_from_slice(
                format!(" = \"Synth Face {index}\" {} No\n", index + 5).as_bytes(),
            );
        }
        source.extend_from_slice(b"End\n");
        let ini = parse_language_ini(&source, UiIniLimits::default()).expect("decode all roles");
        assert!(ini.diagnostics().is_empty());
        for (index, role) in LanguageFontRole::ALL.into_iter().enumerate() {
            let font = ini.font(role);
            assert_eq!(font.name_bytes(), format!("Synth Face {index}").as_bytes());
            assert_eq!(font.size(), i32::try_from(index).expect("small index") + 5);
            assert!(font.is_declared());
        }
    }

    #[test]
    fn an_absent_block_yields_visible_defaults() {
        let ini = parse_language_ini(b"; nothing here\n", UiIniLimits::default())
            .expect("decode empty file");
        assert!(!ini.is_declared());
        assert_eq!(ini, LanguageIni::default());
        assert_eq!(ini.military_caption_delay_ms(), 750);
        assert!((ini.resolution_font_adjustment() - 0.7).abs() < f32::EPSILON);
        assert_eq!(
            ini.resolution_font_size_method(),
            ResolutionFontSizeMethod::ClassicNoCeiling
        );
    }

    #[test]
    fn unknown_and_malformed_fields_stay_discoverable() {
        let ini = parse_language_ini(
            b"Language\n  Sparkle = Yes\n  ResolutionFontSizeMethod = SIDEWAYS\n  \
              MessageFont = \"Synth\" twelve No\nEnd\n",
            UiIniLimits::default(),
        )
        .expect("decode with diagnostics");
        assert_eq!(ini.diagnostics().len(), 3);
        assert_eq!(
            ini.diagnostics()[0].kind(),
            &UiIniDiagnosticKind::UnknownField {
                field: "Sparkle".to_owned().into_boxed_str(),
            }
        );
        assert!(matches!(
            ini.diagnostics()[1].kind(),
            UiIniDiagnosticKind::MalformedField { field, .. } if &**field == "ResolutionFontSizeMethod"
        ));
        assert!(matches!(
            ini.diagnostics()[2].kind(),
            UiIniDiagnosticKind::MalformedField { field, .. } if &**field == "MessageFont"
        ));
        assert!(!ini.font(LanguageFontRole::Message).is_declared());
    }

    #[test]
    fn rejects_structural_failures_and_limit_excess() {
        assert_eq!(
            parse_language_ini(b"Language\n", UiIniLimits::default()),
            Err(UiIniError::UnterminatedBlock {
                format: UiIniFormat::Language,
                line: 1,
            })
        );
        let limits = UiIniLimits {
            max_list_entries: 1,
            ..UiIniLimits::default()
        };
        assert!(matches!(
            parse_language_ini(
                b"Language\n  LocalFontFile = a.ttf\n  LocalFontFile = b.ttf\nEnd\n",
                limits
            ),
            Err(UiIniError::TooManyListEntries {
                line: 3,
                limit: 1,
                ..
            })
        ));
    }

    #[test]
    fn truncating_at_every_prefix_never_panics() {
        let complete = b"Language\n  UnicodeFontName = Synth\n  LocalFontFile = a.ttf\n  \
                         ResolutionFontAdjustment = 0.7\n  MessageFont = \"Synth Sans\" 12 No\nEnd\n";
        for length in 0..=complete.len() {
            let _ = parse_language_ini(&complete[..length], UiIniLimits::default());
        }
    }
}
