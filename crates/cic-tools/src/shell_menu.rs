// Commanders in Chief
// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only
//
// Provenance: every control name, the set of windows the main menu hides when it opens, the fact
// that the default panel is revealed by the player's first input rather than by initialization, and
// each button's combination of panel reveal and transition-group operations are derived from
// Electronic Arts' GPL-3.0 source release, GeneralsGameCode revision
// 9f7abb866f5afd446db14149979e744c7216baaf, specifically
// `GeneralsMD/Code/GameEngine/Source/GameClient/GUI/GUICallbacks/Menus/MainMenu.cpp`
// (`MainMenuInit`, `initialHide`, `showSelectiveButtons`, `MainMenuInput`, `MainMenuSystem`'s
// `GBM_SELECTED` arm, and `quitCallback`). Expressing them as a data table, rather than as native
// menu code, is project design: R4 runs no source callback, so the presentation behaviour those
// functions carried has to arrive as an allowlisted typed sequence or not at all.

use cic_ui::{UiActionAllowlist, UiDemoAction, UiShellEvent};

/// The layout a binding set belongs to, spelled the way the source spells a pushed layout.
///
/// `Shell::push` is called with `Menus/<file>.wnd` while the archives hold `Window/Menus/<file>.wnd`,
/// so a caller joins this against its own menu-directory prefix rather than this table hardcoding
/// one.
pub const MAIN_MENU_PATH: &str = "Menus/MainMenu.wnd";

/// A project-owned binding set for one shell screen.
///
/// This is the whole of what the demo shell knows about a retail menu: which windows it closes with,
/// what its first input reveals, and which controls may run which typed actions. Nothing here is
/// consulted by `cic-ui` or `cic-render` — a caller applies it — so no runtime or renderer path
/// searches for a special window name.
#[derive(Debug, Clone)]
pub struct ShellMenuBindings {
    /// The source-spelled layout path.
    pub path: &'static str,
    /// Windows hidden as the screen opens, in the order the source hides them.
    pub initial_hidden: &'static [&'static str],
    /// Actions the screen's first pointer move or key press runs, once.
    pub first_input: Vec<UiDemoAction>,
    /// Actions the screen runs when its init runs again, having been open before.
    ///
    /// Returning to a screen is not the same as opening it. `Shell::doPop` runs the new top's init,
    /// so the whole initial hidden set is applied a second time and the menu would be left blank —
    /// which is exactly what retail avoids through `MainMenuInit`'s `FirstTimeRunningTheGame` branch
    /// and `MainMenuUpdate`'s `justEntered` delay. These are what those two do the second time
    /// through.
    pub re_entry: Vec<UiDemoAction>,
    /// Which controls may run which actions.
    pub allowlist: UiActionAllowlist,
}

/// What applying one initial-hide entry did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellMenuHide {
    /// The window resolved and was visible, so hiding it changed the screen.
    Hidden,
    /// The window resolved but already declared `HIDDEN`, so the source's call changed nothing
    /// either. Every window `initialHide` names is in this state against retail data.
    AlreadyHidden,
    /// No control of that name exists in the loaded layout.
    Missing,
}

impl ShellMenuHide {
    /// Returns a stable name for a report.
    #[must_use]
    pub const fn row_name(self) -> &'static str {
        match self {
            Self::Hidden => "hidden",
            Self::AlreadyHidden => "already_hidden",
            Self::Missing => "missing",
        }
    }
}

/// What running one routed action did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellMenuActionOutcome {
    /// The action ran.
    Applied,
    /// The action named a control no loaded screen carries.
    UnknownControl,
    /// The action named a transition group the loaded INI does not define.
    UnknownGroup,
    /// The action named a layout the mounted filesystem does not carry.
    UnresolvedLayout,
    /// The action was refused by the shell, which reports why.
    Refused(String),
    /// `Quit` was reached. The demo records it and keeps running; nothing exits.
    QuitRecorded,
}

impl ShellMenuActionOutcome {
    /// Returns a stable name for a report.
    #[must_use]
    pub const fn row_name(&self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::UnknownControl => "unknown_control",
            Self::UnknownGroup => "unknown_group",
            Self::UnresolvedLayout => "unresolved_layout",
            Self::Refused(_) => "refused",
            Self::QuitRecorded => "quit_recorded",
        }
    }
}

