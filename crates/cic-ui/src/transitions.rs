// Commanders in Chief
// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only
//
// Provenance: the group scheduler, the frame accumulator and its integer stepping, the per-window
// frame delay window, reverse, skip, fire-once, the draw and secondary-draw groups, and every style's
// per-frame hidden-state and draw-state machine are derived from Electronic Arts' GPL-3.0 source
// release, GeneralsGameCode revision 9f7abb866f5afd446db14149979e744c7216baaf, specifically
// `Core/GameEngine/Source/GameClient/GUI/GameWindowTransitions.cpp` (`TransitionWindow::init`,
// `TransitionWindow::update`, `TransitionWindow::getTotalFrames`, `TransitionGroup::init`,
// `TransitionGroup::update`, `TransitionGroup::reverse`, `TransitionGroup::isFinished`,
// `TransitionGroup::isReversed`, `TransitionGroup::skip`,
// `GameWindowTransitionsHandler::update`, `::draw`, `::setGroup`, `::reverse`, `::remove`,
// `::isFinished`, `::findGroup`, `getTransitionForStyle`),
// `Core/GameEngine/Include/GameClient/GameWindowTransitions.h` (every `*TRANSITION_*` state
// constant), and
// `GeneralsMD/Code/GameEngine/Source/GameClient/GUI/GameWindowTransitionsStyles.cpp` (each style's
// `init`, `update`, `draw`, `reverse`, and `skip`, plus `drawTypeText` and
// `PushButtonImageDrawThree`). Renderer-neutral draw records and the explicit caller-supplied time
// step are project design: the original draws immediately through `TheDisplay` and reads a wall
// clock through the frame pacer, and this project does neither.

use cic_formats::{TransitionStyle, WindowTransitionsIni, WndColor, WndDrawDataSlot};

use crate::retained::{UiControlId, UiRect};
use crate::shell::{UiScreenId, UiShell};

/// The transition clock's fixed rate. Group frames are counted at this rate whatever the present
/// rate, which is what lets a definition's `FrameDelay` mean the same thing on any machine.
pub const UI_TRANSITION_FRAMES_PER_SECOND: i32 = 30;

/// A resolved window a transition drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiTransitionTarget {
    /// Which screen holds it.
    pub screen: UiScreenId,
    /// Which control it is.
    pub control: UiControlId,
}

/// One renderer-neutral draw a transition contributes for the current frame.
///
/// Every style draws a stand-in while the real window is hidden, which is why these are separate from
/// a control's own frame items: the rectangle, the alpha, and often the image are the transition's,
/// not the control's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiTransitionDraw {
    /// A filled and/or outlined rectangle.
    Rect {
        /// Absolute rectangle.
        rect: UiRect,
        /// Fill colour, absent when the state only outlines.
        fill: Option<WndColor>,
        /// Outline colour, absent when the state only fills.
        outline: Option<WndColor>,
        /// Outline width in pixels; the source passes one everywhere.
        outline_width: i32,
    },
    /// One of a control's own declared images, scaled into an arbitrary rectangle.
    ControlImage {
        /// The control the image belongs to.
        target: UiTransitionTarget,
        /// Which draw-data slot to read.
        slot: WndDrawDataSlot,
        /// Which entry in that slot.
        entry: usize,
        /// Where to draw it.
        rect: UiRect,
        /// Modulating colour, white with a rising alpha for the fades.
        color: WndColor,
    },
    /// A mapped image named by the style rather than by a control, such as the button flash's
    /// `Gradient`.
    NamedImage {
        /// The mapped-image name.
        image: Box<str>,
        /// Where to draw it.
        rect: UiRect,
        /// Modulating colour.
        color: WndColor,
    },
    /// A push button's three-piece enabled art at one alpha, which is `PushButtonImageDrawThree`.
    PushButtonPieces {
        /// The button.
        target: UiTransitionTarget,
        /// The alpha to modulate every piece by.
        alpha: u8,
    },
    /// A partially typed label, drawn where the complete label would sit.
    ///
    /// `drawTypeText` measures the control's *full* display string for placement and then draws the
    /// partial string at that position, so a centred label stays anchored where the finished text
    /// starts rather than sliding as characters arrive.
    TypedText {
        /// The static text control.
        target: UiTransitionTarget,
        /// The characters revealed so far.
        text: String,
    },
}

/// A non-fatal observation from running a transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiTransitionDiagnosticKind {
    /// A `WinName` no loaded screen carries a control for.
    ///
    /// The source asserts and then carries on with a null window, and every style tests for it, so a
    /// transition over a missing window runs its frames and draws nothing.
    WindowNotFound {
        /// The decorated name as authored.
        name: Box<str>,
    },
    /// A style needing a companion window that is not present.
    ///
    /// The main-menu scale styles each pair with a second window — a fixed marker, or the animated
    /// window's own name with `Medium` or `Small` appended — and return from `init` without arming
    /// themselves when it is missing.
    CompanionNotFound {
        /// The companion name that was looked for.
        name: Box<str>,
    },
    /// A style whose draw needs a resource this presentation-only milestone has no source for.
    ///
    /// `CONTROLBARARROW` draws the control bar's arrow image, which comes from the control-bar
    /// definitions rather than from a WND or a mapped-image name, so its geometry runs and its draw is
    /// reported instead of composed.
    UnsupportedDraw {
        /// Which style.
        style: TransitionStyle,
        /// What it wanted.
        reason: Box<str>,
    },
    /// An audio cue the source fires on this frame.
    ///
    /// R4 has no audio. The cue is reported so a later milestone can bind it, and so a capture can
    /// show that the frame it belongs to was reached.
    AudioCue {
        /// The source event name.
        event: Box<str>,
    },
}

/// One non-fatal observation, attributed to the window that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiTransitionDiagnostic {
    window: Box<str>,
    frame: i32,
    kind: UiTransitionDiagnosticKind,
}

impl UiTransitionDiagnostic {
    /// Returns the decorated `WinName` the observation belongs to.
    #[must_use]
    pub fn window(&self) -> &str {
        &self.window
    }

