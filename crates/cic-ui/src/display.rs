// Commanders in Chief
// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only
//
// Provenance: this is project design, not a source-derived algorithm. Electronic Arts' GPL-3.0
// source release, GeneralsGameCode revision 9f7abb866f5afd446db14149979e744c7216baaf, establishes
// only what the original could express: `Core/GameEngine/Include/GameClient/Display.h`'s
// `setDisplayMode( width, height, bitDepth, windowed )` and its `DisplayModeInterface` enumeration,
// which carries width, height, and bit depth and no refresh rate at all, plus
// `Core/GameEngine/Source/Common/OptionPreferences.cpp`'s `getResolution`/`setResolution` and the
// `Windowed` preference. The original therefore has no monitor selector, no borderless mode, no
// refresh selection, and no UI scale independent of render resolution. Everything those four need
// is defined here rather than reproduced, and R4 adds them through a bounded WND patch so no
// user-owned bytes change. What is preserved from the original is the shape of the resolution
// choice — an advertised mode list the player picks from — and the fact that a failed mode change
// must leave the previous mode in place.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

/// How many monitors a catalog may carry.
///
/// A platform adapter enumerates these, so the bound exists to keep a misbehaving or hostile backend
/// from producing an unbounded catalog, not because any real machine approaches it.
pub const UI_MAX_MONITORS: usize = 64;

/// How many video modes a catalog may carry across all monitors.
pub const UI_MAX_DISPLAY_MODES: usize = 4_096;

/// The lowest client size the settings model will offer or accept.
///
/// Below this a menu laid out at the original's 800x600 creation resolution cannot be presented at
/// all under either scale policy, so offering it would produce a window nothing can be read in.
pub const UI_MIN_CLIENT_WIDTH: u32 = 640;
/// The lowest client height the settings model will offer or accept.
pub const UI_MIN_CLIENT_HEIGHT: u32 = 480;

/// One monitor a platform adapter reported.
///
/// `key` is a stable per-session identity, not a persistent one: monitor identity across sessions is
/// a platform concern this project does not try to solve, and a preference naming a monitor that is
/// no longer present falls back rather than failing.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct UiMonitor {
    key: String,
    name: String,
    source_index: usize,
}

impl UiMonitor {
    /// Records one monitor.
    #[must_use]
    pub fn new(key: impl Into<String>, name: impl Into<String>, source_index: usize) -> Self {
        Self {
            key: key.into(),
            name: name.into(),
            source_index,
        }
    }

    /// Returns the stable per-session key modes are grouped by.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Returns the human-readable name, which a menu shows and nothing keys on.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the adapter's own enumeration index, which breaks ties deterministically.
    #[must_use]
    pub const fn source_index(&self) -> usize {
        self.source_index
    }
}

/// One video mode a platform adapter advertised for one monitor.
///
/// Refresh is stored in millihertz exactly as the platform reports it, and is never rounded to whole
/// hertz: 59.94 Hz and 60 Hz are different advertised modes on the same monitor, and a menu that
/// collapsed them would offer a pair exclusive fullscreen cannot select.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct UiVideoMode {
    monitor: String,
    width: u32,
    height: u32,
    refresh_millihertz: u32,
    bit_depth: Option<u16>,
    source_index: usize,
}

impl UiVideoMode {
    /// Records one advertised mode.
    #[must_use]
    pub fn new(
        monitor: impl Into<String>,
        width: u32,
        height: u32,
        refresh_millihertz: u32,
        bit_depth: Option<u16>,
        source_index: usize,
    ) -> Self {
        Self {
            monitor: monitor.into(),
            width,
            height,
            refresh_millihertz,
            bit_depth,
            source_index,
        }
    }

    /// Returns the key of the monitor that advertised it.
    #[must_use]
    pub fn monitor(&self) -> &str {
        &self.monitor
    }