/// One thing a scripted menu session did, in the order it happened.
#[derive(Debug, Clone, PartialEq)]
pub enum ShellMenuRecord {
    /// One entry of the screen's initial hidden set was applied.
    InitialHide {
        /// The window named by the binding table.
        control: String,
        /// What applying it did.
        outcome: ShellMenuHide,
    },
    /// The shell reported a state change.
    Shell(UiShellEvent),
    /// The retained runtime reported an input event.
    Input {
        /// The screen the event belongs to.
        screen: usize,
        /// The control's authored name, absent when it declares none.
        control: Option<String>,
        /// The event kind.
        kind: &'static str,
        /// The event's own operands, empty when it has none.
        detail: String,
    },
    /// The screen's first input fired, which is what reveals the default panel.
    FirstInput,
    /// A control's activation routed one allowlisted action.
    Action {
        /// The control that routed it.
        control: String,
        /// The action.
        action: UiDemoAction,
        /// What running it did.
        outcome: ShellMenuActionOutcome,
    },
    /// A control activated but the allowlist routes nothing for it, so nothing ran.
    ///
    /// This is the ordinary path for most of a retail menu, and it is the record that shows a
    /// callback name was retained and left inert rather than dispatched.
    Unrouted {
        /// The control's authored name, absent when it declares none.
        control: Option<String>,
        /// Its retained system callback name, absent when it declares none.
        callback: Option<String>,
    },
    /// Transition frames were stepped.
    Transition {
        /// How many whole frames were stepped.
        frames: usize,
        /// The group that was current when the step finished.
        group: Option<String>,
        /// Whether the handler has nothing left to run.
        finished: bool,
        /// How many diagnostics the steps produced.
        diagnostics: usize,
    },
    /// A capture was written.
    Capture {
        /// Where it was written.
        path: String,
        /// Its width in pixels.
        width: u32,
        /// Its height in pixels.
        height: u32,
        /// The RGBA hash, which is what a repeat run is compared by.
        sha256: String,
        /// How many quads were staged.
        quads: usize,
        /// How many batches were staged.
        batches: usize,
        /// How many text runs were staged.
        text_runs: usize,
        /// How many staging diagnostics the frame produced.
        diagnostics: usize,
    },
    /// One staging diagnostic from the capture just above it.
    ///
    /// These are listed rather than only counted, because the claim that every one traces to retail's
    /// own data — an image no shipped INI defines, or a control the source draws nothing for — is only
    /// checkable if the names are in the report.
    CaptureDiagnostic {
        /// The frame-item index it belongs to.
        item: usize,
        /// The control it belongs to, absent when the diagnostic is not control-scoped.
        control: Option<usize>,
        /// The diagnostic kind.
        kind: &'static str,
        /// Its operand.
        detail: String,
    },
}

/// Windows `MainMenuInit` hides as the main menu opens, in source order.
///
/// The five `MapBorder*` panels come first, from the loop over `dropDownWindows` — note that the
/// loop starts at `DROPDOWN_SINGLE`, so all five real panels are hidden and the unassigned
/// `DROPDOWN_NONE` slot is never dereferenced. `initialHide`'s faction windows follow, then
/// `showSelectiveButtons(SHOW_NONE)`'s six save and load buttons, then `MainMenuRuler`.
///
/// Two of these groups are worth knowing about before reading a diagnostic. Every window
/// `initialHide` names already declares `HIDDEN` in the retail layout, so that call changes nothing
/// against retail data and is kept only because a modded layout need not declare it. And
/// `MainMenuRuler` is hidden unconditionally and shown again only on a *later* entry to the menu:
/// `FirstTimeRunningTheGame` starts true, so the first initialization in a process leaves it hidden,
/// which is the state a capture reproduces.
const MAIN_MENU_INITIAL_HIDDEN: &[&str] = &[
    // `for(i = 1; i < DROPDOWN_COUNT; ++i) dropDownWindows[i]->winHide(TRUE)`, in the order the
    // slots are assigned: single, multiplayer, main, load/replay, difficulty.
    "MainMenu.wnd:MapBorder",
    "MainMenu.wnd:MapBorder1",
    "MainMenu.wnd:MapBorder2",
    "MainMenu.wnd:MapBorder3",
    "MainMenu.wnd:MapBorder4",
    // `initialHide()`, in its own order, repeats included: it names `WinFactionGLA`,
    // `WinFactionChina`, and `WinFactionUS` twice each, which is harmless there and here.
    "MainMenu.wnd:WinFactionGLA",
    "MainMenu.wnd:WinFactionChina",
    "MainMenu.wnd:WinFactionUS",
    "MainMenu.wnd:WinGrowMarker",
    "MainMenu.wnd:WinFactionTraining",
    "MainMenu.wnd:WinFactionTrainingSmall",
    "MainMenu.wnd:WinFactionTrainingMedium",
    "MainMenu.wnd:WinFactionSkirmish",
    "MainMenu.wnd:WinFactionSkirmishSmall",
    "MainMenu.wnd:WinFactionSkirmishMedium",
    "MainMenu.wnd:WinFactionUSSmall",
    "MainMenu.wnd:WinFactionUSMedium",
    "MainMenu.wnd:WinFactionGLASmall",
    "MainMenu.wnd:WinFactionGLAMedium",
    "MainMenu.wnd:WinFactionChinaSmall",
    "MainMenu.wnd:WinFactionChinaMedium",
    // `showSelectiveButtons(SHOW_NONE)`, which hides all six because no campaign is selected.
    "MainMenu.wnd:ButtonUSARecentSave",
    "MainMenu.wnd:ButtonUSALoadGame",
    "MainMenu.wnd:ButtonGLARecentSave",
    "MainMenu.wnd:ButtonGLALoadGame",
    "MainMenu.wnd:ButtonChinaRecentSave",
    "MainMenu.wnd:ButtonChinaLoadGame",
    // `rule->winHide(TRUE)`.
    "MainMenu.wnd:MainMenuRuler",
];

