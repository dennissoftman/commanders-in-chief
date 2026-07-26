// Commanders in Chief
// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only
//
// Provenance: the `WindowTransition` block shape, its `Window` sub-block and `FireOnce` field, the
// style vocabulary, and every style's frame length are derived from Electronic Arts' GPL-3.0 source
// release, GeneralsGameCode revision 9f7abb866f5afd446db14149979e744c7216baaf, specifically
// `Core/GameEngine/Source/GameClient/GUI/GameWindowTransitions.cpp`
// (`INI::parseWindowTransitions`, `GameWindowTransitionsHandler::m_gameWindowTransitionsFieldParseTable`,
// `GameWindowTransitionsHandler::parseWindow`, `GameWindowTransitionsHandler::getNewGroup`,
// `GameWindowTransitionsHandler::findGroup`, `TransitionWindow::getTotalFrames`),
// `Core/GameEngine/Include/GameClient/GameWindowTransitions.h` (`TransitionStyleNames`, every
// `*TRANSITION_END` constant, `TransitionWindow`, `TransitionGroup`), and
// `GeneralsMD/Code/GameEngine/Source/GameClient/GUI/GameWindowTransitionsStyles.cpp` (each
// transition's constructor and `init`, which set `m_frameLength`). This decoder is a bounded,
// project-authored implementation and contains no retail data.

use crate::ui_ini::{
    LineReader, Separators, Tokens, UiIniDiagnostic, UiIniDiagnosticKind, UiIniError, UiIniFormat,
    UiIniLimits, diagnostic_text, index_of_name, is_end_token, scan_bool, scan_int,
};

/// Which established animation a transition window runs.
///
/// The vocabulary and its order are `TransitionStyleNames`; the order matters because the source
/// stores the style as that table's index and dispatches on it in `getTransitionForStyle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionStyle {
    /// `FLASH`: the window's enabled image flashed in, then faded toward the background.
    Flash,
    /// `BUTTONFLASH`: `FLASH` followed by a gradient wipe across a push button.
    ButtonFlash,
    /// `WINFADE`: the window's enabled image drawn at rising alpha.
    WinFade,
    /// `WINSCALEUP`: the window grown from its centre to its full rectangle.
    WinScaleUp,
    /// `MAINMENUSCALEUP`: a main-menu panel grown into a companion window's rectangle.
    MainMenuScaleUp,
    /// `TYPETEXT`: a static text's label revealed one character per frame.
    TypeText,
    /// `SCREENFADE`: a full-viewport fade.
    ScreenFade,
    /// `COUNTUP`: a static text's integer counted up from zero.
    CountUp,
    /// `FULLFADE`: the window's rectangle faded over the whole screen.
    FullFade,
    /// `TEXTONFRAME`: a label shown on one frame, with no animation.
    TextOnFrame,
    /// `MAINMENUMEDIUMSCALEUP`: the medium-length main-menu grow.
    MainMenuMediumScaleUp,
    /// `MAINMENUSMALLSCALEDOWN`: the main-menu shrink.
    MainMenuSmallScaleDown,
    /// `CONTROLBARARROW`: the in-game control bar's sliding arrow.
    ControlBarArrow,
    /// `SCORESCALEUP`: the score screen's grow.
    ScoreScaleUp,
    /// `REVERSESOUND`: a sound fired on one frame, with nothing drawn.
    ReverseSound,
}

/// Every style name in `TransitionStyleNames` order.
const STYLE_NAMES: [&str; 15] = [
    "FLASH",
    "BUTTONFLASH",
    "WINFADE",
    "WINSCALEUP",
    "MAINMENUSCALEUP",
    "TYPETEXT",
    "SCREENFADE",
    "COUNTUP",
    "FULLFADE",
    "TEXTONFRAME",
    "MAINMENUMEDIUMSCALEUP",
    "MAINMENUSMALLSCALEDOWN",
    "CONTROLBARARROW",
    "SCORESCALEUP",
    "REVERSESOUND",
];

