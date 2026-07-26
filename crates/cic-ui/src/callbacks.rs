// Commanders in Chief
// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only
//
// Provenance: the nine callback tables, their exact spellings, the table each callback record is
// looked up in, and the rule that a name outside its table resolves to nothing are derived from
// Electronic Arts' GPL-3.0 source release, GeneralsGameCode revision
// 9f7abb866f5afd446db14149979e744c7216baaf, specifically
// `GeneralsMD/Code/GameEngine/Source/Common/System/FunctionLexicon.cpp` (`gameWinSystemTable`,
// `gameWinInputTable`, `gameWinTooltipTable`, `gameWinDrawTable`, `winLayoutInitTable`,
// `winLayoutUpdateTable`, `winLayoutShutdownTable`, `FunctionLexicon::init`,
// `FunctionLexicon::keyToFunc`, `FunctionLexicon::findFunction`),
// `GeneralsMD/Code/GameEngine/Include/Common/FunctionLexicon.h` (`TableIndex` and the default table
// of each typed accessor),
// `GeneralsMD/Code/GameEngineDevice/Source/W3DDevice/Common/System/W3DFunctionLexicon.cpp` (the
// device draw and device layout-init tables and `W3DFunctionLexicon::init`),
// `GeneralsMD/Code/GameEngine/Source/GameClient/GUI/GameWindowManagerScript.cpp`
// (`parseSystemCallback`, `parseInputCallback`, `parseTooltipCallback`, `parseDrawCallback`,
// `parseInit`, `parseUpdate`, `parseShutdown`, which each look the authored name up through
// `TheNameKeyGenerator->nameToKey`), and
// `GeneralsMD/Code/GameEngine/Source/Common/NameKeyGenerator.cpp` (`nameToKey`, which compares with
// `strcmp`, so the lookup is case-sensitive). Typed demo actions and the action allowlist are
// project design: the original dispatches a resolved name to native code, and this project never
// does.

use std::collections::BTreeMap;

/// One of the original's nine callback tables.
///
/// The order is `TableIndex`, which matters: a lookup allowed to search every table walks them in
/// this order and takes the first match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum UiCallbackTable {
    /// `TABLE_GAME_WIN_SYSTEM`.
    System,
    /// `TABLE_GAME_WIN_INPUT`.
    Input,
    /// `TABLE_GAME_WIN_TOOLTIP`.
    Tooltip,
    /// `TABLE_GAME_WIN_DEVICEDRAW`, the W3D device's own draw procedures.
    DeviceDraw,
    /// `TABLE_GAME_WIN_DRAW`.
    Draw,
    /// `TABLE_WIN_LAYOUT_INIT`.
    LayoutInit,
    /// `TABLE_WIN_LAYOUT_DEVICEINIT`, the W3D device's own layout initializers.
    LayoutDeviceInit,
    /// `TABLE_WIN_LAYOUT_UPDATE`.
    LayoutUpdate,
    /// `TABLE_WIN_LAYOUT_SHUTDOWN`.
    LayoutShutdown,
}

/// Every callback table, in `TableIndex` order.
pub const UI_CALLBACK_TABLES: [UiCallbackTable; 9] = [
    UiCallbackTable::System,
    UiCallbackTable::Input,
    UiCallbackTable::Tooltip,
    UiCallbackTable::DeviceDraw,
    UiCallbackTable::Draw,
    UiCallbackTable::LayoutInit,
    UiCallbackTable::LayoutDeviceInit,
    UiCallbackTable::LayoutUpdate,
    UiCallbackTable::LayoutShutdown,
];