    /// Returns the mode's width in physical pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the mode's height in physical pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Returns the refresh rate in millihertz, exactly as advertised.
    #[must_use]
    pub const fn refresh_millihertz(&self) -> u32 {
        self.refresh_millihertz
    }

    /// Returns the bit depth, absent when the backend does not report one.
    #[must_use]
    pub const fn bit_depth(&self) -> Option<u16> {
        self.bit_depth
    }

    /// Returns the adapter's own enumeration index, which breaks ties deterministically.
    #[must_use]
    pub const fn source_index(&self) -> usize {
        self.source_index
    }

    /// Returns the mode's resolution.
    #[must_use]
    pub const fn resolution(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Returns whether the mode is large enough to present a menu in.
    #[must_use]
    pub const fn is_presentable(&self) -> bool {
        self.width >= UI_MIN_CLIENT_WIDTH && self.height >= UI_MIN_CLIENT_HEIGHT
    }
}

/// Why a catalog could not be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiDisplayError {
    /// More monitors than [`UI_MAX_MONITORS`].
    TooManyMonitors {
        /// The bound.
        limit: usize,
    },
    /// More modes than [`UI_MAX_DISPLAY_MODES`].
    TooManyModes {
        /// The bound.
        limit: usize,
    },
    /// Two monitors reported the same key, so modes could not be attributed to one of them.
    DuplicateMonitorKey {
        /// The repeated key.
        key: Box<str>,
    },
    /// A monitor reported an empty key, which nothing can be grouped by.
    EmptyMonitorKey {
        /// Its enumeration index.
        source_index: usize,
    },
    /// A mode named a monitor the catalog does not carry.
    UnknownMonitor {
        /// The key it named.
        key: Box<str>,
    },
    /// A mode reported a zero width or height.
    DegenerateMode {
        /// Its enumeration index.
        source_index: usize,
    },
}

impl Display for UiDisplayError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyMonitors { limit } => {
                write!(formatter, "more than {limit} monitors were reported")
            }
            Self::TooManyModes { limit } => {
                write!(formatter, "more than {limit} video modes were reported")
            }
            Self::DuplicateMonitorKey { key } => {
                write!(formatter, "two monitors reported the key {key:?}")
            }
            Self::EmptyMonitorKey { source_index } => {
                write!(formatter, "monitor {source_index} reported an empty key")
            }
            Self::UnknownMonitor { key } => {
                write!(formatter, "a video mode named unknown monitor {key:?}")
            }
            Self::DegenerateMode { source_index } => {
                write!(formatter, "video mode {source_index} has a zero dimension")
            }
        }
    }
}

impl Error for UiDisplayError {}

/// Something a catalog could not offer, reported rather than worked around.
///
/// A backend that advertises no selectable modes, or modes with no refresh rate, is a real case on
/// some platforms. The corresponding control is disabled and one of these explains why, which is the
/// alternative to fabricating a plausible value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiDisplayCapability {
    /// A monitor advertised no video modes at all.
    NoModes {
        /// The monitor's key.
        monitor: String,
    },
    /// A monitor advertised modes but none carried a refresh rate.
    NoRefreshRates {
        /// The monitor's key.
        monitor: String,
    },
    /// A monitor's advertised modes were all smaller than the presentable minimum.
    NoPresentableModes {
        /// The monitor's key.
        monitor: String,
    },
}

impl UiDisplayCapability {
    /// Returns a stable name for a report.
    #[must_use]
    pub const fn row_name(&self) -> &'static str {
        match self {
            Self::NoModes { .. } => "no_modes",
            Self::NoRefreshRates { .. } => "no_refresh_rates",
            Self::NoPresentableModes { .. } => "no_presentable_modes",
        }
    }

    /// Returns the monitor the capability gap belongs to.
    #[must_use]
    pub fn monitor(&self) -> &str {
        match self {
            Self::NoModes { monitor }
            | Self::NoRefreshRates { monitor }
            | Self::NoPresentableModes { monitor } => monitor,
        }
    }
}