    /// Returns the group frame it was observed on.
    #[must_use]
    pub const fn frame(&self) -> i32 {
        self.frame
    }

    /// Returns the observation detail.
    #[must_use]
    pub const fn kind(&self) -> &UiTransitionDiagnosticKind {
        &self.kind
    }
}

/// What one step of the transition handler did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UiTransitionStep {
    frames: Vec<i32>,
    diagnostics: Vec<UiTransitionDiagnostic>,
}

impl UiTransitionStep {
    /// Returns every group frame the step ran, in the order it ran them.
    ///
    /// A step runs one frame per whole frame the accumulator crossed, so a slow present rate runs
    /// several and a fast one may run none.
    #[must_use]
    pub fn frames(&self) -> &[i32] {
        &self.frames
    }

    /// Returns every observation the step produced.
    #[must_use]
    pub fn diagnostics(&self) -> &[UiTransitionDiagnostic] {
        &self.diagnostics
    }
}

/// One window's live transition state.
#[derive(Debug, Clone, PartialEq)]
struct UiTransition {
    style: TransitionStyle,
    name: Box<str>,
    target: Option<UiTransitionTarget>,
    companion: Option<UiTransitionTarget>,
    frame_delay: i32,
    frame_length: i32,
    draw_state: i32,
    forward: bool,
    finished: bool,
    /// The target's absolute rectangle, captured at `init` as the source captures position and size.
    rect: UiRect,
    /// The companion's absolute rectangle.
    companion_rect: UiRect,
    /// The viewport, for the styles that fade the whole screen.
    viewport: UiRect,
    full_text: String,
    partial_text: String,
    count_target: i32,
    count_step: i32,
    count_value: i32,
}

/// A named group of transitions that run together.
#[derive(Debug, Clone, PartialEq)]
struct UiTransitionGroup {
    name: Box<str>,
    fire_once: bool,
    windows: Vec<UiTransition>,
    current_frame: f32,
    direction: i32,
}

/// The retained transition handler: the current group, the group queued behind it, and the two groups
/// that still have something to draw.
///
/// Time is the caller's. `update` takes the number of transition frames elapsed since the last call
/// as a scale, which is what the original computes from its frame pacer and the user's transition
/// speed preference, so a deterministic capture can step whole frames explicitly.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UiTransitionHandler {
    groups: Vec<UiTransitionGroup>,
    current: Option<usize>,
    pending: Option<usize>,
    draw: Option<usize>,
    secondary_draw: Option<usize>,
}

impl UiTransitionHandler {
    /// Builds a handler over decoded definitions. Nothing is current until a group is set.
    #[must_use]
    pub fn new(definitions: &WindowTransitionsIni) -> Self {
        let groups = definitions
            .groups()
            .iter()
            .map(|group| UiTransitionGroup {
                name: String::from_utf8_lossy(group.name_bytes())
                    .into_owned()
                    .into_boxed_str(),
                fire_once: group.fire_once(),
                windows: group
                    .windows()
                    .iter()
                    .map(|window| UiTransition {
                        style: window.style(),
                        name: String::from_utf8_lossy(window.window_name_bytes())
                            .into_owned()
                            .into_boxed_str(),
                        target: None,
                        companion: None,
                        frame_delay: window.frame_delay(),
                        frame_length: window.style().declared_frame_length(),
                        draw_state: -1,
                        forward: true,
                        finished: false,
                        rect: UiRect {
                            x: 0,
                            y: 0,
                            width: 0,
                            height: 0,
                        },
                        companion_rect: UiRect {
                            x: 0,
                            y: 0,
                            width: 0,
                            height: 0,
                        },
                        viewport: UiRect {
                            x: 0,
                            y: 0,
                            width: 0,
                            height: 0,
                        },
                        full_text: String::new(),
                        partial_text: String::new(),
                        count_target: 0,
                        count_step: 1,
                        count_value: 0,
                    })
                    .collect(),
                current_frame: 0.0,
                direction: 1,
            })
            .collect();
        Self {
            groups,
            current: None,
            pending: None,
            draw: None,
            secondary_draw: None,
        }
    }

    /// Returns every group's name in definition order.
    pub fn group_names(&self) -> impl Iterator<Item = &str> {
        self.groups.iter().map(|group| &*group.name)
    }

    /// Returns the current group's name, if one is running.
    #[must_use]
    pub fn current_group(&self) -> Option<&str> {
        self.current.map(|index| &*self.groups[index].name)
    }

    /// Returns the group queued behind the current one, if any.
    #[must_use]
    pub fn pending_group(&self) -> Option<&str> {
        self.pending.map(|index| &*self.groups[index].name)
    }