impl UiCallbackTable {
    /// Returns the stable report name for the table.
    #[must_use]
    pub const fn row_name(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Input => "input",
            Self::Tooltip => "tooltip",
            Self::DeviceDraw => "device_draw",
            Self::Draw => "draw",
            Self::LayoutInit => "layout_init",
            Self::LayoutDeviceInit => "layout_device_init",
            Self::LayoutUpdate => "layout_update",
            Self::LayoutShutdown => "layout_shutdown",
        }
    }

    /// Returns the table's contents, exactly as the source spells each entry.
    ///
    /// This is the Zero Hour lexicon, which is a strict superset: base Generals ships the same nine
    /// tables minus the six names in [`ZERO_HOUR_ONLY_NAMES`], and identical device tables. Use
    /// [`UiCallbackTable::contains`] when the edition matters.
    #[must_use]
    pub const fn names(self) -> &'static [&'static str] {
        match self {
            Self::System => SYSTEM_NAMES,
            Self::Input => INPUT_NAMES,
            Self::Tooltip => TOOLTIP_NAMES,
            Self::DeviceDraw => DEVICE_DRAW_NAMES,
            Self::Draw => DRAW_NAMES,
            Self::LayoutInit => LAYOUT_INIT_NAMES,
            Self::LayoutDeviceInit => LAYOUT_DEVICE_INIT_NAMES,
            Self::LayoutUpdate => LAYOUT_UPDATE_NAMES,
            Self::LayoutShutdown => LAYOUT_SHUTDOWN_NAMES,
        }
    }

    /// Returns whether one edition's build of this table carries a name.
    #[must_use]
    pub fn contains(self, edition: UiCallbackEdition, name: &str) -> bool {
        if !self.names().contains(&name) {
            return false;
        }
        edition == UiCallbackEdition::ZeroHour || !ZERO_HOUR_ONLY_NAMES.contains(&name)
    }
}

/// Which edition's function lexicon a name is looked up in.
///
/// The two editions compile separate copies of the same tables. Only the base-game side differs, and
/// only by omitting the names in [`ZERO_HOUR_ONLY_NAMES`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UiCallbackEdition {
    /// Base Command & Conquer Generals.
    Generals,
    /// Zero Hour, whose tables are a superset.
    #[default]
    ZeroHour,
}

/// The names Zero Hour's lexicon registers and base Generals' does not.
///
/// Five belong to Zero Hour's Generals Challenge menu, which base Generals has no screen for, and
/// `PopupHostGameUpdate` is an update entry Zero Hour added. No retail Generals layout names any of
/// them, so both editions' layouts classify identically today; a modded layout need not.
pub const ZERO_HOUR_ONLY_NAMES: [&str; 6] = [
    "ChallengeMenuInit",
    "ChallengeMenuInput",
    "ChallengeMenuShutdown",
    "ChallengeMenuSystem",
    "ChallengeMenuUpdate",
    "PopupHostGameUpdate",
];

/// Which callback record a retained name came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum UiCallbackSlot {
    /// A window's `SYSTEMCALLBACK`.
    System,
    /// A window's `INPUTCALLBACK`.
    Input,
    /// A window's `TOOLTIPCALLBACK`.
    Tooltip,
    /// A window's `DRAWCALLBACK`.
    Draw,
    /// A layout's `LAYOUTINIT`.
    LayoutInit,
    /// A layout's `LAYOUTUPDATE`.
    LayoutUpdate,
    /// A layout's `LAYOUTSHUTDOWN`.
    LayoutShutdown,
}

/// Every callback slot a WND record can fill, in record order.
pub const UI_CALLBACK_SLOTS: [UiCallbackSlot; 7] = [
    UiCallbackSlot::System,
    UiCallbackSlot::Input,
    UiCallbackSlot::Tooltip,
    UiCallbackSlot::Draw,
    UiCallbackSlot::LayoutInit,
    UiCallbackSlot::LayoutUpdate,
    UiCallbackSlot::LayoutShutdown,
];

