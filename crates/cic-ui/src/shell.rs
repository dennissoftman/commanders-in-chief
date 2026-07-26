// Commanders in Chief
// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only
//
// Provenance: the pseudo-stack, its sixteen-screen bound, the pending push and pending pop protocol
// through `shutdownComplete`, the immediate-pop path, the show/hide-shell pair, hiding every screen,
// running updates from the stack top downwards, and bringing a pushed screen forward are derived
// from Electronic Arts' GPL-3.0 source release, GeneralsGameCode revision
// 9f7abb866f5afd446db14149979e744c7216baaf, specifically
// `GeneralsMD/Code/GameEngine/Source/GameClient/GUI/Shell/Shell.cpp` (`Shell::push`, `Shell::pop`,
// `Shell::popImmediate`, `Shell::doPush`, `Shell::doPop`, `Shell::shutdownComplete`,
// `Shell::showShell`, `Shell::hideShell`, `Shell::hide`, `Shell::update`, `Shell::top`,
// `Shell::linkScreen`, `Shell::unlinkScreen`, `Shell::findScreenByFilename`,
// `Shell::getScreenLayout`), `GeneralsMD/Code/GameEngine/Include/GameClient/Shell.h` (the documented
// push and pop sequences and `MAX_SHELL_STACK`),
// `Core/GameEngine/Source/GameClient/GUI/WindowLayout.cpp` (`WindowLayout::hide`,
// `WindowLayout::bringForward`, `WindowLayout::load`), and
// `Core/GameEngine/Source/GameClient/GUI/GameWindow.cpp` (`GameWindow::winBringToTop`). Typed shell
// events are project design: the original calls a resolved function pointer, and this project never
// does.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

use crate::callbacks::{UiCallbackBinding, UiCallbackSlot, classify_callback};
use crate::frame::{UiClipPolicy, UiFrame};
use crate::input::{UiEvent, UiMouseButton};
use crate::retained::{UiControlId, UiLayout, UiPoint, UiStatus};

/// The original's `MAX_SHELL_STACK`: how many screens may be on the stack at once.
pub const UI_MAX_SHELL_STACK: usize = 16;

/// Why a shell operation could not be performed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiShellError {
    /// A push would exceed [`UI_MAX_SHELL_STACK`].
    ///
    /// The original logs and returns without pushing, leaving the pending-push flag clear.
    StackFull {
        /// The layout that was refused.
        path: Box<str>,
        /// The bound.
        limit: usize,
    },
    /// A push or pop named an empty layout path, which the original refuses outright.
    EmptyPath,
    /// A push arrived while another push or pop was still pending.
    ///
    /// The original asserts that a push and a pop are never pending together; this refuses instead of
    /// continuing with two pending operations.
    OperationPending,
}

impl Display for UiShellError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::StackFull { path, limit } => write!(
                formatter,
                "cannot push {path:?}: the shell stack already holds {limit} screens"
            ),
            Self::EmptyPath => formatter.write_str("a shell screen needs a layout path"),
            Self::OperationPending => {
                formatter.write_str("a shell push or pop is already waiting on a shutdown")
            }
        }
    }
}

impl Error for UiShellError {}

/// One screen on the shell stack: a layout and the virtual path it came from.
#[derive(Debug, Clone)]
pub struct UiScreen {
    path: String,
    layout: UiLayout,
}

impl UiScreen {
    /// Wraps an instantiated layout as a shell screen.
    #[must_use]
    pub fn new(path: impl Into<String>, layout: UiLayout) -> Self {
        Self {
            path: path.into(),
            layout,
        }
    }

    /// Returns the virtual path the layout was loaded from, which is `WindowLayout::getFilename`.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Returns the retained layout.
    #[must_use]
    pub const fn layout(&self) -> &UiLayout {
        &self.layout
    }

    /// Returns the retained layout for mutation.
    pub const fn layout_mut(&mut self) -> &mut UiLayout {
        &mut self.layout
    }

    /// Consumes the screen and returns its layout.
    #[must_use]
    pub fn into_layout(self) -> UiLayout {
        self.layout
    }
}

/// A shell screen's index on the stack, where zero is the bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct UiScreenId(usize);