    /// Returns whether the current group has finished, or `true` when none is running.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.current
            .is_none_or(|index| self.groups[index].is_finished())
    }

    /// Returns whether the current group is running backwards.
    #[must_use]
    pub fn is_reversed(&self) -> bool {
        self.current
            .is_some_and(|index| self.groups[index].direction < 0)
    }

    /// Returns the current group's frame accumulator, for a report or a test.
    #[must_use]
    pub fn current_frame(&self) -> Option<f32> {
        self.current.map(|index| self.groups[index].current_frame)
    }

    /// Makes a group current, reversing or skipping whatever was running.
    ///
    /// This is `setGroup`. With nothing running the named group simply starts. With a group running
    /// and `immediate` set, that group is skipped to its end and the named one replaces it at once.
    /// Otherwise the running group is reversed — unless it is fire-once or already reversed — and the
    /// named group waits behind it, which is how one menu animates out while the next animates in.
    pub fn set_group(
        &mut self,
        shell: &mut UiShell,
        name: &str,
        immediate: bool,
    ) -> UiTransitionStep {
        let mut step = UiTransitionStep::default();
        if name.is_empty() && immediate {
            self.current = None;
        }
        if immediate && let Some(current) = self.current {
            self.groups[current].skip(shell, &mut step);
            self.current = self.find(name);
            if let Some(index) = self.current {
                self.init_group(index, shell, &mut step);
            }
            return step;
        }
        if let Some(current) = self.current {
            let group = &mut self.groups[current];
            if !group.fire_once && group.direction >= 0 {
                group.reverse(shell, &mut step);
            }
            self.pending = self.find(name);
            if let Some(index) = self.pending {
                self.init_group(index, shell, &mut step);
            }
            return step;
        }
        self.current = self.find(name);
        if let Some(index) = self.current {
            self.init_group(index, shell, &mut step);
        }
        step
    }

    /// Runs a group backwards.
    ///
    /// This is `reverse`. Reversing the current group simply flips its direction; reversing the
    /// pending one drops it. Reversing any other group skips whatever is running, then starts the
    /// named group, skips it to its end, and reverses it from there — which is how a menu that was
    /// never animated in still animates out. The source dereferences the group without checking it
    /// exists; an unknown name is a no-op here.
    pub fn reverse(&mut self, shell: &mut UiShell, name: &str) -> UiTransitionStep {
        let mut step = UiTransitionStep::default();
        let Some(target) = self.find(name) else {
            return step;
        };
        if self.current == Some(target) {
            self.groups[target].reverse(shell, &mut step);
            return step;
        }
        if self.pending == Some(target) {
            self.pending = None;
            return step;
        }
        if let Some(current) = self.current {
            self.groups[current].skip(shell, &mut step);
        }
        if let Some(pending) = self.pending {
            self.groups[pending].skip(shell, &mut step);
        }
        self.current = Some(target);
        self.init_group(target, shell, &mut step);
        self.groups[target].skip(shell, &mut step);
        self.groups[target].reverse(shell, &mut step);
        self.pending = None;
        step
    }

    /// Drops a group, skipping it to its end first.
    ///
    /// This is `remove`. One source quirk is reproduced: promoting the pending group after removing
    /// the current one does not clear the pending slot, so both briefly name the same group. The next
    /// `update` clears it, which is why it never shows.
    pub fn remove(
        &mut self,
        shell: &mut UiShell,
        name: &str,
        skip_pending: bool,
    ) -> UiTransitionStep {
        let mut step = UiTransitionStep::default();
        let Some(target) = self.find(name) else {
            return step;
        };
        if self.pending == Some(target) {
            if skip_pending {
                self.groups[target].skip(shell, &mut step);
            }
            self.pending = None;
        }
        if self.current == Some(target) {
            self.groups[target].skip(shell, &mut step);
            self.current = self.pending;
        }
        step
    }

    /// Advances the current group by `time_scale` transition frames and rotates the draw groups.
    ///
    /// This is `update`. `time_scale` is the original's
    /// `getBaseOverUpdateFpsRatio() * m_gameWindowTransitionSpeedMultiplier`: one whole frame per call
    /// at the base rate, less when presenting faster, more when presenting slower. The caller owns it,
    /// so a capture passes `1.0` and steps exactly one frame.
    pub fn update(&mut self, shell: &mut UiShell, time_scale: f32) -> UiTransitionStep {
        let mut step = UiTransitionStep::default();
        // The group that was drawing keeps drawing for one more frame when the current group changed,
        // so the outgoing group's last frame is not dropped.
        self.secondary_draw = if self.draw == self.current {
            None
        } else {
            self.draw
        };
        self.draw = self.current;

        if let Some(current) = self.current
            && !self.groups[current].is_finished()
        {
            self.groups[current].update(shell, time_scale, &mut step);
        }
        if let Some(current) = self.current
            && self.groups[current].is_finished()
            && self.groups[current].fire_once
        {
            self.current = None;
        }
        if let Some(current) = self.current
            && self.pending.is_some()
            && self.groups[current].is_finished()
        {
            self.current = self.pending.take();
        }
        if self.current.is_none() && self.pending.is_some() {
            self.current = self.pending.take();
        }
        if let Some(current) = self.current
            && self.groups[current].is_finished()
            && self.groups[current].direction < 0
        {
            self.current = None;
        }
        step
    }

    /// Returns every draw the frame contributes, in submission order.
    ///
    /// This is `draw`, which submits the draw group and then the secondary draw group.
    #[must_use]
    pub fn draws(&self) -> Vec<UiTransitionDraw> {
        let mut draws = Vec::new();
        for group in [self.draw, self.secondary_draw].into_iter().flatten() {
            for window in &self.groups[group].windows {
                window.emit(&mut draws);
            }
        }
        draws
    }

    /// Returns the resolved target of every window in a group, for a report.
    #[must_use]
    pub fn group_targets(
        &self,
        name: &str,
    ) -> Vec<(&str, TransitionStyle, Option<UiTransitionTarget>)> {
        self.find(name).map_or_else(Vec::new, |index| {
            self.groups[index]
                .windows
                .iter()
                .map(|window| (&*window.name, window.style, window.target))
                .collect()
        })
    }

    /// Returns the frame a group finishes on, which is its longest window's delay plus length.
    #[must_use]
    pub fn group_total_frames(&self, name: &str) -> i32 {
        self.find(name).map_or(0, |index| {
            self.groups[index]
                .windows
                .iter()
                .map(UiTransition::total_frames)
                .max()
                .unwrap_or(0)
        })
    }

    /// Finds a group by name, compared case-insensitively as `findGroup` does.
    fn find(&self, name: &str) -> Option<usize> {
        if name.is_empty() {
            return None;
        }
        self.groups
            .iter()
            .position(|group| group.name.eq_ignore_ascii_case(name))
    }

    fn init_group(&mut self, index: usize, shell: &mut UiShell, step: &mut UiTransitionStep) {
        self.groups[index].init(shell, step);
    }
}

impl UiTransitionGroup {
    /// Resolves and arms every window, and rewinds the accumulator.
    fn init(&mut self, shell: &mut UiShell, step: &mut UiTransitionStep) {
        self.current_frame = 0.0;
        self.direction = 1;
        for window in &mut self.windows {
            window.init(shell, step);
        }
    }

    fn is_finished(&self) -> bool {
        self.windows.iter().all(|window| window.finished)
    }

