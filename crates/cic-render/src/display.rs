// Commanders in Chief
// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only
//
// Provenance: project design. The original enumerates display modes through
// `Core/GameEngine/Include/GameClient/Display.h`'s `DisplayModeInterface`, which reports width,
// height, and bit depth and has no concept of a monitor or a refresh rate, so there is nothing here
// to derive. This is the platform half of `cic_ui::UiDisplayCatalog`: it turns what `winit` reports
// into the immutable, deterministically ordered catalog the settings model consumes, and reports a
// capability gap where a backend advertises nothing rather than inventing a plausible value.

use cic_ui::{UiDisplayCatalog, UiDisplayError, UiMonitor, UiVideoMode};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::monitor::MonitorHandle;
use winit::window::WindowId;

/// Builds the immutable catalog from whatever `winit` advertises.
///
/// The key is derived rather than reported: `winit` gives a `MonitorHandle` no stable identifier, so
/// this composes the monitor's name, its virtual-desktop position, and its enumeration index. That is
/// stable within a session — which is all [`UiMonitor`] promises — and it is legible in a report,
/// which a raw handle address would not be. Position is included because two identical panels
/// side by side report the same name and size.
///
/// A monitor advertising no video modes still enters the catalog. Some backends report none while
/// the monitor is perfectly usable, and dropping it would remove a display the player can see; the
/// catalog raises a capability gap for it instead, and the borderless and windowed modes still work
/// from its desktop size.
///
/// # Errors
///
/// Returns a structured error when the catalog's own bounds or invariants are exceeded — more
/// monitors or modes than the limits allow, or a mode with a zero dimension.
pub fn display_catalog_from_monitors(
    monitors: impl Iterator<Item = MonitorHandle>,
) -> Result<UiDisplayCatalog, UiDisplayError> {
    let mut catalog_monitors = Vec::new();
    let mut catalog_modes = Vec::new();
    for (index, monitor) in monitors.enumerate() {
        let key = monitor_key(&monitor, index);
        let name = monitor
            .name()
            .unwrap_or_else(|| format!("Display {}", index + 1));
        catalog_monitors.push(UiMonitor::new(key.clone(), name, index));

        // The desktop mode is recorded first so it is present even where `video_modes` is empty,
        // which is what borderless and windowed both report their refresh from. `size` is always
        // available; `refresh_rate_millihertz` is not, and a zero there is the catalog's own signal
        // that no rate was advertised rather than a rate of zero.
        let desktop = monitor.size();
        if desktop.width > 0 && desktop.height > 0 {
            catalog_modes.push(UiVideoMode::new(
                key.clone(),
                desktop.width,
                desktop.height,
                monitor.refresh_rate_millihertz().unwrap_or(0),
                None,
                catalog_modes.len(),
            ));
        }
        for mode in monitor.video_modes() {
            let size = mode.size();
            catalog_modes.push(UiVideoMode::new(
                key.clone(),
                size.width,
                size.height,
                mode.refresh_rate_millihertz(),
                Some(mode.bit_depth()),
                catalog_modes.len(),
            ));
        }
    }
    UiDisplayCatalog::new(catalog_monitors, catalog_modes)
}

/// Enumerates the host's monitors into a catalog.
///
/// This is the one path in the display stack that touches the machine, and it is deliberately the
/// only one: everything downstream consumes the immutable catalog, so a deterministic capture
/// injects one instead of calling this. `winit` requires an event loop to enumerate at all, and
/// creating one is restricted to the main thread on every supported platform.
///
/// # Errors
///
/// Returns an error when the event loop cannot be created — which is the platform refusing, not a
/// display problem — or when the catalog's own bounds are exceeded.
pub fn enumerate_display_catalog() -> Result<UiDisplayCatalog, Box<dyn std::error::Error>> {
    /// Pumps the loop exactly once to reach an `ActiveEventLoop`, which is the only thing that can
    /// list monitors, then exits before opening a window.
    struct Enumerator {
        catalog: Option<Result<UiDisplayCatalog, UiDisplayError>>,
    }

    impl ApplicationHandler for Enumerator {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            self.catalog = Some(display_catalog_from_monitors(
                event_loop.available_monitors(),
            ));
            event_loop.exit();
        }

        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
    }

    let event_loop = EventLoop::new()?;
    let mut enumerator = Enumerator { catalog: None };
    event_loop.run_app(&mut enumerator)?;
    Ok(enumerator
        .catalog
        .ok_or("the event loop exited before reporting any monitor")??)
}

/// Returns the monitor whose derived key matches, enumerating in the same order the catalog did.
///
/// The key embeds the enumeration index, so a match is only meaningful against the same enumeration
/// the catalog was built from — which is the whole reason [`UiMonitor`] promises per-session
/// stability and no more.
#[must_use]
pub fn find_monitor(
    monitors: impl Iterator<Item = MonitorHandle>,
    key: &str,
) -> Option<MonitorHandle> {
    monitors
        .enumerate()
        .find(|(index, monitor)| monitor_key(monitor, *index) == key)
        .map(|(_, monitor)| monitor)
}

/// Composes a per-session key for one monitor.
///
/// Two monitors of the same model report the same name and size, so the virtual-desktop position is
/// what separates them, and the enumeration index is the final tie-break for the case where a
/// backend reports neither a name nor a distinct position.
fn monitor_key(monitor: &MonitorHandle, index: usize) -> String {
    let position = monitor.position();
    let name = monitor.name().unwrap_or_default();
    format!("{name}@{},{}#{index}", position.x, position.y)
}