impl UiScreenId {
    /// Returns the stack index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

/// A typed shell state change.
///
/// The original runs a layout's init, update, or shutdown by calling the function pointer the
/// function lexicon resolved from the layout's authored name. This project emits the name and its
/// classification instead, and a caller decides what — if anything — to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiShellEvent {
    /// A screen was linked onto the stack.
    ScreenPushed {
        /// Its stack index.
        screen: UiScreenId,
        /// Its virtual path.
        path: String,
    },
    /// A screen was unlinked and its windows destroyed.
    ScreenPopped {
        /// The virtual path of the screen that left.
        path: String,
    },
    /// A screen's `LAYOUTINIT` would run here.
    LayoutInit {
        /// Its stack index.
        screen: UiScreenId,
        /// The retained callback name, absent when the layout declares none.
        callback: Option<String>,
        /// What the name resolves to, absent when the layout declares none.
        binding: Option<UiCallbackBinding>,
    },
    /// A screen's `LAYOUTUPDATE` would run here.
    LayoutUpdate {
        /// Its stack index.
        screen: UiScreenId,
        /// The retained callback name, absent when the layout declares none.
        callback: Option<String>,
        /// What the name resolves to, absent when the layout declares none.
        binding: Option<UiCallbackBinding>,
    },
    /// A screen's `LAYOUTSHUTDOWN` would run here.
    LayoutShutdown {
        /// Its stack index.
        screen: UiScreenId,
        /// The retained callback name, absent when the layout declares none.
        callback: Option<String>,
        /// What the name resolves to, absent when the layout declares none.
        binding: Option<UiCallbackBinding>,
        /// Whether the screen will be popped as soon as shutdown returns.
        ///
        /// This is the original's `immediatePop` parameter, which a layout's shutdown reads to decide
        /// whether it may animate or must finish now.
        immediate: bool,
    },
    /// A screen was moved to the front of the draw order.
    BroughtForward {
        /// Its stack index.
        screen: UiScreenId,
    },
    /// Every screen was hidden or shown.
    VisibilityChanged {
        /// Whether screens are now hidden.
        hidden: bool,
    },
}

/// What operation the shell is waiting on a shutdown to finish.
#[derive(Debug, Clone, Default)]
enum Pending {
    /// Nothing is pending.
    #[default]
    None,
    /// A layout is loaded and waiting to be linked once the current top finishes shutting down.
    Push(Box<UiScreen>),
    /// The top of the stack is waiting to be unlinked.
    Pop,
}

/// The retained shell: a screen stack, a draw order over it, and the pending-operation protocol.
///
/// The stack and the draw order are separate because they are separate in the original. The stack is
/// a navigation history — `top` is the screen a back button returns from — while draw order lives in
/// the window manager's own list, which `bringForward` reorders. A screen can therefore be on top of
/// the stack while another screen draws over it.
///
/// Nothing here loads a layout. A caller owns the virtual filesystem, instantiates a [`UiLayout`],
/// and hands it over, which keeps this crate free of filesystem and renderer dependencies.
#[derive(Debug, Clone, Default)]
pub struct UiShell {
    screens: Vec<UiScreen>,
    /// Stack indices from back to front. Every stack index appears exactly once.
    draw_order: Vec<usize>,
    pending: Pending,
    hidden: bool,
}

impl UiShell {
    /// Creates an empty shell.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns how many screens are on the stack.
    #[must_use]
    pub fn screen_count(&self) -> usize {
        self.screens.len()
    }

    /// Returns every screen from the bottom of the stack upwards.
    #[must_use]
    pub fn screens(&self) -> &[UiScreen] {
        &self.screens
    }

    /// Returns one screen by stack index.
    #[must_use]
    pub fn screen(&self, screen: UiScreenId) -> Option<&UiScreen> {
        self.screens.get(screen.0)
    }

    /// Returns one screen by stack index for mutation.
    pub fn screen_mut(&mut self, screen: UiScreenId) -> Option<&mut UiScreen> {
        self.screens.get_mut(screen.0)
    }

    /// Returns the top of the stack, which is `Shell::top`.
    #[must_use]
    pub fn top(&self) -> Option<&UiScreen> {
        self.screens.last()
    }

    /// Returns the top of the stack for mutation.
    pub fn top_mut(&mut self) -> Option<&mut UiScreen> {
        self.screens.last_mut()
    }

    /// Returns the top of the stack's index.
    #[must_use]
    pub fn top_id(&self) -> Option<UiScreenId> {
        self.screens.len().checked_sub(1).map(UiScreenId)
    }