/// An immutable, deterministically ordered set of monitors and the modes each advertises.
///
/// Ordering is fixed at construction and never depends on the adapter's enumeration order alone:
/// monitors sort by key then source index, and modes sort by monitor key, width, height, refresh
/// millihertz, bit depth, then source index. Two runs against the same hardware therefore produce
/// the same lists in the same order, which is what lets a capture pin a selection by index.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UiDisplayCatalog {
    monitors: Vec<UiMonitor>,
    modes: Vec<UiVideoMode>,
    capabilities: Vec<UiDisplayCapability>,
}

impl UiDisplayCatalog {
    /// Builds a catalog from what an adapter reported, sorting and deduplicating it.
    ///
    /// Exact duplicates are dropped: some backends advertise the same mode more than once, and a menu
    /// showing it twice would let a player pick between two identical entries.
    ///
    /// # Errors
    ///
    /// Returns a structured error for an exceeded bound, a duplicate or empty monitor key, a mode
    /// naming an unknown monitor, or a mode with a zero dimension.
    pub fn new(monitors: Vec<UiMonitor>, modes: Vec<UiVideoMode>) -> Result<Self, UiDisplayError> {
        if monitors.len() > UI_MAX_MONITORS {
            return Err(UiDisplayError::TooManyMonitors {
                limit: UI_MAX_MONITORS,
            });
        }
        if modes.len() > UI_MAX_DISPLAY_MODES {
            return Err(UiDisplayError::TooManyModes {
                limit: UI_MAX_DISPLAY_MODES,
            });
        }
        let mut keys = BTreeSet::new();
        for monitor in &monitors {
            if monitor.key.is_empty() {
                return Err(UiDisplayError::EmptyMonitorKey {
                    source_index: monitor.source_index,
                });
            }
            if !keys.insert(monitor.key.clone()) {
                return Err(UiDisplayError::DuplicateMonitorKey {
                    key: monitor.key.clone().into_boxed_str(),
                });
            }
        }
        for mode in &modes {
            if !keys.contains(&mode.monitor) {
                return Err(UiDisplayError::UnknownMonitor {
                    key: mode.monitor.clone().into_boxed_str(),
                });
            }
            if mode.width == 0 || mode.height == 0 {
                return Err(UiDisplayError::DegenerateMode {
                    source_index: mode.source_index,
                });
            }
        }

        let mut monitors = monitors;
        monitors.sort_by(|left, right| {
            left.key
                .cmp(&right.key)
                .then(left.source_index.cmp(&right.source_index))
        });
        let mut modes = modes;
        modes.sort_by(|left, right| {
            left.monitor
                .cmp(&right.monitor)
                .then(left.width.cmp(&right.width))
                .then(left.height.cmp(&right.height))
                .then(left.refresh_millihertz.cmp(&right.refresh_millihertz))
                .then(left.bit_depth.cmp(&right.bit_depth))
                .then(left.source_index.cmp(&right.source_index))
        });
        // Only the source index may differ between two otherwise identical entries, so comparing
        // everything else is what makes a re-advertised mode a duplicate.
        modes.dedup_by(|left, right| {
            left.monitor == right.monitor
                && left.width == right.width
                && left.height == right.height
                && left.refresh_millihertz == right.refresh_millihertz
                && left.bit_depth == right.bit_depth
        });

        let capabilities = monitors
            .iter()
            .filter_map(|monitor| {
                let own: Vec<&UiVideoMode> = modes
                    .iter()
                    .filter(|mode| mode.monitor == monitor.key)
                    .collect();
                if own.is_empty() {
                    return Some(UiDisplayCapability::NoModes {
                        monitor: monitor.key.clone(),
                    });
                }
                if !own.iter().any(|mode| mode.is_presentable()) {
                    return Some(UiDisplayCapability::NoPresentableModes {
                        monitor: monitor.key.clone(),
                    });
                }
                if own.iter().all(|mode| mode.refresh_millihertz == 0) {
                    return Some(UiDisplayCapability::NoRefreshRates {
                        monitor: monitor.key.clone(),
                    });
                }
                None
            })
            .collect();

        Ok(Self {
            monitors,
            modes,
            capabilities,
        })
    }