/// Every style in `TransitionStyleNames` order, so an index maps back to a variant.
const STYLES: [TransitionStyle; 15] = [
    TransitionStyle::Flash,
    TransitionStyle::ButtonFlash,
    TransitionStyle::WinFade,
    TransitionStyle::WinScaleUp,
    TransitionStyle::MainMenuScaleUp,
    TransitionStyle::TypeText,
    TransitionStyle::ScreenFade,
    TransitionStyle::CountUp,
    TransitionStyle::FullFade,
    TransitionStyle::TextOnFrame,
    TransitionStyle::MainMenuMediumScaleUp,
    TransitionStyle::MainMenuSmallScaleDown,
    TransitionStyle::ControlBarArrow,
    TransitionStyle::ScoreScaleUp,
    TransitionStyle::ReverseSound,
];

impl TransitionStyle {
    /// Returns the style a `Style` record names, compared case-insensitively.
    ///
    /// `INI::parseLookupList` reaches `scanIndexList`, which compares with `stricmp`, so a style
    /// name's case is insignificant even though field names are case-sensitive.
    #[must_use]
    pub fn from_name(name: &[u8]) -> Option<Self> {
        index_of_name(name, &STYLE_NAMES).map(|index| STYLES[index])
    }

    /// Returns the canonical `TransitionStyleNames` spelling.
    #[must_use]
    pub const fn name(self) -> &'static str {
        STYLE_NAMES[self.index()]
    }

    /// Returns the style's index in `TransitionStyleNames`, which is what the source stores.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Flash => 0,
            Self::ButtonFlash => 1,
            Self::WinFade => 2,
            Self::WinScaleUp => 3,
            Self::MainMenuScaleUp => 4,
            Self::TypeText => 5,
            Self::ScreenFade => 6,
            Self::CountUp => 7,
            Self::FullFade => 8,
            Self::TextOnFrame => 9,
            Self::MainMenuMediumScaleUp => 10,
            Self::MainMenuSmallScaleDown => 11,
            Self::ControlBarArrow => 12,
            Self::ScoreScaleUp => 13,
            Self::ReverseSound => 14,
        }
    }

    /// Returns how many 30-per-second frames the style's state machine spans.
    ///
    /// This is the style's `*TRANSITION_END` constant, which every transition's constructor assigns
    /// to `m_frameLength`. Three styles shorten it at `init` from data rather than from the
    /// definition, so this is the declared maximum for them: `TYPETEXT` runs one frame per character
    /// of the static text it animates, `COUNTUP` runs one frame per one, hundred, or thousand of the
    /// integer it counts to, and `COUNTUP` runs no frames at all when its window starts hidden.
    #[must_use]
    pub const fn declared_frame_length(self) -> i32 {
        match self {
            // START 0, FADE_IN_1..3, FADE_TO_BACKGROUND_1..4, END.
            Self::Flash => 8,
            // FLASH's states plus FADE_TO_GRADE_IN_1 = 11 through FADE_TO_GRADE_OUT_4 = 16, END.
            Self::ButtonFlash => 17,
            // `WINFADE` counts START 0, FADE_IN_1..9, END; `FULLFADE` declares END = 10 outright.
            Self::WinFade | Self::FullFade => 10,
            // START 0, states 1..5, END.
            Self::WinScaleUp | Self::MainMenuSmallScaleDown | Self::ScoreScaleUp => 6,
            Self::MainMenuScaleUp => 5,
            Self::MainMenuMediumScaleUp => 3,
            // Both text styles cap at 30 and shorten from their window's own text; `SCREENFADE`
            // declares END = 30 outright.
            Self::TypeText | Self::CountUp | Self::ScreenFade => 30,
            // One state after START: the label is simply present.
            Self::TextOnFrame => 1,
            // START 0, BEGIN_FADE 16, END 22.
            Self::ControlBarArrow => 22,
            // START 0, FIRESOUND 1, END 2.
            Self::ReverseSound => 2,
        }
    }
}

/// One window a transition group animates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionWindowDef {
    line: usize,
    window_name: Vec<u8>,
    style: TransitionStyle,
    frame_delay: i32,
}

impl TransitionWindowDef {
    /// Returns the one-based line the `Window` sub-block opened on.
    #[must_use]
    pub const fn line(&self) -> usize {
        self.line
    }

    /// Returns the decorated window name exactly as spelled, empty when the block declares none.
    ///
    /// This is the same decorated `<layout>:<control>` spelling a WND `NAME` record carries, because
    /// the source turns it into a name key and asks the window manager for the window with that id.
    /// `nameToKey` compares with `strcmp`, so the match is case-sensitive.
    #[must_use]
    pub fn window_name_bytes(&self) -> &[u8] {
        &self.window_name
    }