impl UiCallbackSlot {
    /// Returns the stable report name for the slot.
    #[must_use]
    pub const fn row_name(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Input => "input",
            Self::Tooltip => "tooltip",
            Self::Draw => "draw",
            Self::LayoutInit => "layout_init",
            Self::LayoutUpdate => "layout_update",
            Self::LayoutShutdown => "layout_shutdown",
        }
    }

    /// Returns the WND record that fills the slot.
    #[must_use]
    pub const fn record_name(self) -> &'static str {
        match self {
            Self::System => "SYSTEMCALLBACK",
            Self::Input => "INPUTCALLBACK",
            Self::Tooltip => "TOOLTIPCALLBACK",
            Self::Draw => "DRAWCALLBACK",
            Self::LayoutInit => "LAYOUTINIT",
            Self::LayoutUpdate => "LAYOUTUPDATE",
            Self::LayoutShutdown => "LAYOUTSHUTDOWN",
        }
    }

    /// Returns the one table the slot's accessor searches by default.
    #[must_use]
    pub const fn own_table(self) -> UiCallbackTable {
        match self {
            Self::System => UiCallbackTable::System,
            Self::Input => UiCallbackTable::Input,
            Self::Tooltip => UiCallbackTable::Tooltip,
            Self::Draw => UiCallbackTable::Draw,
            Self::LayoutInit => UiCallbackTable::LayoutInit,
            Self::LayoutUpdate => UiCallbackTable::LayoutUpdate,
            Self::LayoutShutdown => UiCallbackTable::LayoutShutdown,
        }
    }

    /// Returns whether the slot's accessor searches every table rather than only its own.
    ///
    /// `gameWinDrawFunc` and `winLayoutInitFunc` default to `TABLE_ANY`, which is how a layout's
    /// `W3DMainMenuInit` and a control's `W3DGadgetPushButtonImageDraw` resolve at all: both live in
    /// device tables the pinned accessors would never look in. Every other slot is pinned, so the
    /// same spelling in the wrong record resolves to nothing.
    #[must_use]
    pub const fn searches_every_table(self) -> bool {
        matches!(self, Self::Draw | Self::LayoutInit)
    }
}

/// What a retained callback name resolves to.
///
/// Nothing here is a function: a [`UiCallbackBinding`] says whether the original would have found
/// native code behind the name, and a caller decides separately whether a typed action may run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiCallbackBinding {
    /// The authored `[None]` or `[NONE]` placeholder: the writer's explicit "no callback".
    ///
    /// The original turns this into a name key like any other, finds no table entry, and stores a
    /// null pointer, so it behaves exactly like an unknown name. It is distinguished here because it
    /// is deliberate rather than a compatibility gap.
    None,
    /// A name a searched table carries.
    Established {
        /// Which table it was found in. This is the slot's own table unless the slot searches every
        /// table and an earlier table matched first.
        table: UiCallbackTable,
    },
    /// A name no searched table carries. The original stores a null pointer and never calls it; this
    /// project reports it and leaves it inert.
    Unknown,
}

impl UiCallbackBinding {
    /// Returns the stable report name for the binding.
    #[must_use]
    pub const fn row_name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Established { .. } => "established",
            Self::Unknown => "unknown",
        }
    }

    /// Returns whether nothing at all would run for this name.
    #[must_use]
    pub const fn is_inert(self) -> bool {
        matches!(self, Self::None | Self::Unknown)
    }
}

/// Returns whether a retained name is the writer's explicit "nothing selected" placeholder.
///
/// Retail spells it both `[None]` and `[NONE]`, and the comparison here is case-insensitive for that
/// reason.
#[must_use]
pub fn is_none_callback(name: &str) -> bool {
    name.eq_ignore_ascii_case("[None]")
}

/// Classifies one retained callback name against Zero Hour's lexicon.
#[must_use]
pub fn classify_callback(slot: UiCallbackSlot, name: &str) -> UiCallbackBinding {
    classify_callback_in(UiCallbackEdition::ZeroHour, slot, name)
}

/// Classifies one retained callback name for the record that carried it, in one edition's lexicon.
///
/// The comparison is case-sensitive because `nameToKey` compares with `strcmp`: a name differing only
/// in case is a different key and finds no entry. A slot whose accessor searches every table walks
/// them in `TableIndex` order and takes the first match, exactly as `findFunction` does.
#[must_use]
pub fn classify_callback_in(
    edition: UiCallbackEdition,
    slot: UiCallbackSlot,
    name: &str,
) -> UiCallbackBinding {
    if is_none_callback(name) {
        return UiCallbackBinding::None;
    }
    if slot.searches_every_table() {
        for table in UI_CALLBACK_TABLES {
            if table.contains(edition, name) {
                return UiCallbackBinding::Established { table };
            }
        }
        return UiCallbackBinding::Unknown;
    }
    let table = slot.own_table();
    if table.contains(edition, name) {
        UiCallbackBinding::Established { table }
    } else {
        UiCallbackBinding::Unknown
    }
}