    /// Steps every whole frame the accumulator crossed, stopping early once every window is done.
    fn update(&mut self, shell: &mut UiShell, time_scale: f32, step: &mut UiTransitionStep) {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "the source truncates the same accumulator to Int"
        )]
        let previous = self.current_frame as i32;
        let scale = if self.direction < 0 {
            -time_scale
        } else {
            time_scale
        };
        self.current_frame += scale;
        #[expect(
            clippy::cast_possible_truncation,
            reason = "the source truncates the same accumulator to Int"
        )]
        let next = self.current_frame as i32;
        if next == previous {
            return;
        }
        let increment = if next > previous { 1 } else { -1 };
        let mut frame = previous + increment;
        loop {
            for window in &mut self.windows {
                window.update(shell, frame, step);
            }
            step.frames.push(frame);
            if self.is_finished() || frame == next {
                break;
            }
            frame += increment;
        }
    }

    /// Flips direction and rewinds every window to the group's own end.
    fn reverse(&mut self, shell: &mut UiShell, step: &mut UiTransitionStep) {
        self.direction = -1;
        let total = self
            .windows
            .iter()
            .map(UiTransition::total_frames)
            .max()
            .unwrap_or(0);
        for window in &mut self.windows {
            window.reverse(shell, step);
        }
        #[expect(
            clippy::cast_precision_loss,
            reason = "a frame count is far inside f32's exact integer range"
        )]
        {
            self.current_frame = total as f32;
        }
    }

    fn skip(&mut self, shell: &mut UiShell, step: &mut UiTransitionStep) {
        for window in &mut self.windows {
            window.skip(shell, step);
        }
    }
}

/// A style's fixed companion window, if it has one.
///
/// `MAINMENUSCALEUP` names one window outright; the other two append a suffix to the animated
/// window's own decorated name.
fn companion_name(style: TransitionStyle, window: &str) -> Option<String> {
    match style {
        TransitionStyle::MainMenuScaleUp => Some("MainMenu.wnd:WinGrowMarker".to_owned()),
        TransitionStyle::MainMenuMediumScaleUp => Some(format!("{window}Medium")),
        TransitionStyle::MainMenuSmallScaleDown => Some(format!("{window}Small")),
        _ => None,
    }
}

impl UiTransition {
    /// Returns the frame this window finishes on, which is `getTotalFrames`.
    const fn total_frames(&self) -> i32 {
        self.frame_delay.saturating_add(self.frame_length)
    }

    /// Resolves the window, captures geometry, and runs the start frame backwards to seed state.
    ///
    /// Every style's `init` does the same three things: capture the target's screen position and size,
    /// run `update(START)` with the direction flipped so the start state applies, then clear the
    /// finished flag and face forward.
    fn init(&mut self, shell: &mut UiShell, step: &mut UiTransitionStep) {
        self.draw_state = -1;
        self.finished = false;
        self.frame_length = self.style.declared_frame_length();
        self.partial_text.clear();
        self.count_value = 0;
        self.viewport = viewport_rect(shell);

        self.target = resolve(shell, &self.name);
        if self.target.is_none() {
            step.diagnostics.push(UiTransitionDiagnostic {
                window: self.name.clone(),
                frame: 0,
                kind: UiTransitionDiagnosticKind::WindowNotFound {
                    name: self.name.clone(),
                },
            });
        }
        if let Some(target) = self.target {
            self.rect = screen_rect(shell, target);
            self.full_text = control_text(shell, target);
        }

        if let Some(name) = companion_name(self.style, &self.name) {
            self.companion = resolve(shell, &name);
            if let Some(companion) = self.companion {
                self.companion_rect = screen_rect(shell, companion);
            } else {
                // Each of these styles returns from `init` here, leaving itself unarmed: not
                // finished, no frames applied, nothing drawn.
                step.diagnostics.push(UiTransitionDiagnostic {
                    window: self.name.clone(),
                    frame: 0,
                    kind: UiTransitionDiagnosticKind::CompanionNotFound {
                        name: name.into_boxed_str(),
                    },
                });
                return;
            }
        }

        match self.style {
            TransitionStyle::TypeText => {
                let characters = i32::try_from(self.full_text.chars().count()).unwrap_or(i32::MAX);
                self.frame_length = characters.min(self.style.declared_frame_length());
            }
            TransitionStyle::CountUp => {
                // A count-up over an already-hidden window is finished before it starts.
                if self.target.is_some_and(|target| is_hidden(shell, target)) {
                    self.forward = true;
                    self.finished = true;
                    self.frame_length = 0;
                    return;
                }
                self.count_target = leading_integer(&self.full_text);
                let declared = self.style.declared_frame_length();
                let (state, length) = if self.count_target < declared {
                    (1, self.count_target)
                } else if self.count_target / 100 < declared {
                    (100, self.count_target / 100)
                } else {
                    (1_000, self.count_target / 1_000)
                };
                self.count_step = state;
                self.frame_length = length.min(declared);
            }
            TransitionStyle::TextOnFrame => {
                // Likewise for a label that is already hidden.
                if self.target.is_some_and(|target| is_hidden(shell, target)) {
                    self.forward = true;
                    self.finished = true;
                    self.frame_length = 0;
                    return;
                }
            }
            TransitionStyle::ReverseSound => {
                // Its `init` marks it finished outright: it exists to fire one cue.
                self.forward = true;
                self.finished = true;
                return;
            }
            TransitionStyle::ControlBarArrow => {
                step.diagnostics.push(UiTransitionDiagnostic {
                    window: self.name.clone(),
                    frame: 0,
                    kind: UiTransitionDiagnosticKind::UnsupportedDraw {
                        style: self.style,
                        reason:
                            "the control bar's arrow image is not a WND or mapped-image resource"
                                .to_owned()
                                .into_boxed_str(),
                    },
                });
            }
            _ => {}
        }

        self.forward = false;
        self.apply(shell, 0, step);
        self.finished = false;
        self.forward = true;
        if self.style == TransitionStyle::CountUp {
            self.count_value = 0;
            self.set_text(shell, "0");
        }
    }

