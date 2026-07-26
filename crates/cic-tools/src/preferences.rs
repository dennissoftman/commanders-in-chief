// Commanders in Chief
// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only
//
// Provenance: project design. The original persists display choices through
// `Core/GameEngine/Source/Common/OptionPreferences.cpp`, whose `getResolution`/`setResolution` write
// a `Resolution` pair and a `Windowed` boolean into the user's `Options.ini`. This project never
// writes to that file — it is user-owned data and the settings here have no representation in it —
// so confirmed choices go to a project-owned file beside the existing `config`. What is kept from
// the original is only the shape of the decision: a resolution and a window mode are the two things
// worth remembering across sessions.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use cic_ui::{UiDisplayOutcome, UiDisplaySelection, UiScaleChoice, UiWindowMode};

use crate::resource::{ResourceError, config_path};

/// Where confirmed display preferences are written.
///
/// Beside the existing `config` rather than inside it: that file records where the user's
/// installations are, which is machine setup, while this records what the player chose, which is
/// theirs. Mixing them would mean a display change rewrites the file that knows where the game is.
///
/// # Errors
///
/// Returns an error when no platform configuration directory can be determined.
pub fn display_preferences_path() -> Result<PathBuf, ResourceError> {
    let config = config_path()?;
    let parent = config
        .parent()
        .ok_or_else(|| ResourceError::InvalidConfigPath(config.clone()))?;
    Ok(parent.join("display"))
}

/// A confirmed display choice, as persisted.
///
/// Every field is stored in the spelling its own type round-trips through, so the file is legible
/// and a hand edit that names something the menu could never offer is refused on load rather than
/// silently carried into a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayPreferences {
    /// The monitor's per-session key.
    ///
    /// Keys are not stable across sessions — monitor identity is a platform problem this project
    /// does not solve — so a key naming no present monitor falls back to the default rather than
    /// failing. It is recorded because on an unchanged machine it does match, and matching is worth
    /// having.
    pub monitor: String,
    /// How the window presents itself.
    pub window_mode: UiWindowMode,
    /// The client or mode width.
    pub width: u32,
    /// The client or mode height.
    pub height: u32,
    /// The refresh rate in millihertz. Only meaningful for an exclusive-fullscreen preference.
    pub refresh_millihertz: u32,
    /// The UI scale.
    pub scale: UiScaleChoice,
}

impl DisplayPreferences {
    /// Records a selection that has been accepted.
    #[must_use]
    pub fn from_selection(selection: &UiDisplaySelection) -> Self {
        Self {
            monitor: selection.monitor.clone(),
            window_mode: selection.window_mode,
            width: selection.resolution.0,
            height: selection.resolution.1,
            refresh_millihertz: selection.refresh_millihertz,
            scale: selection.scale,
        }
    }

    /// Returns the selection to request at startup.
    ///
    /// The result still has to be resolved against the live catalog before it is applied: the
    /// monitor may be gone and the mode may no longer be advertised, and both are the settings
    /// model's decision rather than this file's.
    #[must_use]
    pub fn to_selection(&self) -> UiDisplaySelection {
        UiDisplaySelection {
            monitor: self.monitor.clone(),
            window_mode: self.window_mode,
            resolution: (self.width, self.height),
            refresh_millihertz: self.refresh_millihertz,
            scale: self.scale,
        }
    }

    /// Reads the preferences, returning `None` when none have been written.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read, or carries an unknown record, a malformed
    /// line, or a value outside what the settings model can offer.
    pub fn load(path: &Path) -> Result<Option<Self>, ResourceError> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(ResourceError::Io {
                    path: path.to_path_buf(),
                    error,
                });
            }
        };
        let mut monitor = String::new();
        let mut window_mode = UiWindowMode::default();
        let mut width = 0;
        let mut height = 0;
        let mut refresh_millihertz = 0;
        let mut scale = UiScaleChoice::default();
        for (index, line) in text.lines().enumerate() {
            let invalid = || ResourceError::InvalidConfig { line: index + 1 };
            let Some((key, value)) = line.split_once('=') else {
                return Err(invalid());
            };
            match key {
                "version" if value == "1" => {}
                "monitor" => value.clone_into(&mut monitor),
                "window_mode" => {
                    window_mode = UiWindowMode::from_row_name(value).ok_or_else(invalid)?;
                }
                "width" => width = value.parse::<u32>().map_err(|_| invalid())?,
                "height" => height = value.parse::<u32>().map_err(|_| invalid())?,
                "refresh_millihertz" => {
                    refresh_millihertz = value.parse::<u32>().map_err(|_| invalid())?;
                }
                "ui_scale" => scale = UiScaleChoice::from_row_name(value).ok_or_else(invalid)?,
                _ => return Err(invalid()),
            }
        }
        Ok(Some(Self {
            monitor,
            window_mode,
            width,
            height,
            refresh_millihertz,
            scale,
        }))
    }

    /// Writes the preferences, creating the configuration directory if it does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error when the directory cannot be created or the file cannot be written.
    pub fn save(&self, path: &Path) -> Result<(), ResourceError> {
        let parent = path
            .parent()
            .ok_or_else(|| ResourceError::InvalidConfigPath(path.to_path_buf()))?;
        fs::create_dir_all(parent).map_err(|error| ResourceError::Io {
            path: parent.to_path_buf(),
            error,
        })?;
        let text = format!(
            "version=1\nmonitor={}\nwindow_mode={}\nwidth={}\nheight={}\n\
             refresh_millihertz={}\nui_scale={}\n",
            self.monitor,
            self.window_mode.row_name(),
            self.width,
            self.height,
            self.refresh_millihertz,
            self.scale.row_name()
        );
        fs::write(path, text).map_err(|error| ResourceError::Io {
            path: path.to_path_buf(),
            error,
        })
    }
}