/// One typed action this project is willing to run in place of a source callback.
///
/// The original routes a button press into native menu code that may start a game, open a network
/// session, or quit. R4 is presentation-only, so the allowlist covers exactly the navigation and
/// presentation verbs a demo shell needs; anything else stays inert. A caller receives the action as
/// data and decides what to do with it, so nothing here reaches a renderer, a filesystem, or a
/// simulation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiDemoAction {
    /// Push a screen onto the shell stack.
    PushScreen {
        /// The layout's virtual path.
        path: String,
    },
    /// Pop the top screen off the shell stack.
    PopScreen,
    /// Show one control by decorated or undecorated name, which is how a retail menu reveals a
    /// subpanel it overlays.
    ShowControl {
        /// The control's name.
        control: String,
    },
    /// Hide one control by name.
    HideControl {
        /// The control's name.
        control: String,
    },
    /// Set the current transition group by name.
    SetTransitionGroup {
        /// The group's name.
        group: String,
    },
    /// Leave the demo. A caller decides what that means; nothing here exits a process.
    Quit,
}

/// A project-owned map from a control's authored name to the typed action it may run.
///
/// Keys are decorated `<layout>:<control>` names when a binding is layout-specific and undecorated
/// control names otherwise, matching what [`crate::UiLayout::find`] accepts. Iteration is ordered by
/// name so a report over an allowlist is stable.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UiActionAllowlist {
    entries: BTreeMap<String, UiDemoAction>,
}

impl UiActionAllowlist {
    /// Creates an empty allowlist, which routes nothing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Allows one control to run one action, replacing any earlier entry for that name.
    pub fn allow(&mut self, control: impl Into<String>, action: UiDemoAction) {
        self.entries.insert(control.into(), action);
    }

    /// Returns the action a control's name is allowed to run, if any.
    ///
    /// Both the decorated and undecorated spellings are tried, so an allowlist written against
    /// either resolves. The match is case-sensitive: an authored name is compared as spelled.
    #[must_use]
    pub fn resolve(&self, name: &str) -> Option<&UiDemoAction> {
        if let Some(action) = self.entries.get(name) {
            return Some(action);
        }
        let (_, control) = name.split_once(':')?;
        self.entries.get(control)
    }

    /// Returns every entry ordered by control name.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &UiDemoAction)> {
        self.entries
            .iter()
            .map(|(name, action)| (name.as_str(), action))
    }

    /// Returns how many controls the allowlist covers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether the allowlist routes nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// `gameWinSystemTable`.