    /// Returns the first screen loaded from this path, compared case-insensitively.
    ///
    /// `Shell::findScreenByFilename` compares with `compareNoCase` and refuses an empty name.
    #[must_use]
    pub fn find_screen_by_path(&self, path: &str) -> Option<UiScreenId> {
        if path.is_empty() {
            return None;
        }
        self.screens
            .iter()
            .position(|screen| screen.path.eq_ignore_ascii_case(path))
            .map(UiScreenId)
    }

    /// Returns stack indices from the back of the draw order to the front.
    #[must_use]
    pub fn draw_order(&self) -> Vec<UiScreenId> {
        self.draw_order.iter().copied().map(UiScreenId).collect()
    }

    /// Finds a control by its full decorated `<layout>:<control>` name across every screen.
    ///
    /// The comparison is exact and case-sensitive, because the original resolves such a name through
    /// `nameToKey` — which compares with `strcmp` — against the window ids `winCreateFromScript`
    /// derived from the same decorated spelling. Screens are searched from the bottom of the stack
    /// upwards, and within a screen in source order, so a duplicate name resolves stably.
    #[must_use]
    pub fn find_control_by_decorated_name(&self, name: &str) -> Option<(UiScreenId, UiControlId)> {
        if name.is_empty() {
            return None;
        }
        for (index, screen) in self.screens.iter().enumerate() {
            if let Some(control) = screen
                .layout
                .controls()
                .iter()
                .find(|control| control.name() == Some(name))
            {
                return Some((UiScreenId(index), control.id()));
            }
        }
        None
    }

    /// Returns a screen's layout by stack index, for a caller holding a resolved screen id.
    #[must_use]
    pub fn layout(&self, screen: UiScreenId) -> Option<&UiLayout> {
        self.screens.get(screen.0).map(UiScreen::layout)
    }

    /// Returns a screen's layout by stack index for mutation.
    pub fn layout_mut(&mut self, screen: UiScreenId) -> Option<&mut UiLayout> {
        self.screens.get_mut(screen.0).map(UiScreen::layout_mut)
    }

    /// Returns whether a push or pop is waiting on a shutdown to complete.
    #[must_use]
    pub fn is_operation_pending(&self) -> bool {
        !matches!(self.pending, Pending::None)
    }

    /// Returns whether the shell as a whole was hidden by [`UiShell::hide`].
    #[must_use]
    pub const fn is_hidden(&self) -> bool {
        self.hidden
    }

    /// Pushes a screen, running the current top's shutdown first.
    ///
    /// This is `Shell::push`. The layout is recorded as a pending push and, when a visible screen is
    /// on top, that screen's shutdown runs and the push waits: the caller must call
    /// [`UiShell::shutdown_complete`] when the shutdown has finished, exactly as the original's
    /// layouts call `Shell::shutdownComplete`. When the stack is empty, or its top is already hidden,
    /// the original short-circuits straight to `shutdownComplete`, and so does this.
    ///
    /// `shutdown_immediate` is passed through to the shutdown event as the original passes it to the
    /// shutdown function.
    ///
    /// # Errors
    ///
    /// Returns an error when the path is empty, the stack is full, or another operation is pending.
    pub fn push(
        &mut self,
        screen: UiScreen,
        shutdown_immediate: bool,
    ) -> Result<Vec<UiShellEvent>, UiShellError> {
        if screen.path.is_empty() {
            return Err(UiShellError::EmptyPath);
        }
        if self.is_operation_pending() {
            return Err(UiShellError::OperationPending);
        }
        if self.screens.len() >= UI_MAX_SHELL_STACK {
            return Err(UiShellError::StackFull {
                path: screen.path.clone().into_boxed_str(),
                limit: UI_MAX_SHELL_STACK,
            });
        }
        self.pending = Pending::Push(Box::new(screen));
        let visible_top = self
            .top_id()
            .filter(|top| !self.screens[top.0].layout.is_hidden());
        match visible_top {
            Some(top) => Ok(vec![self.shutdown_event(top, shutdown_immediate)]),
            // With nothing to shut down the original calls `shutdownComplete(nullptr)` itself, which
            // performs the push in the same call.
            None => Ok(self.shutdown_complete()),
        }
    }