    /// Applies one group frame, if it falls inside this window's own window of frames.
    fn update(&mut self, shell: &mut UiShell, frame: i32, step: &mut UiTransitionStep) {
        if frame < self.frame_delay || frame > self.frame_delay + self.frame_length {
            return;
        }
        self.apply(shell, frame - self.frame_delay, step);
    }

    /// Runs one of this style's own frames, numbered from its own start rather than the group's.
    ///
    /// Each arm is the corresponding style's `update` switch: which windows it hides or shows, which
    /// draw state it selects, when it declares itself finished, and which audio cue it fires. A style
    /// whose window did not resolve applies nothing, because every source arm tests the window first.
    #[expect(
        clippy::too_many_lines,
        reason = "fifteen source state machines, each a faithful transcription of one switch"
    )]
    fn apply(&mut self, shell: &mut UiShell, frame: i32, step: &mut UiTransitionStep) {
        self.draw_state = -1;
        let end = self.style.declared_frame_length();
        if frame < 0 || frame > end {
            return;
        }
        let has_target = self.target.is_some();
        let has_companion = self.companion.is_some();
        match self.style {
            TransitionStyle::Flash => {
                if !has_target {
                    return;
                }
                match frame {
                    0 => {
                        if !self.forward {
                            set_hidden(shell, self.target, true);
                            self.finished = true;
                        }
                    }
                    1..=3 => {
                        if frame == 1 && self.forward {
                            self.cue(frame, "GUIBoarderFadeIn", step);
                        }
                        set_hidden(shell, self.target, true);
                        self.draw_state = frame;
                    }
                    4..=7 => {
                        // The wash keeps drawing while the real window is already back.
                        set_hidden(shell, self.target, false);
                        self.draw_state = frame;
                    }
                    _ => {
                        if self.forward {
                            set_hidden(shell, self.target, false);
                            self.finished = true;
                        }
                    }
                }
            }
            TransitionStyle::ButtonFlash => {
                if !has_target {
                    return;
                }
                match frame {
                    0 => {
                        if !self.forward {
                            set_hidden(shell, self.target, true);
                            self.finished = true;
                        }
                    }
                    1..=7 => {
                        if frame == 1 && self.forward {
                            self.cue(frame, "GUIButtonsFadeIn", step);
                        }
                        set_hidden(shell, self.target, true);
                        // Reversing walks the wash back out through its mirror state.
                        self.draw_state = if self.forward { frame } else { 8 - frame };
                    }
                    11..=16 => {
                        // The grade-ins and first two grade-outs hide the window while reversing;
                        // the last two show it either way.
                        let hidden = !self.forward && frame < 15;
                        set_hidden(shell, self.target, hidden);
                        self.draw_state = if self.forward { frame } else { 27 - frame };
                    }
                    17 => {
                        if self.forward {
                            set_hidden(shell, self.target, false);
                            self.finished = true;
                        }
                    }
                    _ => {}
                }
                if frame > 7 && frame < 11 {
                    self.draw_state = BUTTON_FLASH_SHOW_BACKGROUND;
                }
            }
            TransitionStyle::WinFade => {
                if !has_target {
                    return;
                }
                match frame {
                    0 => {
                        if !self.forward {
                            set_hidden(shell, self.target, true);
                            self.finished = true;
                        }
                    }
                    1..=9 => {
                        set_hidden(shell, self.target, true);
                        self.draw_state = frame;
                    }
                    _ => {
                        if self.forward {
                            set_hidden(shell, self.target, false);
                            self.finished = true;
                        }
                    }
                }
            }
            TransitionStyle::WinScaleUp | TransitionStyle::ScoreScaleUp => {
                if !has_target {
                    return;
                }
                let cue = if self.style == TransitionStyle::WinScaleUp {
                    "GUILogoMouseOver"
                } else {
                    "GUIScoreScreenPictures"
                };
                match frame {
                    0 => {
                        if !self.forward {
                            set_hidden(shell, self.target, true);
                            self.finished = true;
                        }
                    }
                    1..=5 => {
                        if frame == 1 && self.forward {
                            self.cue(frame, cue, step);
                        }
                        set_hidden(shell, self.target, true);
                        self.draw_state = frame;
                    }
                    _ => {
                        if self.forward {
                            set_hidden(shell, self.target, false);
                            self.finished = true;
                        }
                    }
                }
            }
            TransitionStyle::MainMenuScaleUp => {
                if !has_target || !has_companion {
                    return;
                }
                if frame == 0 {
                    if !self.forward {
                        // Only the marker is hidden here: the source's hide of the animated window
                        // is commented out.
                        set_hidden(shell, self.companion, true);
                        self.finished = true;
                    }
                } else if frame == end && self.forward {
                    set_hidden(shell, self.target, true);
                    set_hidden(shell, self.companion, false);
                    self.finished = true;
                }
                if frame == 1 {
                    // Unlike its siblings, this cue is not gated on running forwards.
                    self.cue(frame, "GUILogoSelect", step);
                }
                if frame > 0 && frame < end {
                    set_hidden(shell, self.target, true);
                    set_hidden(shell, self.companion, true);
                    self.draw_state = frame;
                }
            }
            TransitionStyle::MainMenuMediumScaleUp => {
                if !has_target || !has_companion {
                    return;
                }
                if frame == 0 {
                    if !self.forward {
                        set_hidden(shell, self.target, false);
                        set_hidden(shell, self.companion, true);
                        self.finished = true;
                    }
                } else if frame == end && self.forward {
                    set_hidden(shell, self.target, true);
                    set_hidden(shell, self.companion, false);
                    self.finished = true;
                }
                if frame > 0 && frame < end {
                    if frame == 1 && self.forward {
                        self.cue(frame, "GUILogoMouseOver", step);
                    }
                    set_hidden(shell, self.target, true);
                    set_hidden(shell, self.companion, true);
                    self.draw_state = frame;
                }
            }
            TransitionStyle::MainMenuSmallScaleDown => {
                if !has_target || !has_companion {
                    return;
                }
                match frame {
                    0 => {
                        if !self.forward {
                            set_hidden(shell, self.target, false);
                            set_hidden(shell, self.companion, true);
                            self.finished = true;
                        }
                    }
                    1..=5 => {
                        set_hidden(shell, self.target, true);
                        set_hidden(shell, self.companion, true);
                        self.draw_state = frame;
                    }
                    _ => {
                        if self.forward {
                            set_hidden(shell, self.target, true);
                            set_hidden(shell, self.companion, false);
                            self.finished = true;
                        }
                    }
                }
            }
            TransitionStyle::TypeText => {
                if !has_target {
                    return;
                }
                if frame == 0 {
                    if !self.forward {
                        set_hidden(shell, self.target, true);
                        self.finished = true;
                    }
                } else if frame == end && self.forward {
                    set_hidden(shell, self.target, false);
                    self.finished = true;
                }
                if frame >= self.frame_length {
                    set_hidden(shell, self.target, false);
                }
                if frame > 0 && frame < self.frame_length {
                    set_hidden(shell, self.target, true);
                    self.draw_state = frame;
                    self.cue(frame, "GUITypeText", step);
                    if self.forward {
                        if let Some(character) = self
                            .full_text
                            .chars()
                            .nth(usize::try_from(frame - 1).unwrap_or(0))
                        {
                            self.partial_text.push(character);
                        }
                    } else {
                        self.partial_text.pop();
                    }
                }
            }
            TransitionStyle::CountUp => {
                if !has_target {
                    return;
                }
                if frame == 0 {
                    if !self.forward {
                        self.count_value = 0;
                        self.set_text(shell, "0");
                        set_hidden(shell, self.target, true);
                        self.finished = true;
                    }
                } else if frame == end && self.forward {
                    set_hidden(shell, self.target, false);
                    self.finished = true;
                }
                if frame >= self.frame_length {
                    set_hidden(shell, self.target, false);
                }
                if frame > 0 && frame < self.frame_length {
                    // A count-up stays visible throughout: it rewrites the label rather than
                    // covering it.
                    set_hidden(shell, self.target, false);
                    self.draw_state = frame;
                    self.cue(frame, "GUIScoreScreenTick", step);
                    self.count_value = self
                        .count_value
                        .saturating_add(self.count_step)
                        .min(self.count_target);
                    let value = self.count_value.to_string();
                    self.set_text(shell, &value);
                }
                if frame == self.frame_length {
                    let full = self.full_text.clone();
                    self.set_text(shell, &full);
                    self.finished = true;
                }
            }
            TransitionStyle::ScreenFade | TransitionStyle::ControlBarArrow => {
                // Neither animates a window: one covers the viewport, the other slides the control
                // bar's own arrow image in.
                if frame == 0 || frame == end {
                    self.finished = true;
                }
                self.draw_state = frame;
            }
            TransitionStyle::FullFade => {
                if !has_target {
                    return;
                }
                if frame == 0 {
                    if !self.forward {
                        set_hidden(shell, self.target, true);
                        self.finished = true;
                    }
                } else if frame == end && self.forward {
                    set_hidden(shell, self.target, false);
                    self.finished = true;
                }
                if frame == end / 2 {
                    // The window swaps in or out under the darkest frame of the fade.
                    set_hidden(shell, self.target, !self.forward);
                }
                self.draw_state = frame;
            }
            TransitionStyle::TextOnFrame => {
                if !has_target {
                    return;
                }
                if frame == 0 {
                    if !self.forward {
                        set_hidden(shell, self.target, true);
                        self.finished = true;
                    }
                } else if self.forward {
                    set_hidden(shell, self.target, false);
                    self.finished = true;
                }
            }
            TransitionStyle::ReverseSound => match frame {
                0 => {
                    if !self.forward {
                        self.finished = true;
                    }
                }
                1 => self.cue(frame, "GUITransitionFade", step),
                _ => {
                    if self.forward {
                        self.finished = true;
                    }
                }
            },
        }
    }

    fn reverse(&mut self, shell: &mut UiShell, step: &mut UiTransitionStep) {
        match self.style {
            TransitionStyle::CountUp => {
                if self.target.is_some_and(|target| is_hidden(shell, target)) {
                    self.forward = false;
                    self.finished = true;
                    self.frame_length = 0;
                    return;
                }
            }
            TransitionStyle::TextOnFrame => {
                self.finished = false;
                self.forward = false;
                if self.target.is_some_and(|target| is_hidden(shell, target)) {
                    self.finished = true;
                    self.frame_length = 0;
                }
                return;
            }
            TransitionStyle::TypeText => {
                self.partial_text = self.full_text.clone();
            }
            TransitionStyle::MainMenuMediumScaleUp => {
                // This one hides both windows as it turns around; the others only flip their flags.
                set_hidden(shell, self.target, true);
                set_hidden(shell, self.companion, true);
            }
            _ => {}
        }
        self.finished = false;
        self.forward = false;
        let _ = step;
    }

    /// Jumps to the end state, which every style implements as `update(END)`.
    fn skip(&mut self, shell: &mut UiShell, step: &mut UiTransitionStep) {
        // Three styles guard the jump on not being finished already.
        if matches!(
            self.style,
            TransitionStyle::CountUp | TransitionStyle::TextOnFrame | TransitionStyle::ReverseSound
        ) && self.finished
        {
            return;
        }
        let end = self.style.declared_frame_length();
        self.apply(shell, end, step);
    }

    /// Emits this window's draw for its current draw state.
    fn emit(&self, draws: &mut Vec<UiTransitionDraw>) {
        let state = self.draw_state;
        if state < 0 {
            return;
        }
        match self.style {
            TransitionStyle::Flash => emit_flash(self, state, draws),
            TransitionStyle::ButtonFlash => emit_button_flash(self, state, draws),
            TransitionStyle::WinFade => emit_win_fade(self, state, draws),
            TransitionStyle::WinScaleUp | TransitionStyle::ScoreScaleUp => {
                emit_centre_scale(self, state, draws);
            }
            TransitionStyle::MainMenuScaleUp => emit_main_menu_scale_up(self, state, draws),
            TransitionStyle::MainMenuMediumScaleUp | TransitionStyle::MainMenuSmallScaleDown => {
                emit_main_menu_expand(self, state, draws);
            }
            TransitionStyle::TypeText => emit_typed_text(self, draws),
            TransitionStyle::ScreenFade => emit_screen_fade(self, state, draws),
            TransitionStyle::FullFade => emit_full_fade(self, state, draws),
            // A count-up writes the control's own text, a text-on-frame only shows it, a reverse
            // sound only fires a cue, and the control-bar arrow's image is unavailable here.
            TransitionStyle::CountUp
            | TransitionStyle::TextOnFrame
            | TransitionStyle::ReverseSound
            | TransitionStyle::ControlBarArrow => {}
        }
    }

    fn set_text(&self, shell: &mut UiShell, text: &str) {
        if let Some(target) = self.target
            && let Some(layout) = shell.layout_mut(target.screen)
        {
            layout.set_text_label(target.control, text);
        }
    }

    fn cue(&self, frame: i32, event: &str, step: &mut UiTransitionStep) {
        step.diagnostics.push(UiTransitionDiagnostic {
            window: self.name.clone(),
            frame,
            kind: UiTransitionDiagnosticKind::AudioCue {
                event: event.to_owned().into_boxed_str(),
            },
        });
    }
}