const SYSTEM_NAMES: &[&str] = &[
    "PassSelectedButtonsToParentSystem",
    "PassMessagesToParentSystem",
    "GameWinDefaultSystem",
    "GadgetPushButtonSystem",
    "GadgetCheckBoxSystem",
    "GadgetRadioButtonSystem",
    "GadgetTabControlSystem",
    "GadgetListBoxSystem",
    "GadgetComboBoxSystem",
    "GadgetHorizontalSliderSystem",
    "GadgetVerticalSliderSystem",
    "GadgetProgressBarSystem",
    "GadgetStaticTextSystem",
    "GadgetTextEntrySystem",
    "MessageBoxSystem",
    "QuitMessageBoxSystem",
    "ExtendedMessageBoxSystem",
    "MOTDSystem",
    "MainMenuSystem",
    "OptionsMenuSystem",
    "SinglePlayerMenuSystem",
    "QuitMenuSystem",
    "MapSelectMenuSystem",
    "ReplayMenuSystem",
    "CreditsMenuSystem",
    "LanLobbyMenuSystem",
    "LanGameOptionsMenuSystem",
    "LanMapSelectMenuSystem",
    "SkirmishGameOptionsMenuSystem",
    "SkirmishMapSelectMenuSystem",
    "ChallengeMenuSystem",
    "SaveLoadMenuSystem",
    "PopupCommunicatorSystem",
    "PopupBuddyNotificationSystem",
    "PopupReplaySystem",
    "KeyboardOptionsMenuSystem",
    "WOLLadderScreenSystem",
    "WOLLoginMenuSystem",
    "WOLLocaleSelectSystem",
    "WOLLobbyMenuSystem",
    "WOLGameSetupMenuSystem",
    "WOLMapSelectMenuSystem",
    "WOLBuddyOverlaySystem",
    "WOLBuddyOverlayRCMenuSystem",
    "RCGameDetailsMenuSystem",
    "GameSpyPlayerInfoOverlaySystem",
    "WOLMessageWindowSystem",
    "WOLQuickMatchMenuSystem",
    "WOLWelcomeMenuSystem",
    "WOLStatusMenuSystem",
    "WOLQMScoreScreenSystem",
    "WOLCustomScoreScreenSystem",
    "NetworkDirectConnectSystem",
    "PopupHostGameSystem",
    "PopupJoinGameSystem",
    "PopupLadderSelectSystem",
    "InGamePopupMessageSystem",
    "ControlBarSystem",
    "ControlBarObserverSystem",
    "IMECandidateWindowSystem",
    "ReplayControlSystem",
    "InGameChatSystem",
    "DisconnectControlSystem",
    "DiplomacySystem",
    "GeneralsExpPointsSystem",
    "DifficultySelectSystem",
    "IdleWorkerSystem",
    "EstablishConnectionsControlSystem",
    "GameInfoWindowSystem",
    "ScoreScreenSystem",
    "DownloadMenuSystem",
];

/// `gameWinInputTable`.
const INPUT_NAMES: &[&str] = &[
    "GameWinDefaultInput",
    "GameWinBlockInput",
    "GadgetPushButtonInput",
    "GadgetCheckBoxInput",
    "GadgetRadioButtonInput",
    "GadgetTabControlInput",
    "GadgetListBoxInput",
    "GadgetListBoxMultiInput",
    "GadgetComboBoxInput",
    "GadgetHorizontalSliderInput",
    "GadgetVerticalSliderInput",
    "GadgetStaticTextInput",
    "GadgetTextEntryInput",
    "MainMenuInput",
    "MapSelectMenuInput",
    "OptionsMenuInput",
    "SinglePlayerMenuInput",
    "LanLobbyMenuInput",
    "ReplayMenuInput",
    "CreditsMenuInput",
    "KeyboardOptionsMenuInput",
    "PopupCommunicatorInput",
    "LanGameOptionsMenuInput",
    "LanMapSelectMenuInput",
    "SkirmishGameOptionsMenuInput",
    "SkirmishMapSelectMenuInput",
    "ChallengeMenuInput",
    "WOLLadderScreenInput",
    "WOLLoginMenuInput",
    "WOLLocaleSelectInput",
    "WOLLobbyMenuInput",
    "WOLGameSetupMenuInput",
    "WOLMapSelectMenuInput",
    "WOLBuddyOverlayInput",
    "GameSpyPlayerInfoOverlayInput",
    "WOLMessageWindowInput",
    "WOLQuickMatchMenuInput",
    "WOLWelcomeMenuInput",
    "WOLStatusMenuInput",
    "WOLQMScoreScreenInput",
    "WOLCustomScoreScreenInput",
    "NetworkDirectConnectInput",
    "PopupHostGameInput",
    "PopupJoinGameInput",
    "PopupLadderSelectInput",
    "InGamePopupMessageInput",
    "ControlBarInput",
    "ReplayControlInput",
    "InGameChatInput",
    "DisconnectControlInput",
    "DiplomacyInput",
    "EstablishConnectionsControlInput",
    "LeftHUDInput",
    "ScoreScreenInput",
    "SaveLoadMenuInput",
    "BeaconWindowInput",
    "DifficultySelectInput",
    "PopupReplayInput",
    "GeneralsExpPointsInput",
    "DownloadMenuInput",
    "IMECandidateWindowInput",
];