    /// Returns every monitor, ordered.
    #[must_use]
    pub fn monitors(&self) -> &[UiMonitor] {
        &self.monitors
    }

    /// Returns every mode across every monitor, ordered.
    #[must_use]
    pub fn modes(&self) -> &[UiVideoMode] {
        &self.modes
    }

    /// Returns everything the catalog could not offer.
    #[must_use]
    pub fn capabilities(&self) -> &[UiDisplayCapability] {
        &self.capabilities
    }

    /// Returns whether the catalog knows a monitor.
    #[must_use]
    pub fn has_monitor(&self, key: &str) -> bool {
        self.monitors.iter().any(|monitor| monitor.key == key)
    }

    /// Returns the first monitor, which is the fallback when a preference names none that exists.
    #[must_use]
    pub fn default_monitor(&self) -> Option<&UiMonitor> {
        self.monitors.first()
    }

    /// Returns every presentable resolution one monitor advertises, ordered and without repeats.
    ///
    /// Resolutions repeat across refresh rates, so this is what a resolution control is populated
    /// from — the refresh control is then populated from whichever resolution is chosen.
    #[must_use]
    pub fn resolutions(&self, monitor: &str) -> Vec<(u32, u32)> {
        let mut seen = Vec::new();
        for mode in &self.modes {
            if mode.monitor != monitor || !mode.is_presentable() {
                continue;
            }
            let resolution = mode.resolution();
            if !seen.contains(&resolution) {
                seen.push(resolution);
            }
        }
        seen
    }

    /// Returns every refresh rate one monitor advertises at one resolution, ordered and unique.
    ///
    /// A zero is a backend that reported no refresh for that mode and is never offered as a choice.
    #[must_use]
    pub fn refresh_rates(&self, monitor: &str, resolution: (u32, u32)) -> Vec<u32> {
        let mut seen = Vec::new();
        for mode in &self.modes {
            if mode.monitor != monitor
                || mode.resolution() != resolution
                || mode.refresh_millihertz == 0
            {
                continue;
            }
            if !seen.contains(&mode.refresh_millihertz) {
                seen.push(mode.refresh_millihertz);
            }
        }
        seen
    }

    /// Returns the monitor's desktop mode, which borderless and windowed report rather than select.
    ///
    /// A platform adapter marks it by enumerating it first for that monitor; with nothing to go on,
    /// the largest advertised mode at the highest refresh is the best available answer and is what a
    /// desktop almost always is.
    #[must_use]
    pub fn desktop_mode(&self, monitor: &str) -> Option<&UiVideoMode> {
        self.modes
            .iter()
            .filter(|mode| mode.monitor == monitor)
            .max_by_key(|mode| {
                (
                    mode.width,
                    mode.height,
                    mode.refresh_millihertz,
                    mode.bit_depth,
                )
            })
    }

    /// Returns the exact advertised mode a monitor, resolution, and refresh name.
    #[must_use]
    pub fn exact_mode(
        &self,
        monitor: &str,
        resolution: (u32, u32),
        refresh_millihertz: u32,
    ) -> Option<&UiVideoMode> {
        self.modes.iter().find(|mode| {
            mode.monitor == monitor
                && mode.resolution() == resolution
                && mode.refresh_millihertz == refresh_millihertz
        })
    }
}