    /// Returns the animation to run.
    #[must_use]
    pub const fn style(&self) -> TransitionStyle {
        self.style
    }

    /// Returns how many frames pass before this window's animation starts.
    #[must_use]
    pub const fn frame_delay(&self) -> i32 {
        self.frame_delay
    }

    /// Returns the frame this window's animation finishes on, as `getTotalFrames` computes it.
    #[must_use]
    pub const fn total_frames(&self) -> i32 {
        self.frame_delay
            .saturating_add(self.style.declared_frame_length())
    }
}

/// One named, immutable transition group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionGroupDef {
    line: usize,
    name: Vec<u8>,
    fire_once: bool,
    windows: Vec<TransitionWindowDef>,
}

impl TransitionGroupDef {
    /// Returns the one-based line the group opened on.
    #[must_use]
    pub const fn line(&self) -> usize {
        self.line
    }

    /// Returns the group name exactly as spelled.
    #[must_use]
    pub fn name_bytes(&self) -> &[u8] {
        &self.name
    }

    /// Returns whether the group runs once and then clears itself.
    ///
    /// A group that is not fire-once stays current after finishing, and the handler reverses it when
    /// another group is set, which is how a menu's forward animation plays backwards on the way out.
    #[must_use]
    pub const fn fire_once(&self) -> bool {
        self.fire_once
    }

    /// Returns every window in declaration order, which is the order the source appends them.
    #[must_use]
    pub fn windows(&self) -> &[TransitionWindowDef] {
        &self.windows
    }

    /// Returns the frame the whole group finishes on: the longest of its windows' totals.
    #[must_use]
    pub fn total_frames(&self) -> i32 {
        self.windows
            .iter()
            .map(TransitionWindowDef::total_frames)
            .max()
            .unwrap_or(0)
    }
}

/// A bounded, immutable set of transition groups from one `Data/INI/WindowTransitions.ini`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowTransitionsIni {
    groups: Vec<TransitionGroupDef>,
    diagnostics: Vec<UiIniDiagnostic>,
}

impl WindowTransitionsIni {
    /// Returns every group in source order.
    #[must_use]
    pub fn groups(&self) -> &[TransitionGroupDef] {
        &self.groups
    }

    /// Returns the group with this name, compared case-insensitively.
    ///
    /// `GameWindowTransitionsHandler::findGroup` compares with `compareNoCase`, so a caller naming
    /// a group resolves it whatever the case, even though the definition itself is stored verbatim.
    #[must_use]
    pub fn find(&self, name: &[u8]) -> Option<&TransitionGroupDef> {
        self.groups
            .iter()
            .find(|group| group.name.eq_ignore_ascii_case(name))
    }

    /// Returns every non-fatal observation in encounter order.
    #[must_use]
    pub fn diagnostics(&self) -> &[UiIniDiagnostic] {
        &self.diagnostics
    }
}

const FORMAT: UiIniFormat = UiIniFormat::WindowTransition;