/// `gameWinTooltipTable`.
const TOOLTIP_NAMES: &[&str] = &["GameWinDefaultTooltip"];

/// `gameWinDrawTable` from the base lexicon.
const DRAW_NAMES: &[&str] = &["IMECandidateMainDraw", "IMECandidateTextAreaDraw"];

/// The W3D device's `gameWinDrawTable`, loaded as `TABLE_GAME_WIN_DEVICEDRAW`. Every gadget's colour
/// and image draw procedure lives here, which is why the draw slot has to search every table.
const DEVICE_DRAW_NAMES: &[&str] = &[
    "GameWinDefaultDraw",
    "W3DGameWinDefaultDraw",
    "W3DGadgetPushButtonDraw",
    "W3DGadgetPushButtonImageDraw",
    "W3DGadgetCheckBoxDraw",
    "W3DGadgetCheckBoxImageDraw",
    "W3DGadgetRadioButtonDraw",
    "W3DGadgetRadioButtonImageDraw",
    "W3DGadgetTabControlDraw",
    "W3DGadgetTabControlImageDraw",
    "W3DGadgetListBoxDraw",
    "W3DGadgetListBoxImageDraw",
    "W3DGadgetComboBoxDraw",
    "W3DGadgetComboBoxImageDraw",
    "W3DGadgetHorizontalSliderDraw",
    "W3DGadgetHorizontalSliderImageDraw",
    "W3DGadgetVerticalSliderDraw",
    "W3DGadgetVerticalSliderImageDraw",
    "W3DGadgetProgressBarDraw",
    "W3DGadgetProgressBarImageDraw",
    "W3DGadgetStaticTextDraw",
    "W3DGadgetStaticTextImageDraw",
    "W3DGadgetTextEntryDraw",
    "W3DGadgetTextEntryImageDraw",
    "W3DLeftHUDDraw",
    "W3DCameoMovieDraw",
    "W3DRightHUDDraw",
    "W3DPowerDraw",
    "W3DMainMenuDraw",
    "W3DMainMenuFourDraw",
    "W3DMetalBarMenuDraw",
    "W3DCreditsMenuDraw",
    "W3DClockDraw",
    "W3DMainMenuMapBorder",
    "W3DMainMenuButtonDropShadowDraw",
    "W3DMainMenuRandomTextDraw",
    "W3DThinBorderDraw",
    "W3DShellMenuSchemeDraw",
    "W3DCommandBarBackgroundDraw",
    "W3DCommandBarTopDraw",
    "W3DCommandBarGenExpDraw",
    "W3DCommandBarHelpPopupDraw",
    "W3DCommandBarGridDraw",
    "W3DCommandBarForegroundDraw",
    "W3DNoDraw",
    "W3DDrawMapPreview",
];

/// The W3D device's `layoutInitTable`, loaded as `TABLE_WIN_LAYOUT_DEVICEINIT`.
const LAYOUT_DEVICE_INIT_NAMES: &[&str] = &["W3DMainMenuInit"];