/// How a window presents itself.
///
/// The original has one boolean, `windowed`, and `setDisplayMode` takes a bit depth beside it. These
/// three are project design, and they differ in what the refresh rate *means* rather than only in
/// how the window is decorated, which is why they are one enumeration and not two booleans.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UiWindowMode {
    /// A decorated window of an explicit client size. Refresh follows the desktop and is reported,
    /// never selected.
    #[default]
    Windowed,
    /// A borderless window covering the selected monitor. Resolution and refresh are the monitor's
    /// desktop mode; neither is selectable.
    BorderlessDesktop,
    /// An exclusive-fullscreen mode set. Resolution and refresh must together name one advertised
    /// mode, and both are selectable.
    ExclusiveFullscreen,
}

impl UiWindowMode {
    /// Returns a stable name for a report or a preference file.
    #[must_use]
    pub const fn row_name(self) -> &'static str {
        match self {
            Self::Windowed => "windowed",
            Self::BorderlessDesktop => "borderless",
            Self::ExclusiveFullscreen => "exclusive",
        }
    }

    /// Parses the name [`UiWindowMode::row_name`] produces.
    #[must_use]
    pub fn from_row_name(name: &str) -> Option<Self> {
        match name {
            "windowed" => Some(Self::Windowed),
            "borderless" => Some(Self::BorderlessDesktop),
            "exclusive" => Some(Self::ExclusiveFullscreen),
            _ => None,
        }
    }

    /// Returns whether the player chooses the resolution in this mode.
    ///
    /// Borderless takes the desktop's, so its resolution control is disabled.
    #[must_use]
    pub const fn selects_resolution(self) -> bool {
        !matches!(self, Self::BorderlessDesktop)
    }

    /// Returns whether the player chooses the refresh rate in this mode.
    ///
    /// Only an exclusive mode set actually programmes a refresh rate. Windowed and borderless both
    /// present at whatever the desktop is doing, so offering a choice there would be a lie.
    #[must_use]
    pub const fn selects_refresh(self) -> bool {
        matches!(self, Self::ExclusiveFullscreen)
    }
}

/// How the UI is scaled, independent of render resolution.
///
/// The original has nothing like this: its menus are laid out at a creation resolution and stretched
/// by `parseScreenRect`. `Automatic` is the project's `Modern` uniform-scale policy left to choose
/// its own factor; the fixed steps let a player override it on a display where it reads too small or
/// too large.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UiScaleChoice {
    /// Let the scale policy choose.
    #[default]
    Automatic,
    /// A fixed percentage of the policy's own scale, in whole percent.
    Fixed(u16),
}

impl UiScaleChoice {
    /// The fixed steps a menu offers, in whole percent.
    pub const STEPS: [u16; 6] = [75, 100, 125, 150, 175, 200];

    /// Returns a stable name for a report or a preference file.
    #[must_use]
    pub fn row_name(self) -> String {
        match self {
            Self::Automatic => "automatic".to_owned(),
            Self::Fixed(percent) => percent.to_string(),
        }
    }

    /// Parses the name [`UiScaleChoice::row_name`] produces, rejecting a step nothing offers.
    #[must_use]
    pub fn from_row_name(name: &str) -> Option<Self> {
        if name == "automatic" {
            return Some(Self::Automatic);
        }
        let percent = name.parse::<u16>().ok()?;
        Self::STEPS
            .contains(&percent)
            .then_some(Self::Fixed(percent))
    }
}

/// One complete display choice, before it has been applied or accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiDisplaySelection {
    /// The monitor's key.
    pub monitor: String,
    /// How the window presents itself.
    pub window_mode: UiWindowMode,
    /// The client or mode size.
    pub resolution: (u32, u32),
    /// The refresh rate in millihertz. Meaningful only in exclusive fullscreen; in the other two
    /// modes it carries the desktop's rate so a report can state what is actually being presented.
    pub refresh_millihertz: u32,
    /// The UI scale.
    pub scale: UiScaleChoice,
}