/// Hides or shows one resolved target, if it resolved at all.
fn set_hidden(shell: &mut UiShell, target: Option<UiTransitionTarget>, hidden: bool) {
    if let Some(target) = target
        && let Some(layout) = shell.layout_mut(target.screen)
    {
        layout.set_hidden(target.control, hidden);
    }
}

/// Resolves a decorated window name to a transition target.
fn resolve(shell: &UiShell, name: &str) -> Option<UiTransitionTarget> {
    shell
        .find_control_by_decorated_name(name)
        .map(|(screen, control)| UiTransitionTarget { screen, control })
}

/// Returns the shell's viewport rectangle, which the whole-screen fades cover.
fn viewport_rect(shell: &UiShell) -> UiRect {
    shell.screens().first().map_or(
        UiRect {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        },
        |screen| {
            let viewport = screen.layout().presentation().viewport;
            UiRect {
                x: 0,
                y: 0,
                width: viewport.width(),
                height: viewport.height(),
            }
        },
    )
}

fn screen_rect(shell: &UiShell, target: UiTransitionTarget) -> UiRect {
    shell.layout(target.screen).map_or(
        UiRect {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        },
        |layout| layout.screen_rect(target.control),
    )
}

fn is_hidden(shell: &UiShell, target: UiTransitionTarget) -> bool {
    shell
        .layout(target.screen)
        .is_some_and(|layout| layout.control(target.control).is_hidden())
}