/// `winLayoutInitTable`.
const LAYOUT_INIT_NAMES: &[&str] = &[
    "MainMenuInit",
    "OptionsMenuInit",
    "SaveLoadMenuInit",
    "SaveLoadMenuFullScreenInit",
    "PopupCommunicatorInit",
    "KeyboardOptionsMenuInit",
    "SinglePlayerMenuInit",
    "MapSelectMenuInit",
    "LanLobbyMenuInit",
    "ReplayMenuInit",
    "CreditsMenuInit",
    "LanGameOptionsMenuInit",
    "LanMapSelectMenuInit",
    "SkirmishGameOptionsMenuInit",
    "SkirmishMapSelectMenuInit",
    "ChallengeMenuInit",
    "WOLLadderScreenInit",
    "WOLLoginMenuInit",
    "WOLLocaleSelectInit",
    "WOLLobbyMenuInit",
    "WOLGameSetupMenuInit",
    "WOLMapSelectMenuInit",
    "WOLBuddyOverlayInit",
    "WOLBuddyOverlayRCMenuInit",
    "RCGameDetailsMenuInit",
    "GameSpyPlayerInfoOverlayInit",
    "WOLMessageWindowInit",
    "WOLQuickMatchMenuInit",
    "WOLWelcomeMenuInit",
    "WOLStatusMenuInit",
    "WOLQMScoreScreenInit",
    "WOLCustomScoreScreenInit",
    "NetworkDirectConnectInit",
    "PopupHostGameInit",
    "PopupJoinGameInit",
    "PopupLadderSelectInit",
    "InGamePopupMessageInit",
    "GameInfoWindowInit",
    "ScoreScreenInit",
    "DownloadMenuInit",
    "DifficultySelectInit",
    "PopupReplayInit",
];

/// `winLayoutUpdateTable`.
const LAYOUT_UPDATE_NAMES: &[&str] = &[
    "MainMenuUpdate",
    "OptionsMenuUpdate",
    "SinglePlayerMenuUpdate",
    "MapSelectMenuUpdate",
    "LanLobbyMenuUpdate",
    "ReplayMenuUpdate",
    "SaveLoadMenuUpdate",
    "CreditsMenuUpdate",
    "LanGameOptionsMenuUpdate",
    "LanMapSelectMenuUpdate",
    "SkirmishGameOptionsMenuUpdate",
    "SkirmishMapSelectMenuUpdate",
    "ChallengeMenuUpdate",
    "WOLLadderScreenUpdate",
    "WOLLoginMenuUpdate",
    "WOLLocaleSelectUpdate",
    "WOLLobbyMenuUpdate",
    "WOLGameSetupMenuUpdate",
    "PopupHostGameUpdate",
    "WOLMapSelectMenuUpdate",
    "WOLBuddyOverlayUpdate",
    "GameSpyPlayerInfoOverlayUpdate",
    "WOLMessageWindowUpdate",
    "WOLQuickMatchMenuUpdate",
    "WOLWelcomeMenuUpdate",
    "WOLStatusMenuUpdate",
    "WOLQMScoreScreenUpdate",
    "WOLCustomScoreScreenUpdate",
    "NetworkDirectConnectUpdate",
    "ScoreScreenUpdate",
    "DownloadMenuUpdate",
    "PopupReplayUpdate",
];

/// `winLayoutShutdownTable`.
const LAYOUT_SHUTDOWN_NAMES: &[&str] = &[
    "MainMenuShutdown",
    "OptionsMenuShutdown",
    "SaveLoadMenuShutdown",
    "PopupCommunicatorShutdown",
    "KeyboardOptionsMenuShutdown",
    "SinglePlayerMenuShutdown",
    "MapSelectMenuShutdown",
    "LanLobbyMenuShutdown",
    "ReplayMenuShutdown",
    "CreditsMenuShutdown",
    "LanGameOptionsMenuShutdown",
    "LanMapSelectMenuShutdown",
    "SkirmishGameOptionsMenuShutdown",
    "SkirmishMapSelectMenuShutdown",
    "ChallengeMenuShutdown",
    "WOLLadderScreenShutdown",
    "WOLLoginMenuShutdown",
    "WOLLocaleSelectShutdown",
    "WOLLobbyMenuShutdown",
    "WOLGameSetupMenuShutdown",
    "WOLMapSelectMenuShutdown",
    "WOLBuddyOverlayShutdown",
    "GameSpyPlayerInfoOverlayShutdown",
    "WOLMessageWindowShutdown",
    "WOLQuickMatchMenuShutdown",
    "WOLWelcomeMenuShutdown",
    "WOLStatusMenuShutdown",
    "WOLQMScoreScreenShutdown",
    "WOLCustomScoreScreenShutdown",
    "NetworkDirectConnectShutdown",
    "ScoreScreenShutdown",
    "DownloadMenuShutdown",
    "PopupReplayShutdown",
];