/// Why a selection cannot be applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiSelectionError {
    /// The catalog carries no such monitor.
    UnknownMonitor {
        /// The key that was named.
        key: Box<str>,
    },
    /// The monitor does not advertise that resolution.
    UnknownResolution {
        /// The width that was named.
        width: u32,
        /// The height that was named.
        height: u32,
    },
    /// The monitor advertises the resolution but not with that refresh rate.
    ///
    /// Only exclusive fullscreen can raise this: it is the one mode that names an advertised pair.
    UnknownRefreshRate {
        /// The rate that was named, in millihertz.
        refresh_millihertz: u32,
    },
    /// The resolution is below the presentable minimum.
    NotPresentable {
        /// The width that was named.
        width: u32,
        /// The height that was named.
        height: u32,
    },
}

impl Display for UiSelectionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownMonitor { key } => write!(formatter, "no monitor {key:?}"),
            Self::UnknownResolution { width, height } => {
                write!(formatter, "no advertised mode at {width}x{height}")
            }
            Self::UnknownRefreshRate { refresh_millihertz } => write!(
                formatter,
                "no advertised mode at {refresh_millihertz} mHz for that resolution"
            ),
            Self::NotPresentable { width, height } => write!(
                formatter,
                "{width}x{height} is below the {UI_MIN_CLIENT_WIDTH}x{UI_MIN_CLIENT_HEIGHT} minimum"
            ),
        }
    }
}

impl Error for UiSelectionError {}

impl UiDisplaySelection {
    /// Checks a selection against a catalog and normalizes what the mode does not let a player pick.
    ///
    /// Borderless takes the monitor's desktop resolution and refresh whatever was requested, and
    /// windowed keeps its requested client size but reports the desktop's refresh rather than
    /// pretending to select one. Only exclusive fullscreen has to name a pair the monitor actually
    /// advertises, and only it can fail on refresh.
    ///
    /// # Errors
    ///
    /// Returns a structured error for an unknown monitor, an unadvertised resolution or refresh, or
    /// a size below the presentable minimum.
    pub fn resolve(&self, catalog: &UiDisplayCatalog) -> Result<Self, UiSelectionError> {
        if !catalog.has_monitor(&self.monitor) {
            return Err(UiSelectionError::UnknownMonitor {
                key: self.monitor.clone().into_boxed_str(),
            });
        }
        let desktop = catalog.desktop_mode(&self.monitor);
        let desktop_refresh = desktop.map_or(0, UiVideoMode::refresh_millihertz);
        match self.window_mode {
            UiWindowMode::BorderlessDesktop => {
                let desktop = desktop.ok_or(UiSelectionError::UnknownResolution {
                    width: self.resolution.0,
                    height: self.resolution.1,
                })?;
                Ok(Self {
                    resolution: desktop.resolution(),
                    refresh_millihertz: desktop.refresh_millihertz(),
                    ..self.clone()
                })
            }
            UiWindowMode::Windowed => {
                let (width, height) = self.resolution;
                if width < UI_MIN_CLIENT_WIDTH || height < UI_MIN_CLIENT_HEIGHT {
                    return Err(UiSelectionError::NotPresentable { width, height });
                }
                // A windowed client size is not required to be an advertised mode — a window may be
                // any size the desktop can hold — so only the minimum is enforced.
                Ok(Self {
                    refresh_millihertz: desktop_refresh,
                    ..self.clone()
                })
            }
            UiWindowMode::ExclusiveFullscreen => {
                let (width, height) = self.resolution;
                if !catalog
                    .resolutions(&self.monitor)
                    .contains(&self.resolution)
                {
                    return Err(
                        if width < UI_MIN_CLIENT_WIDTH || height < UI_MIN_CLIENT_HEIGHT {
                            UiSelectionError::NotPresentable { width, height }
                        } else {
                            UiSelectionError::UnknownResolution { width, height }
                        },
                    );
                }
                if catalog
                    .exact_mode(&self.monitor, self.resolution, self.refresh_millihertz)
                    .is_none()
                {
                    return Err(UiSelectionError::UnknownRefreshRate {
                        refresh_millihertz: self.refresh_millihertz,
                    });
                }
                Ok(self.clone())
            }
        }
    }
}