/// Writes preferences if — and only if — a display change was confirmed.
///
/// The rule that a rolled-back mode must not be persisted is the whole point of the transaction, so
/// it is enforced here rather than left to each caller to remember. Every other outcome writes
/// nothing and returns `false`, including a timeout, a decline, and a platform failure.
///
/// # Errors
///
/// Returns an error when the file cannot be written.
pub fn persist_confirmed_display(
    outcome: &UiDisplayOutcome,
    path: &Path,
) -> Result<bool, ResourceError> {
    let UiDisplayOutcome::Confirmed { accepted } = outcome else {
        return Ok(false);
    };
    DisplayPreferences::from_selection(accepted).save(path)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::env;

    use cic_ui::{
        UiDisplayOutcome, UiDisplaySelection, UiRollbackReason, UiScaleChoice, UiWindowMode,
    };

    use super::{DisplayPreferences, persist_confirmed_display};

    fn selection() -> UiDisplaySelection {
        UiDisplaySelection {
            monitor: "DISPLAY1".to_owned(),
            window_mode: UiWindowMode::ExclusiveFullscreen,
            resolution: (2560, 1080),
            refresh_millihertz: 144_000,
            scale: UiScaleChoice::Fixed(125),
        }
    }

    /// A scratch path unique to one test, so the suite may run its tests in parallel.
    fn scratch(name: &str) -> std::path::PathBuf {
        env::temp_dir().join(format!("cic-display-preferences-{name}"))
    }

    #[test]
    fn preferences_round_trip_and_reject_a_value_no_menu_offers() {
        let path = scratch("round-trip");
        let _ = std::fs::remove_file(&path);
        // Absent preferences are not an error: a first run simply has none.
        assert_eq!(DisplayPreferences::load(&path).expect("absent"), None);

        let preferences = DisplayPreferences::from_selection(&selection());
        preferences.save(&path).expect("write preferences");
        assert_eq!(
            DisplayPreferences::load(&path).expect("read back"),
            Some(preferences.clone())
        );
        // What comes back is the selection that went in, ready to resolve against a live catalog.
        assert_eq!(preferences.to_selection(), selection());

        // A hand edit naming a scale no step offers is refused rather than carried into a session.
        std::fs::write(
            &path,
            "version=1\nmonitor=DISPLAY1\nwindow_mode=exclusive\nwidth=2560\nheight=1080\n\
             refresh_millihertz=144000\nui_scale=133\n",
        )
        .expect("write a hand-edited file");
        assert!(DisplayPreferences::load(&path).is_err());

        // So is an unknown window mode and an unknown record.
        std::fs::write(&path, "version=1\nwindow_mode=fullscreen\n").expect("write");
        assert!(DisplayPreferences::load(&path).is_err());
        std::fs::write(&path, "version=1\nvsync=on\n").expect("write");
        assert!(DisplayPreferences::load(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn only_a_confirmed_outcome_is_ever_written() {
        let path = scratch("confirmed-only");
        let _ = std::fs::remove_file(&path);
        let restored = Box::new(selection());

        // Every outcome that is not a confirmation writes nothing at all — that rule is the whole
        // point of the transaction, so it is enforced here rather than left to each caller.
        for outcome in [
            UiDisplayOutcome::Idle,
            UiDisplayOutcome::AwaitingConfirmation { deadline_ms: 1 },
            UiDisplayOutcome::RolledBack {
                reason: UiRollbackReason::TimedOut { elapsed_ms: 15_001 },
                restored: restored.clone(),
            },
            UiDisplayOutcome::RolledBack {
                reason: UiRollbackReason::Declined,
                restored: restored.clone(),
            },
            UiDisplayOutcome::RolledBack {
                reason: UiRollbackReason::Failed {
                    detail: "surface lost".to_owned(),
                },
                restored,
            },
        ] {
            assert!(
                !persist_confirmed_display(&outcome, &path).expect("no write"),
                "{outcome:?} must not persist"
            );
            assert!(!path.exists(), "{outcome:?} must leave no file");
        }

        let confirmed = UiDisplayOutcome::Confirmed {
            accepted: Box::new(selection()),
        };
        assert!(persist_confirmed_display(&confirmed, &path).expect("write"));
        assert_eq!(
            DisplayPreferences::load(&path).expect("read back"),
            Some(DisplayPreferences::from_selection(&selection()))
        );
        let _ = std::fs::remove_file(&path);
    }
}
