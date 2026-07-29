//! What a control is allowed to ask the engine to do.
//!
//! # Why this is a closed enum and not a string
//!
//! A layout file is data, and the layering rule this crate exists under is that data must not be able
//! to name an action the engine did not define. The obvious implementation — a handler name looked up
//! in a table at activation time — fails in two ways that matter. It defers a typo from load to the
//! moment a user clicks, which is the worst possible time to discover it; and once mods can supply
//! layouts, a string is an open channel into whatever the lookup table happens to contain.
//!
//! An enum closes both. `serde` refuses a variant it does not know, so an unrecognised action is a
//! load error naming the file, and the set of things a layout can trigger is fixed at compile time by
//! construction rather than by review.
//!
//! # Why it is not generic over the host
//!
//! A parameter would let a caller substitute its own action type and get the open set back by another
//! route. The cost of the closed version is that adding a screen means adding a variant here, which is
//! a deliberate edit in one place — exactly the property being bought.

use serde::{Deserialize, Serialize};

/// Every effect a layout file may attach to a control.
///
/// Variants are named for intent rather than for the widget that raises them, because the same effect
/// is reachable from a button, a menu entry, and a key binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Action {
    /// Leave the game.
    Quit,
    /// Return to the previous screen, popping the stack.
    Back,
    /// Show the main menu.
    OpenMainMenu,
    /// Show the settings screen.
    OpenSettings,
    /// Show skirmish setup.
    OpenSkirmishSetup,
    /// Ask whether to leave the game.
    ///
    /// Distinct from [`Self::Quit`] so a main menu's exit button can be the question and the modal's
    /// button can be the answer. One action meaning "ask" on one screen and "do it" on another is
    /// exactly the context-dependence this set exists to avoid.
    OpenQuitConfirm,
    /// Commit the settings currently staged.
    ///
    /// Distinct from [`Self::ConfirmSettings`] because a display change is applied first and
    /// confirmed second — see [`crate::layout`] for why that ordering is not optional.
    ApplySettings,
    /// Keep the applied settings permanently, ending the revert window.
    ConfirmSettings,
    /// Discard staged settings and restore what was in force.
    RevertSettings,
    /// Start the configured skirmish.
    LaunchSkirmish,
}

impl Action {
    /// Whether this action leaves the current screen.
    ///
    /// Used by the screen stack to decide whether retained state — a scroll offset, a text cursor —
    /// is still wanted after the action runs.
    #[must_use]
    pub const fn leaves_screen(self) -> bool {
        matches!(
            self,
            Self::Quit
                | Self::Back
                | Self::OpenMainMenu
                | Self::OpenSettings
                | Self::OpenSkirmishSetup
                | Self::OpenQuitConfirm
                | Self::LaunchSkirmish
        )
    }
}

#[cfg(test)]
mod tests {
    use super::Action;

    #[test]
    fn an_action_round_trips_as_snake_case() {
        let encoded = serde_json::to_string(&Action::OpenSkirmishSetup).expect("encode");
        assert_eq!(encoded, "\"open_skirmish_setup\"");
        let decoded: Action = serde_json::from_str(&encoded).expect("decode");
        assert_eq!(decoded, Action::OpenSkirmishSetup);
    }

    #[test]
    fn an_action_the_engine_does_not_define_is_refused() {
        // The whole point of the enum. A layout naming `format_hard_drive` must fail to load, not
        // fail to find a handler later.
        let refused = serde_json::from_str::<Action>("\"format_hard_drive\"");
        assert!(refused.is_err(), "an unknown action must not deserialize");
    }

    #[test]
    fn applying_settings_does_not_leave_the_screen_but_navigating_does() {
        // Apply and revert have to stay on the settings screen, or the revert timer would be armed
        // somewhere the user cannot reach the confirm button.
        assert!(!Action::ApplySettings.leaves_screen());
        assert!(!Action::ConfirmSettings.leaves_screen());
        assert!(!Action::RevertSettings.leaves_screen());
        assert!(Action::Back.leaves_screen());
        assert!(Action::OpenSettings.leaves_screen());
    }
}