    /// Pops the top screen, running its shutdown first.
    ///
    /// This is `Shell::pop`: the pop is recorded as pending and waits for
    /// [`UiShell::shutdown_complete`]. The original always passes `immediatePop = FALSE` here.
    ///
    /// # Errors
    ///
    /// Returns an error when another operation is pending. An empty stack is not an error: the
    /// original returns quietly, and this returns no events.
    pub fn pop(&mut self) -> Result<Vec<UiShellEvent>, UiShellError> {
        if self.is_operation_pending() {
            return Err(UiShellError::OperationPending);
        }
        let Some(top) = self.top_id() else {
            return Ok(Vec::new());
        };
        self.pending = Pending::Pop;
        Ok(vec![self.shutdown_event(top, false)])
    }

    /// Pops the top screen now, without waiting for a shutdown to report back.
    ///
    /// This is `Shell::popImmediate`, which deliberately leaves the pending-pop flag clear, runs the
    /// shutdown with `immediatePop = TRUE`, and unlinks the screen as soon as it returns.
    ///
    /// # Errors
    ///
    /// Returns an error when another operation is pending.
    pub fn pop_immediate(&mut self) -> Result<Vec<UiShellEvent>, UiShellError> {
        if self.is_operation_pending() {
            return Err(UiShellError::OperationPending);
        }
        let Some(top) = self.top_id() else {
            return Ok(Vec::new());
        };
        let mut events = vec![self.shutdown_event(top, true)];
        events.extend(self.do_pop(false));
        Ok(events)
    }

    /// Completes a pending push or pop.
    ///
    /// This is `Shell::shutdownComplete`, the hinge of both sequences: a pending push links the
    /// loaded layout, runs its init, and brings it forward, while a pending pop unlinks the top,
    /// destroys its windows, and runs the new top's init.
    ///
    /// `impending_push` is the original's parameter of the same name: when a pop is completing only
    /// to make room for a push, the new top's init is skipped because the push is about to cover it.
    pub fn shutdown_complete(&mut self) -> Vec<UiShellEvent> {
        self.shutdown_complete_with(false)
    }

    /// Completes a pending push or pop, declaring whether a push follows immediately.
    pub fn shutdown_complete_with(&mut self, impending_push: bool) -> Vec<UiShellEvent> {
        match std::mem::replace(&mut self.pending, Pending::None) {
            Pending::Push(screen) => self.do_push(*screen),
            Pending::Pop => self.do_pop(impending_push),
            Pending::None => Vec::new(),
        }
    }

    /// Runs the top screen's init as though it had just been pushed.
    ///
    /// This is `Shell::showShell`, which the original uses when returning to a stack that already
    /// exists. Its `bringForward` call is commented out in the source, so nothing is reordered here.
    pub fn show_shell(&mut self, run_init: bool) -> Vec<UiShellEvent> {
        if !run_init {
            return Vec::new();
        }
        match self.top_id() {
            Some(top) => vec![self.init_event(top)],
            None => Vec::new(),
        }
    }

    /// Runs the top screen's shutdown without popping it.
    ///
    /// This is `Shell::hideShell`, which the original uses when leaving the menus for a game while
    /// keeping the stack intact. The shutdown is told an immediate pop is coming even though none
    /// happens, which is what the source passes.
    pub fn hide_shell(&mut self) -> Vec<UiShellEvent> {
        match self.top_id() {
            Some(top) => vec![self.shutdown_event(top, true)],
            None => Vec::new(),
        }
    }

    /// Hides or shows every screen on the stack.
    ///
    /// This is `Shell::hide`, which walks the whole stack rather than only the top and calls each
    /// layout's own `hide`.
    pub fn hide(&mut self, hidden: bool) -> Vec<UiShellEvent> {
        for screen in &mut self.screens {
            screen.layout.hide(hidden);
        }
        self.hidden = hidden;
        vec![UiShellEvent::VisibilityChanged { hidden }]
    }

    /// Emits one update per screen, from the top of the stack downwards.
    ///
    /// This is `Shell::update`, which runs every screen's update whether or not it is on top or
    /// visible, starting at the top index and counting down. The original gates this on a fixed
    /// thirty-per-second wall clock; the caller owns the clock here, so a deterministic capture can
    /// step updates explicitly.
    #[must_use]
    pub fn update(&self) -> Vec<UiShellEvent> {
        (0..self.screens.len())
            .rev()
            .map(|index| self.update_event(UiScreenId(index)))
            .collect()
    }