fn control_text(shell: &UiShell, target: UiTransitionTarget) -> String {
    shell
        .layout(target.screen)
        .and_then(|layout| layout.control(target.control).displayed_text())
        .unwrap_or_default()
        .to_owned()
}

/// Reads the leading integer of a label, as `atoi` does for the count-up's target.
fn leading_integer(text: &str) -> i32 {
    let trimmed = text.trim_start();
    let (negative, digits) = match trimmed.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
    };
    let end = digits
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(digits.len());
    let value = digits[..end].parse::<i64>().unwrap_or(0);
    let signed = if negative { -value } else { value };
    i32::try_from(signed).unwrap_or(0)
}

const fn white(alpha: u8) -> WndColor {
    WndColor::from_rgba(255, 255, 255, alpha)
}

const fn black(alpha: u8) -> WndColor {
    WndColor::from_rgba(0, 0, 0, alpha)
}

fn emit_flash(transition: &UiTransition, state: i32, draws: &mut Vec<UiTransitionDraw>) {
    // The rect is inset by one pixel and narrowed by two, and its height is left alone — the source
    // passes `m_size.y` where every other axis is adjusted.
    let rect = UiRect {
        x: transition.rect.x + 1,
        y: transition.rect.y + 1,
        width: transition.rect.width - 2,
        height: transition.rect.height,
    };
    let (outline, fill) = match state {
        1 => (100, 33),
        2 => (150, 66),
        3 => (200, 99),
        4 => (250, 75),
        5 => (250, 50),
        6 => (250, 25),
        7 => (250, 10),
        _ => return,
    };
    draws.push(UiTransitionDraw::Rect {
        rect,
        fill: Some(white(fill)),
        outline: Some(white(outline)),
        outline_width: 1,
    });
}

fn emit_button_flash(transition: &UiTransition, state: i32, draws: &mut Vec<UiTransitionDraw>) {
    let Some(target) = transition.target else {
        return;
    };
    let rect = transition.rect;
    // Frames 8 to 10 fall between the two halves of the sequence and only show the button's art.
    if state == BUTTON_FLASH_SHOW_BACKGROUND {
        draws.push(UiTransitionDraw::PushButtonPieces { target, alpha: 255 });
        return;
    }
    if let Some((outline, fill)) = match state {
        1 => Some((100, 75)),
        2 => Some((150, 150)),
        3 => Some((200, 200)),
        4 => Some((250, 150)),
        5 => Some((250, 100)),
        6 => Some((250, 50)),
        7 => Some((250, 15)),
        _ => None,
    } {
        // The four fade-to-background states show the button's own art under the wash.
        if state >= 4 {
            draws.push(UiTransitionDraw::PushButtonPieces { target, alpha: 255 });
        }
        draws.push(UiTransitionDraw::Rect {
            rect,
            fill: Some(white(fill)),
            outline: Some(white(outline)),
            outline_width: 1,
        });
        return;
    }
    // The gradient rises over the two grade-ins and falls over the four grade-outs, which is why the
    // first grade-in and the second grade-out share an alpha.
    if let Some(alpha) = match state {
        11 | 14 => Some(100),
        12 => Some(200),
        13 => Some(150),
        15 => Some(50),
        16 => Some(17),
        _ => None,
    } {
        // Going forward the art appears under the first grade-in; reversing, under each grade-out.
        let art = if transition.forward {
            state == 11
        } else {
            (13..=16).contains(&state)
        };
        if art {
            draws.push(UiTransitionDraw::PushButtonPieces { target, alpha: 255 });
        }
        draws.push(UiTransitionDraw::NamedImage {
            image: "Gradient".to_owned().into_boxed_str(),
            rect,
            color: white(alpha),
        });
    }
}