/// Decodes one `Data/INI/WindowTransitions.ini` into immutable transition groups.
///
/// # Errors
///
/// Returns a structured error when the input or any count, name, or value exceeds its explicit
/// limit, when a block opens without a name, or when a block is never closed by `End`.
pub fn parse_window_transitions_ini(
    bytes: &[u8],
    limits: UiIniLimits,
) -> Result<WindowTransitionsIni, UiIniError> {
    let mut reader = LineReader::new(bytes, FORMAT, limits)?;
    let mut groups: Vec<TransitionGroupDef> = Vec::new();
    let mut diagnostics = Vec::new();

    while let Some((line, text)) = reader.next_line()? {
        let mut tokens = Tokens::new(text);
        let Some(keyword) = tokens.next(Separators::Default) else {
            continue;
        };
        if keyword != b"WindowTransition" {
            diagnostics.push(UiIniDiagnostic::new(
                line,
                UiIniDiagnosticKind::UnknownBlock {
                    keyword: diagnostic_text(keyword),
                },
            ));
            skip_block(&mut reader, line, false)?;
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
        // A repeated group name is dropped rather than merged. `getNewGroup` refuses to allocate a
        // second group with an existing name and returns nothing, so the source parses the repeated
        // definition's fields into no group at all — unlike every other UI definition family, where
        // a later definition overwrites the earlier one's fields.
        if let Some(first) = groups
            .iter()
            .find(|group| group.name.eq_ignore_ascii_case(name))
        {
            diagnostics.push(UiIniDiagnostic::new(
                line,
                UiIniDiagnosticKind::DuplicateDefinition {
                    name: diagnostic_text(name),
                    first_line: first.line,
                },
            ));
            skip_block(&mut reader, line, true)?;
            continue;
        }
        if groups.len() >= limits.max_definitions {
            return Err(UiIniError::TooManyDefinitions {
                format: FORMAT,
                line,
                limit: limits.max_definitions,
            });
        }
        let mut group = TransitionGroupDef {
            line,
            name: name.to_vec(),
            fire_once: false,
            windows: Vec::new(),
        };
        decode_group(&mut reader, &mut group, &mut diagnostics, line, limits)?;
        groups.push(group);
    }

    Ok(WindowTransitionsIni {
        groups,
        diagnostics,
    })
}

fn decode_group(
    reader: &mut LineReader<'_>,
    group: &mut TransitionGroupDef,
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
            b"Window" => {
                if group.windows.len() >= limits.max_list_entries {
                    return Err(UiIniError::TooManyListEntries {
                        format: FORMAT,
                        line,
                        field: diagnostic_text(field),
                        limit: limits.max_list_entries,
                    });
                }
                let window = decode_window(reader, diagnostics, line, limits)?;
                group.windows.push(window);
            }
            b"FireOnce" => match tokens.next(Separators::Default).and_then(scan_bool) {
                Some(value) => group.fire_once = value,
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

fn decode_window(
    reader: &mut LineReader<'_>,
    diagnostics: &mut Vec<UiIniDiagnostic>,
    opened: usize,
    limits: UiIniLimits,
) -> Result<TransitionWindowDef, UiIniError> {
    // `TransitionWindow`'s constructor leaves the name empty, the delay zero, and the style at index
    // zero, which is `FLASH`.
    let mut window = TransitionWindowDef {
        line: opened,
        window_name: Vec::new(),
        style: TransitionStyle::Flash,
        frame_delay: 0,
    };
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
            return Ok(window);
        }
        match field {
            b"WinName" => {
                let Some(value) = tokens.next_ascii_string() else {
                    push_malformed(diagnostics, line, field, "expected a decorated window name");
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
                window.window_name = value;
            }
            b"Style" => {
                match tokens
                    .next(Separators::Default)
                    .and_then(TransitionStyle::from_name)
                {
                    Some(style) => window.style = style,
                    None => push_malformed(
                        diagnostics,
                        line,
                        field,
                        "expected an established transition style name",
                    ),
                }
            }
            b"FrameDelay" => match tokens.next(Separators::Default).and_then(scan_int) {
                Some(value) => window.frame_delay = value,
                None => push_malformed(diagnostics, line, field, "expected an integer"),
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

/// Consumes a block this decoder does not own, up to its first `End`.
///
/// A repeated `WindowTransition` is the one owned block skipped this way, and its nested `Window`
/// sub-blocks are counted so the group's own `End` is the one that closes it.
fn skip_block(
    reader: &mut LineReader<'_>,
    opened: usize,
    count_windows: bool,
) -> Result<(), UiIniError> {
    let mut depth = 1_usize;
    loop {
        let Some((_, text)) = reader.next_line()? else {
            return Err(UiIniError::UnterminatedBlock {
                format: FORMAT,
                line: opened,
            });
        };
        let mut tokens = Tokens::new(text);
        let Some(token) = tokens.next(Separators::Default) else {
            continue;
        };
        if is_end_token(token) {
            depth -= 1;
            if depth == 0 {
                return Ok(());
            }
        } else if count_windows && token == b"Window" {
            depth += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{TransitionStyle, parse_window_transitions_ini};
    use crate::ui_ini::{
        UiIniDiagnostic, UiIniDiagnosticKind, UiIniError, UiIniFormat, UiIniLimits,
    };

    #[test]
    fn decodes_groups_windows_styles_and_delays() {
        let ini = parse_window_transitions_ini(
            b"; synthetic fixture\n\
              WindowTransition SynthMenu\n\
                Window\n\
                  WinName = SynthMenu.wnd:SynthRuler\n\
                  Style   = WINFADE\n\
                  FrameDelay = 0\n\
                END\n\
              \n\
                Window\n\
                  WinName = SynthMenu.wnd:ButtonOne\n\
                  Style   = buttonflash\n\
                  FrameDelay = 5\n\
                END\n\
                FireOnce = YES\n\
              END\n\
              WindowTransition SynthMenuBack\n\
                Window\n\
                  WinName = SynthMenu.wnd:ButtonOne\n\
                  Style   = TYPETEXT\n\
                END\n\
              END\n",
            UiIniLimits::default(),
        )
        .expect("decode transition groups");
        assert!(ini.diagnostics().is_empty());
        assert_eq!(ini.groups().len(), 2);

        let menu = &ini.groups()[0];
        assert_eq!(menu.name_bytes(), b"SynthMenu");
        assert!(menu.fire_once());
        assert_eq!(menu.windows().len(), 2);
        assert_eq!(
            menu.windows()[0].window_name_bytes(),
            b"SynthMenu.wnd:SynthRuler"
        );
        assert_eq!(menu.windows()[0].style(), TransitionStyle::WinFade);
        assert_eq!(menu.windows()[0].frame_delay(), 0);
        assert_eq!(menu.windows()[0].total_frames(), 10);
        // A style name's case is insignificant; `scanIndexList` compares with `stricmp`.
        assert_eq!(menu.windows()[1].style(), TransitionStyle::ButtonFlash);
        assert_eq!(menu.windows()[1].frame_delay(), 5);
        assert_eq!(menu.windows()[1].total_frames(), 22);
        assert_eq!(menu.total_frames(), 22);

        // A group with no `FireOnce` is not fire-once, matching the constructor.
        let back = &ini.groups()[1];
        assert!(!back.fire_once());
        assert_eq!(back.windows()[0].frame_delay(), 0);

        // Group lookup is case-insensitive, matching `findGroup`.
        assert!(ini.find(b"synthmenu").is_some());
        assert!(ini.find(b"SynthMenuMissing").is_none());
    }

    #[test]
    fn every_established_style_name_resolves_to_its_frame_length() {
        let expected: [(&[u8], TransitionStyle, i32); 15] = [
            (b"FLASH", TransitionStyle::Flash, 8),
            (b"BUTTONFLASH", TransitionStyle::ButtonFlash, 17),
            (b"WINFADE", TransitionStyle::WinFade, 10),
            (b"WINSCALEUP", TransitionStyle::WinScaleUp, 6),
            (b"MAINMENUSCALEUP", TransitionStyle::MainMenuScaleUp, 5),
            (b"TYPETEXT", TransitionStyle::TypeText, 30),
            (b"SCREENFADE", TransitionStyle::ScreenFade, 30),
            (b"COUNTUP", TransitionStyle::CountUp, 30),
            (b"FULLFADE", TransitionStyle::FullFade, 10),
            (b"TEXTONFRAME", TransitionStyle::TextOnFrame, 1),
            (
                b"MAINMENUMEDIUMSCALEUP",
                TransitionStyle::MainMenuMediumScaleUp,
                3,
            ),
            (
                b"MAINMENUSMALLSCALEDOWN",
                TransitionStyle::MainMenuSmallScaleDown,
                6,
            ),
            (b"CONTROLBARARROW", TransitionStyle::ControlBarArrow, 22),
            (b"SCORESCALEUP", TransitionStyle::ScoreScaleUp, 6),
            (b"REVERSESOUND", TransitionStyle::ReverseSound, 2),
        ];
        for (index, (name, style, frames)) in expected.into_iter().enumerate() {
            assert_eq!(TransitionStyle::from_name(name), Some(style));
            assert_eq!(style.name().as_bytes(), name);
            assert_eq!(style.index(), index);
            assert_eq!(style.declared_frame_length(), frames);
        }
        assert_eq!(TransitionStyle::from_name(b"SPIN"), None);
    }

    #[test]
    fn unknown_and_malformed_records_are_reported_not_guessed() {
        let ini = parse_window_transitions_ini(
            b"WindowTransition Synth\n\
                Window\n\
                  WinName = Synth.wnd:Button\n\
                  Style = SPIN\n\
                  FrameDelay = soon\n\
                  Wobble = 3\n\
                END\n\
                FireOnce = maybe\n\
                Sparkle = Yes\n\
              END\n\
              ControlBarScheme Synth\n\
                ScreenCreationRes = 800 600\n\
              END\n",
            UiIniLimits::default(),
        )
        .expect("decode diagnostics");
        let window = &ini.groups()[0].windows()[0];
        // An unreadable style stays at the constructor's default rather than being guessed.
        assert_eq!(window.style(), TransitionStyle::Flash);
        assert_eq!(window.frame_delay(), 0);
        assert!(!ini.groups()[0].fire_once());
        let kinds: Vec<&UiIniDiagnosticKind> = ini
            .diagnostics()
            .iter()
            .map(UiIniDiagnostic::kind)
            .collect();
        assert!(matches!(
            kinds[0],
            UiIniDiagnosticKind::MalformedField { field, .. } if &**field == "Style"
        ));
        assert!(matches!(
            kinds[1],
            UiIniDiagnosticKind::MalformedField { field, .. } if &**field == "FrameDelay"
        ));
        assert_eq!(
            kinds[2],
            &UiIniDiagnosticKind::UnknownField {
                field: "Wobble".to_owned().into_boxed_str(),
            }
        );
        assert!(matches!(
            kinds[3],
            UiIniDiagnosticKind::MalformedField { field, .. } if &**field == "FireOnce"
        ));
        assert_eq!(
            kinds[4],
            &UiIniDiagnosticKind::UnknownField {
                field: "Sparkle".to_owned().into_boxed_str(),
            }
        );
        // A block this decoder does not own is skipped to its `End` and named in a diagnostic.
        assert_eq!(
            kinds[5],
            &UiIniDiagnosticKind::UnknownBlock {
                keyword: "ControlBarScheme".to_owned().into_boxed_str(),
            }
        );
        assert_eq!(ini.groups().len(), 1);
    }

    #[test]
    fn a_duplicate_group_name_drops_the_later_definition() {
        let ini = parse_window_transitions_ini(
            b"WindowTransition Synth\n  FireOnce = Yes\nEND\n\
              WindowTransition synth\n\
                Window\n  WinName = Synth.wnd:Late\nEND\n\
                FireOnce = No\n\
              END\n",
            UiIniLimits::default(),
        )
        .expect("decode duplicate groups");
        assert_eq!(ini.groups().len(), 1);
        assert!(ini.groups()[0].fire_once());
        assert!(ini.groups()[0].windows().is_empty());
        assert_eq!(
            ini.diagnostics()[0].kind(),
            &UiIniDiagnosticKind::DuplicateDefinition {
                name: "synth".to_owned().into_boxed_str(),
                first_line: 1,
            }
        );
    }

    #[test]
    fn rejects_structural_failures_and_truncation_never_panics() {
        assert_eq!(
            parse_window_transitions_ini(b"WindowTransition Synth\n", UiIniLimits::default()),
            Err(UiIniError::UnterminatedBlock {
                format: UiIniFormat::WindowTransition,
                line: 1,
            })
        );
        assert_eq!(
            parse_window_transitions_ini(
                b"WindowTransition Synth\n  Window\n    WinName = a\nEND\n",
                UiIniLimits::default()
            ),
            Err(UiIniError::UnterminatedBlock {
                format: UiIniFormat::WindowTransition,
                line: 1,
            })
        );
        assert_eq!(
            parse_window_transitions_ini(b"WindowTransition\nEND\n", UiIniLimits::default()),
            Err(UiIniError::MissingBlockName {
                format: UiIniFormat::WindowTransition,
                line: 1,
            })
        );
        let limits = UiIniLimits {
            max_list_entries: 1,
            ..UiIniLimits::default()
        };
        assert_eq!(
            parse_window_transitions_ini(
                b"WindowTransition Synth\n\
                    Window\n  WinName = a\nEND\n\
                    Window\n  WinName = b\nEND\n\
                  END\n",
                limits
            ),
            Err(UiIniError::TooManyListEntries {
                format: UiIniFormat::WindowTransition,
                line: 5,
                field: "Window".to_owned().into_boxed_str(),
                limit: 1,
            })
        );
        let complete = b"WindowTransition Synth\n\
                           Window\n    WinName = Synth.wnd:B\n    Style = WINFADE\n\
                           FrameDelay = 2\n  END\n  FireOnce = Yes\nEND\n";
        for length in 0..=complete.len() {
            let _ = parse_window_transitions_ini(&complete[..length], UiIniLimits::default());
        }
    }
}