    /// Moves one screen to the front of the draw order, preserving every other screen's order.
    ///
    /// `WindowLayout::bringForward` walks its windows from the tail and brings each to the top of the
    /// window manager's list, which lands the layout in front with its own window order intact.
    pub fn bring_forward(&mut self, screen: UiScreenId) -> Vec<UiShellEvent> {
        if screen.0 >= self.screens.len() {
            return Vec::new();
        }
        self.draw_order.retain(|index| *index != screen.0);
        self.draw_order.push(screen.0);
        vec![UiShellEvent::BroughtForward { screen }]
    }

    /// Returns the control the pointer is over, searching every screen front to back.
    ///
    /// The original's cursor search runs over the window manager's single global window list, not per
    /// layout, so its three passes — `ABOVE` first, then windows with none of `ABOVE`, `BELOW`, or
    /// `HIDDEN`, then `BELOW` — span every screen at once. Each pass here walks the draw order from
    /// front to back and, within a screen, its roots in source order. A screen that holds the mouse
    /// confines the search to itself.
    #[must_use]
    pub fn hit_test(&self, point: UiPoint) -> Option<(UiScreenId, UiControlId)> {
        for (index, screen) in self.screens.iter().enumerate() {
            if screen.layout.capture().is_some() {
                return screen
                    .layout
                    .hit_test(point)
                    .map(|control| (UiScreenId(index), control));
            }
        }
        let passes: [fn(UiStatus) -> bool; 3] = [
            |status| status.contains(UiStatus::ABOVE),
            |status| !status.intersects(UiStatus::ABOVE) && !status.intersects(UiStatus::BELOW),
            |status| status.contains(UiStatus::BELOW),
        ];
        for accepts in passes {
            for index in self.draw_order.iter().rev() {
                let screen = &self.screens[*index];
                if let Some(control) = screen.layout.hit_test_pass(point, accepts) {
                    return Some((UiScreenId(*index), control));
                }
            }
        }
        None
    }

    /// Moves the pointer over the whole stack, so exactly one control anywhere holds the hover.
    ///
    /// The original's hover follows `getWindowUnderCursor` over one global window list, so a screen
    /// the pointer is not over must drop whatever hover it held. Each screen is told the result of the
    /// shell-level search rather than searching for itself, and events carry the screen they belong
    /// to.
    pub fn pointer_moved(&mut self, point: UiPoint) -> Vec<(UiScreenId, UiEvent)> {
        let target = self.hit_test(point);
        let mut events = Vec::new();
        for index in 0..self.screens.len() {
            let id = UiScreenId(index);
            let control = target.filter(|(screen, _)| *screen == id).map(|(_, c)| c);
            events.extend(
                self.screens[index]
                    .layout
                    .pointer_moved_to(control)
                    .into_iter()
                    .map(|event| (id, event)),
            );
        }
        events
    }

    /// Presses at a point, on whichever screen the layered search lands in.
    pub fn pointer_pressed(
        &mut self,
        point: UiPoint,
        button: UiMouseButton,
    ) -> Vec<(UiScreenId, UiEvent)> {
        let Some((screen, control)) = self.hit_test(point) else {
            return Vec::new();
        };
        self.screens[screen.0]
            .layout
            .pointer_pressed_on(control, button)
            .into_iter()
            .map(|event| (screen, event))
            .collect()
    }

    /// Releases at a point, on the screen that holds the mouse capture.
    ///
    /// The press is what decided the screen: while a control holds the mouse the original's search is
    /// confined to it, so no fresh search happens here and a release that wandered onto another
    /// screen still cancels the press it started.
    pub fn pointer_released(
        &mut self,
        point: UiPoint,
        button: UiMouseButton,
    ) -> Vec<(UiScreenId, UiEvent)> {
        let Some(screen) = self.capture_screen() else {
            return Vec::new();
        };
        self.screens[screen.0]
            .layout
            .pointer_released(point, button)
            .into_iter()
            .map(|event| (screen, event))
            .collect()
    }

    /// Returns the screen whose layout holds the mouse capture.
    #[must_use]
    pub fn capture_screen(&self) -> Option<UiScreenId> {
        self.screens
            .iter()
            .position(|screen| screen.layout.capture().is_some())
            .map(UiScreenId)
    }