/// `BUTTONFLASHTRANSITION_SHOW_BACKGROUND`, one past the sequence's end.
const BUTTON_FLASH_SHOW_BACKGROUND: i32 = 18;

fn emit_win_fade(transition: &UiTransition, state: i32, draws: &mut Vec<UiTransitionDraw>) {
    let Some(target) = transition.target else {
        return;
    };
    if state <= 0 || state >= transition.style.declared_frame_length() {
        return;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "state is 1..=9, so 25 * state is 25..=225"
    )]
    let alpha = (25 * state) as u8;
    draws.push(UiTransitionDraw::ControlImage {
        target,
        slot: WndDrawDataSlot::Enabled,
        entry: 0,
        rect: transition.rect,
        color: white(alpha),
    });
}

/// The two centre-growing styles: a rect centred on the control, sized by whole increments of
/// `size / length` so the last frame lands one increment short of full size.
fn emit_centre_scale(transition: &UiTransition, state: i32, draws: &mut Vec<UiTransitionDraw>) {
    let Some(target) = transition.target else {
        return;
    };
    let length = transition.style.declared_frame_length();
    if state <= 0 || state >= length {
        return;
    }
    let centre_x = transition.rect.x + transition.rect.width / 2;
    let centre_y = transition.rect.y + transition.rect.height / 2;
    let increment_x = transition.rect.width / length;
    let increment_y = transition.rect.height / length;
    let width = increment_x * state;
    let height = increment_y * state;
    draws.push(UiTransitionDraw::ControlImage {
        target,
        slot: WndDrawDataSlot::Enabled,
        entry: 0,
        rect: UiRect {
            x: centre_x - width / 2,
            y: centre_y - height / 2,
            width,
            height,
        },
        color: white(255),
    });
}

/// `MAINMENUSCALEUP` slides and grows from the animated window's rect toward the marker's.
fn emit_main_menu_scale_up(
    transition: &UiTransition,
    state: i32,
    draws: &mut Vec<UiTransitionDraw>,
) {
    let (Some(_target), Some(companion)) = (transition.target, transition.companion) else {
        return;
    };
    let length = transition.style.declared_frame_length();
    if state <= 0 || state >= length {
        return;
    }
    let increment_x = (transition.companion_rect.x - transition.rect.x) / length;
    let increment_y = (transition.companion_rect.y - transition.rect.y) / length;
    let grow_x = (transition.companion_rect.width - transition.rect.width) / length;
    let grow_y = (transition.companion_rect.height - transition.rect.height) / length;
    // The marker carries the image: `init` copies the animated window's *disabled* art onto it.
    draws.push(UiTransitionDraw::ControlImage {
        target: companion,
        slot: WndDrawDataSlot::Enabled,
        entry: 0,
        rect: UiRect {
            x: transition.rect.x + increment_x * state,
            y: transition.rect.y + increment_y * state,
            width: transition.rect.width + grow_x * state,
            height: transition.rect.height + grow_y * state,
        },
        color: white(255),
    });
}

/// The medium and small main-menu styles expand the animated window's own art about its centre by
/// half an increment per side, toward or away from the companion's size.
fn emit_main_menu_expand(transition: &UiTransition, state: i32, draws: &mut Vec<UiTransitionDraw>) {
    let Some(target) = transition.target else {
        return;
    };
    if transition.companion.is_none() {
        return;
    }
    let length = transition.style.declared_frame_length();
    if state <= 0 || state >= length {
        return;
    }
    let grow_x = (transition.companion_rect.width - transition.rect.width) / length;
    let grow_y = (transition.companion_rect.height - transition.rect.height) / length;
    let inset_x = grow_x * state / 2;
    let inset_y = grow_y * state / 2;
    draws.push(UiTransitionDraw::ControlImage {
        target,
        slot: WndDrawDataSlot::Enabled,
        entry: 0,
        rect: UiRect {
            x: transition.rect.x - inset_x,
            y: transition.rect.y - inset_y,
            width: transition.rect.width + inset_x * 2,
            height: transition.rect.height + inset_y * 2,
        },
        color: white(255),
    });
}

fn emit_typed_text(transition: &UiTransition, draws: &mut Vec<UiTransitionDraw>) {
    let Some(target) = transition.target else {
        return;
    };
    if transition.draw_state <= 0 || transition.draw_state >= transition.frame_length {
        return;
    }
    draws.push(UiTransitionDraw::TypedText {
        target,
        text: transition.partial_text.clone(),
    });
}

fn emit_screen_fade(transition: &UiTransition, state: i32, draws: &mut Vec<UiTransitionDraw>) {
    // The ramp divides by one less than the length, so the last frame is fully opaque.
    let length = transition.style.declared_frame_length();
    let alpha = ramp_alpha(state, length - 1);
    draws.push(UiTransitionDraw::Rect {
        rect: transition.viewport,
        fill: Some(black(alpha)),
        outline: None,
        outline_width: 0,
    });
}

fn emit_full_fade(transition: &UiTransition, state: i32, draws: &mut Vec<UiTransitionDraw>) {
    let length = transition.style.declared_frame_length();
    let half = length / 2;
    let steps = if state > half { length - state } else { state };
    let alpha = ramp_alpha(steps, half);
    draws.push(UiTransitionDraw::Rect {
        rect: transition.rect,
        fill: Some(black(alpha)),
        outline: Some(WndColor::from_rgba(60, 60, 180, alpha)),
        outline_width: 1,
    });
}

/// Returns `255 * steps / total`, clamped, matching the source's `percent * 255 * state` with
/// `percent = 1 / total`.
fn ramp_alpha(steps: i32, total: i32) -> u8 {
    if total <= 0 {
        return 0;
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "frame counts are far inside f32's exact integer range"
    )]
    let value = (255.0 / total as f32) * steps as f32;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped into 0..=255 before truncating, as the source clamps to 255"
    )]
    {
        value.clamp(0.0, 255.0) as u8
    }
}
