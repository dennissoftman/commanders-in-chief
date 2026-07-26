// Commanders in Chief
// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Retained, renderer-neutral user-interface state.
//!
//! [`cic_formats`] produces immutable WND definitions. This crate instantiates one into a retained
//! control tree that owns presentation state — visibility, enablement, hover, press, focus,
//! selection, text, and scroll offsets — and answers layout, hit-testing, focus, and input
//! questions against it.
//!
//! Three boundaries hold throughout:
//!
//! - **UI state is presentation state.** Nothing here is authoritative game state, and this crate
//!   depends on no simulation, renderer, filesystem, or audio component.
//! - **Callback names stay data.** Input produces typed [`UiEvent`] values carrying the source
//!   callback name; nothing dispatches a name to a function.
//! - **Order is stable.** Source order controls the tree, hit testing, focus traversal, and draw
//!   submission; no iteration depends on a host hash or a clock.

mod frame;
mod input;
mod retained;
#[cfg(test)]
mod tests;

pub use frame::{
    UI_MAX_TABS, UiClipPolicy, UiControlFamily, UiDrawState, UiFrame, UiFrameItem, UiSlotImages,
    UiTabGeometry, UiTextAlign, UiTextRun,
};
pub use input::{UiEvent, UiKey, UiMouseButton};
pub use retained::{
    UiControl, UiControlId, UiControlKind, UiDiagnostic, UiDiagnosticKind, UiLayout, UiLayoutError,
    UiLimits, UiPoint, UiPresentation, UiRect, UiScalePolicy, UiStatus, UiViewport,
};