    /// Returns the screen whose layout holds keyboard focus, else the front-most screen.
    ///
    /// Focus is global in the original — the window manager holds a single focused window — so a key
    /// has exactly one destination. With nothing focused the front-most screen is the one a key
    /// reaches, which is where `Shell::push` leaves focus after bringing a screen forward.
    #[must_use]
    pub fn focus_screen(&self) -> Option<UiScreenId> {
        self.screens
            .iter()
            .position(|screen| screen.layout.focus().is_some())
            .map(UiScreenId)
            .or_else(|| self.draw_order.last().copied().map(UiScreenId))
    }

    /// Builds one renderer-neutral frame per screen, from the back of the draw order to the front.
    ///
    /// A screen whose layout is hidden contributes an empty frame rather than being dropped, because
    /// its stack position is still meaningful to a caller attributing items back to screens.
    #[must_use]
    pub fn frames(&self, clip: UiClipPolicy) -> Vec<(UiScreenId, UiFrame)> {
        self.draw_order
            .iter()
            .map(|index| (UiScreenId(*index), self.screens[*index].layout.frame(clip)))
            .collect()
    }

    /// Builds one composed frame over every screen, in draw order.
    ///
    /// The original draws the whole window list in one pass, so a composed frame — not one frame per
    /// screen — is what a renderer receives. Screens appear back to front, which puts the most
    /// recently brought-forward screen's items last and therefore on top.
    #[must_use]
    pub fn frame(&self, clip: UiClipPolicy) -> UiFrame {
        let mut composed = UiFrame::default();
        for (_, frame) in self.frames(clip) {
            composed.append(frame);
        }
        composed
    }

    fn do_push(&mut self, screen: UiScreen) -> Vec<UiShellEvent> {
        let index = self.screens.len();
        let id = UiScreenId(index);
        let path = screen.path.clone();
        self.screens.push(screen);
        self.draw_order.push(index);
        let mut events = vec![
            UiShellEvent::ScreenPushed { screen: id, path },
            self.init_event(id),
        ];
        events.extend(self.bring_forward(id));
        events
    }

    fn do_pop(&mut self, impending_push: bool) -> Vec<UiShellEvent> {
        let Some(top) = self.top_id() else {
            return Vec::new();
        };
        let screen = self.screens.remove(top.0);
        self.draw_order.retain(|index| *index != top.0);
        let mut events = vec![UiShellEvent::ScreenPopped { path: screen.path }];
        // The new top's init runs unless a push is about to cover it. The source's `bringForward`
        // here is commented out, so the draw order is left alone.
        if !impending_push && let Some(new_top) = self.top_id() {
            events.push(self.init_event(new_top));
        }
        events
    }

    fn init_event(&self, screen: UiScreenId) -> UiShellEvent {
        let (callback, binding) = self.callback(screen, UiCallbackSlot::LayoutInit);
        UiShellEvent::LayoutInit {
            screen,
            callback,
            binding,
        }
    }

    fn update_event(&self, screen: UiScreenId) -> UiShellEvent {
        let (callback, binding) = self.callback(screen, UiCallbackSlot::LayoutUpdate);
        UiShellEvent::LayoutUpdate {
            screen,
            callback,
            binding,
        }
    }

    fn shutdown_event(&self, screen: UiScreenId, immediate: bool) -> UiShellEvent {
        let (callback, binding) = self.callback(screen, UiCallbackSlot::LayoutShutdown);
        UiShellEvent::LayoutShutdown {
            screen,
            callback,
            binding,
            immediate,
        }
    }

    fn callback(
        &self,
        screen: UiScreenId,
        slot: UiCallbackSlot,
    ) -> (Option<String>, Option<UiCallbackBinding>) {
        let Some(entry) = self.screens.get(screen.0) else {
            return (None, None);
        };
        let name = match slot {
            UiCallbackSlot::LayoutInit => entry.layout.layout_init_callback(),
            UiCallbackSlot::LayoutUpdate => entry.layout.layout_update_callback(),
            UiCallbackSlot::LayoutShutdown => entry.layout.layout_shutdown_callback(),
            _ => None,
        };
        match name {
            Some(name) => (Some(name.to_owned()), Some(classify_callback(slot, name))),
            None => (None, None),
        }
    }
}