/// How long a confirmation dialog waits before rolling back, in milliseconds.
///
/// A mode change that leaves the display unusable cannot be confirmed by the person looking at it,
/// so the timeout is what actually protects them; the value is the familiar fifteen seconds.
pub const UI_DISPLAY_CONFIRM_TIMEOUT_MS: u64 = 15_000;

/// Why an applied mode was rolled back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiRollbackReason {
    /// The confirmation window elapsed without a confirmation.
    TimedOut {
        /// How long was waited, in milliseconds.
        elapsed_ms: u64,
    },
    /// The player declined.
    Declined,
    /// The platform could not apply the mode, or applied it and then lost it.
    Failed {
        /// What the platform reported.
        detail: String,
    },
}

impl UiRollbackReason {
    /// Returns a stable name for a report.
    #[must_use]
    pub const fn row_name(&self) -> &'static str {
        match self {
            Self::TimedOut { .. } => "timed_out",
            Self::Declined => "declined",
            Self::Failed { .. } => "failed",
        }
    }
}

/// What a transaction step did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiDisplayOutcome {
    /// The mode was applied and is awaiting confirmation. The caller reconfigures the surface now.
    AwaitingConfirmation {
        /// When the confirmation window closes, on the caller's own clock.
        deadline_ms: u64,
    },
    /// The mode was confirmed and is now the accepted one. Only here may a preference be written.
    Confirmed {
        /// The accepted selection.
        accepted: Box<UiDisplaySelection>,
    },
    /// The mode was reverted and the previous accepted one is in force again.
    RolledBack {
        /// Why.
        reason: UiRollbackReason,
        /// What is in force now.
        restored: Box<UiDisplaySelection>,
    },
    /// Nothing was pending, so the step did nothing.
    Idle,
}

impl UiDisplayOutcome {
    /// Returns a stable name for a report.
    #[must_use]
    pub const fn row_name(&self) -> &'static str {
        match self {
            Self::AwaitingConfirmation { .. } => "awaiting_confirmation",
            Self::Confirmed { .. } => "confirmed",
            Self::RolledBack { .. } => "rolled_back",
            Self::Idle => "idle",
        }
    }
}

/// A transactional display-mode change: apply, wait for confirmation, commit or revert.
///
/// Time is the caller's throughout — every method that cares takes a millisecond stamp — so a
/// deterministic capture steps a timeout without a clock existing anywhere in the path. The accepted
/// selection is the one thing that survives a rollback, and it is the only one a preference may be
/// written from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiDisplayTransaction {
    accepted: UiDisplaySelection,
    pending: Option<UiDisplaySelection>,
    deadline_ms: Option<u64>,
    started_ms: u64,
    timeout_ms: u64,
}

impl UiDisplayTransaction {
    /// Starts from an already-accepted selection, which is what a rollback returns to.
    #[must_use]
    pub fn new(accepted: UiDisplaySelection) -> Self {
        Self {
            accepted,
            pending: None,
            deadline_ms: None,
            started_ms: 0,
            timeout_ms: UI_DISPLAY_CONFIRM_TIMEOUT_MS,
        }
    }

    /// Overrides the confirmation window, for a test or a preference.
    #[must_use]
    pub const fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    /// Returns the selection currently in force.
    #[must_use]
    pub const fn accepted(&self) -> &UiDisplaySelection {
        &self.accepted
    }

    /// Returns the selection awaiting confirmation, if any.
    #[must_use]
    pub const fn pending(&self) -> Option<&UiDisplaySelection> {
        self.pending.as_ref()
    }