/// Returns the project-owned bindings for the retail main menu.
///
/// The button arms reproduce `MainMenuSystem`'s `GBM_SELECTED` handler in its own order: reveal the
/// panel the press moves to, then `remove` the group that was showing, `reverse` its counterpart,
/// and `setGroup` the one that animates in. The panel a press *leaves* is hidden by that reversed
/// group's own final frame, not by an explicit hide, which is why a demo that skipped transitions
/// would show two panels at once.
#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "one arm per source handler; splitting would separate a binding from its provenance"
)]
pub fn main_menu_bindings() -> ShellMenuBindings {
    let mut allowlist = UiActionAllowlist::new();

    // `controlID == buttonSinglePlayerID`.
    allowlist.allow_all(
        "MainMenu.wnd:ButtonSinglePlayer",
        panel_change(
            "MainMenu.wnd:MapBorder",
            "MainMenuDefaultMenu",
            "MainMenuDefaultMenuBack",
            "MainMenuSinglePlayerMenu",
        ),
    );
    // `controlID == buttonSingleBackID`.
    allowlist.allow_all(
        "MainMenu.wnd:ButtonSingleBack",
        panel_change(
            "MainMenu.wnd:MapBorder2",
            "MainMenuSinglePlayerMenu",
            "MainMenuSinglePlayerMenuBack",
            "MainMenuDefaultMenu",
        ),
    );
    // `controlID == buttonMultiPlayerID`.
    allowlist.allow_all(
        "MainMenu.wnd:ButtonMultiplayer",
        panel_change(
            "MainMenu.wnd:MapBorder1",
            "MainMenuDefaultMenu",
            "MainMenuDefaultMenuBack",
            "MainMenuMultiPlayerMenu",
        ),
    );
    // `controlID == buttonMultiBackID`. The reversed group is spelled `...Reverse` here where the
    // other Back arms spell theirs `...Back`; that asymmetry is the source's, and both names exist.
    allowlist.allow_all(
        "MainMenu.wnd:ButtonMultiBack",
        panel_change(
            "MainMenu.wnd:MapBorder2",
            "MainMenuMultiPlayerMenu",
            "MainMenuMultiPlayerMenuReverse",
            "MainMenuDefaultMenu",
        ),
    );
    // `controlID == buttonLoadReplayID`.
    allowlist.allow_all(
        "MainMenu.wnd:ButtonLoadReplay",
        panel_change(
            "MainMenu.wnd:MapBorder3",
            "MainMenuDefaultMenu",
            "MainMenuDefaultMenuBack",
            "MainMenuLoadReplayMenu",
        ),
    );
    // `controlID == buttonLoadReplayBackID`.
    allowlist.allow_all(
        "MainMenu.wnd:ButtonLoadReplayBack",
        panel_change(
            "MainMenu.wnd:MapBorder2",
            "MainMenuLoadReplayMenu",
            "MainMenuLoadReplayMenuBack",
            "MainMenuDefaultMenu",
        ),
    );

    // `controlID == skirmishID`, which is the one main-menu button that really pushes a screen. It
    // reveals the single-player panel it is standing on, removes the faction group, reverses the
    // skirmish exit group, and pushes. The campaign-selected guard and the script-engine hook it
    // also runs belong to gameplay, so neither is represented.
    allowlist.allow_all(
        "MainMenu.wnd:ButtonSkirmish",
        vec![
            UiDemoAction::ShowControl {
                control: "MainMenu.wnd:MapBorder".to_owned(),
            },
            UiDemoAction::RemoveTransitionGroup {
                group: "MainMenuFactionSkirmish".to_owned(),
                skip_pending: false,
            },
            UiDemoAction::ReverseTransitionGroup {
                group: "MainMenuSinglePlayerMenuBackSkirmish".to_owned(),
            },
            UiDemoAction::PushScreen {
                path: "Menus/SkirmishGameOptionsMenu.wnd".to_owned(),
            },
        ],
    );

    // `controlID == optionsID`. The source does *not* push here: it fetches the shell's cached
    // options layout, runs its init, unhides it, and brings it forward, so the options menu overlays
    // the main menu instead of replacing it on the stack. A push is the bounded equivalent this demo
    // uses, because the retained shell owns no cached-layout slot; the difference is that retail's
    // main menu keeps running its update underneath, which nothing here depends on.
    allowlist.allow(
        "MainMenu.wnd:ButtonOptions",
        UiDemoAction::PushScreen {
            path: "Menus/OptionsMenu.wnd".to_owned(),
        },
    );

    // `controlID == exitID`, whose windowed branch calls `quitCallback` — which pops the shell and
    // sets the engine quitting — and whose fullscreen branch first raises a yes/no box. Only the
    // leaving is represented: `Quit` is data a caller interprets, and nothing here ends a process.
    allowlist.allow("MainMenu.wnd:ButtonExit", UiDemoAction::Quit);

    // Every menu's Back returns one screen, keyed undecorated so it covers the pushed screens too.
    // This is project design rather than one source arm: each pushed menu has its own back handler.
    allowlist.allow("ButtonBack", UiDemoAction::PopScreen);

    ShellMenuBindings {
        path: MAIN_MENU_PATH,
        initial_hidden: MAIN_MENU_INITIAL_HIDDEN,
        // `MainMenuInput`, on the first pointer move over twenty pixels or the first character.
        // `MainMenuFade` is set immediately so it skips whatever was running, then the default menu
        // group is queued behind it. This is why a freshly initialized retail main menu draws no
        // buttons at all: `MainMenuInit` hid every panel and only input reveals one.
        first_input: vec![
            UiDemoAction::ShowControl {
                control: "MainMenu.wnd:MapBorder2".to_owned(),
            },
            UiDemoAction::SetTransitionGroup {
                group: "MainMenuFade".to_owned(),
                immediate: true,
            },
            UiDemoAction::SetTransitionGroup {
                group: "MainMenuDefaultMenu".to_owned(),
                immediate: false,
            },
        ],
        // Coming back from a pushed screen. `MainMenuInit` runs again and hides everything again,
        // but `FirstTimeRunningTheGame` is now false, so it takes the other branch: it shows
        // `MainMenuRuler` instead of leaving it hidden, and arms `justEntered`. Two updates later
        // `MainMenuUpdate` sets `MainMenuDefaultMenuLogoFade` — the same default panel as the first
        // time, plus the logo — and puts focus back on the parent. The delay is a frame counter with
        // no presentation effect once the group is armed, so it is not reproduced.
        re_entry: vec![
            UiDemoAction::ShowControl {
                control: "MainMenu.wnd:MainMenuRuler".to_owned(),
            },
            // No explicit reveal of `MapBorder2` here, unlike the first-input path: the group's own
            // `FLASH` on that window unhides it at frame four and leaves it shown, and the source's
            // one `winHide(FALSE)` call at this point is commented out. The buttons follow from
            // their own `BUTTONFLASH` entries the same way.
            UiDemoAction::SetTransitionGroup {
                group: "MainMenuDefaultMenuLogoFade".to_owned(),
                immediate: false,
            },
            UiDemoAction::FocusControl {
                control: "MainMenu.wnd:MainMenuParent".to_owned(),
            },
        ],
        allowlist,
    }
}

/// Builds the four actions every main-menu panel change runs, in `MainMenuSystem`'s order.
fn panel_change(reveal: &str, remove: &str, reverse: &str, set: &str) -> Vec<UiDemoAction> {
    vec![
        UiDemoAction::ShowControl {
            control: reveal.to_owned(),
        },
        UiDemoAction::RemoveTransitionGroup {
            group: remove.to_owned(),
            skip_pending: false,
        },
        UiDemoAction::ReverseTransitionGroup {
            group: reverse.to_owned(),
        },
        UiDemoAction::SetTransitionGroup {
            group: set.to_owned(),
            immediate: false,
        },
    ]
}