    /// Returns when the confirmation window closes, on the caller's own clock.
    ///
    /// A dialog counting down needs this to say how long is left; nothing about the transaction
    /// depends on anyone reading it.
    #[must_use]
    pub const fn deadline_ms(&self) -> Option<u64> {
        self.deadline_ms
    }

    /// Returns whether a confirmation is outstanding.
    #[must_use]
    pub const fn is_awaiting_confirmation(&self) -> bool {
        self.pending.is_some()
    }

    /// Requests a selection, resolving it against the catalog first.
    ///
    /// A request while another is outstanding replaces it, keeping the original accepted selection as
    /// the rollback target — so a player who changes their mind twice still returns to what worked.
    ///
    /// # Errors
    ///
    /// Returns a structured error when the selection is not one the catalog can offer. Nothing is
    /// applied in that case and the accepted selection is untouched.
    pub fn request(
        &mut self,
        catalog: &UiDisplayCatalog,
        selection: &UiDisplaySelection,
        now_ms: u64,
    ) -> Result<UiDisplayOutcome, UiSelectionError> {
        let resolved = selection.resolve(catalog)?;
        self.pending = Some(resolved);
        self.started_ms = now_ms;
        let deadline = now_ms.saturating_add(self.timeout_ms);
        self.deadline_ms = Some(deadline);
        Ok(UiDisplayOutcome::AwaitingConfirmation {
            deadline_ms: deadline,
        })
    }

    /// Confirms the outstanding selection, making it the accepted one.
    ///
    /// A confirmation that arrives after the deadline is refused and rolls back instead: the dialog
    /// the player was answering is gone by then, and honouring it would apply a mode nobody is
    /// currently looking at.
    pub fn confirm(&mut self, now_ms: u64) -> UiDisplayOutcome {
        let Some(pending) = self.pending.take() else {
            return UiDisplayOutcome::Idle;
        };
        if self.deadline_ms.is_some_and(|deadline| now_ms > deadline) {
            self.deadline_ms = None;
            return UiDisplayOutcome::RolledBack {
                reason: UiRollbackReason::TimedOut {
                    elapsed_ms: now_ms.saturating_sub(self.started_ms),
                },
                restored: Box::new(self.accepted.clone()),
            };
        }
        self.deadline_ms = None;
        self.accepted = pending;
        UiDisplayOutcome::Confirmed {
            accepted: Box::new(self.accepted.clone()),
        }
    }

    /// Reverts the outstanding selection because the player declined.
    pub fn decline(&mut self) -> UiDisplayOutcome {
        self.rollback(UiRollbackReason::Declined)
    }

    /// Reverts the outstanding selection because the platform could not hold it.
    pub fn fail(&mut self, detail: impl Into<String>) -> UiDisplayOutcome {
        self.rollback(UiRollbackReason::Failed {
            detail: detail.into(),
        })
    }

    /// Reverts the outstanding selection if the confirmation window has closed.
    ///
    /// This is what a frame loop calls; it returns [`UiDisplayOutcome::Idle`] until the deadline
    /// passes, so calling it every frame is correct and calling it never is the bug.
    pub fn poll(&mut self, now_ms: u64) -> UiDisplayOutcome {
        if self.pending.is_none() {
            return UiDisplayOutcome::Idle;
        }
        match self.deadline_ms {
            Some(deadline) if now_ms > deadline => self.rollback(UiRollbackReason::TimedOut {
                elapsed_ms: now_ms.saturating_sub(self.started_ms),
            }),
            _ => UiDisplayOutcome::Idle,
        }
    }

    fn rollback(&mut self, reason: UiRollbackReason) -> UiDisplayOutcome {
        if self.pending.take().is_none() {
            return UiDisplayOutcome::Idle;
        }
        self.deadline_ms = None;
        UiDisplayOutcome::RolledBack {
            reason,
            restored: Box::new(self.accepted.clone()),
        }
    }
}
