//! The navigable shell: the screens, what is open, what is staged, and where an action goes.
//!
//! # What this adds over the pieces it holds
//!
//! [`ScreenStack`] knows how to move between screens and [`Transaction`] knows how to apply a setting
//! that can undo itself, but neither knows that they belong to each other. The routing between them is
//! where the interesting rules live, and all three of them are rules about *leaving*:
//!
//! - Applying a settings change must not move the stack, because the revert window is only useful
//!   while the confirm button is somewhere the user can reach.
//! - **Closing the settings screen with a change unconfirmed reverts it.** Nobody is going to confirm
//!   a change on a screen that is no longer open, so the countdown would fire minutes later somewhere
//!   else — and a user who navigated away without confirming has already said what they meant.
//! - Going back from the root asks whether to leave the game rather than doing nothing. Escape on the
//!   main menu meaning *nothing at all* is the one response that reads as a broken key.
//!
//! # Why the outcome is a struct and not an enum
//!
//! One event produces at most one [`Action`] — that much [`Interface`] guarantees. But one action can
//! genuinely do two things a host must react to: going back from settings both navigates *and* puts
//! different settings in force. An enum would force a choice between reporting the navigation and
//! reporting the revert, and whichever was dropped would be a bug in a host that needed it.
//!
//! # Why the solved layouts live here
//!
//! Input routing needs a solved layout and so does drawing, they need the same one, and the layout to
//! solve depends on which screens are open. Keeping them beside the stack means there is one place that
//! rebuilds them and one set of events that invalidates them — construction, a viewport change, and a
//! transition. Held by a host instead, the third is the one that gets forgotten, and the symptom is
//! clicks landing on the screen that *was* on top.

use crate::geometry::{Rect, Viewport};
use crate::layout::Layout;
use crate::screen::{Screen, ScreenStack, Screens, Transition};
use crate::settings::{Probation, Transaction};
use crate::solve::{Measure, Solved, solve};
use crate::state::Interface;
use crate::transition::{Motion, Reveal};
use crate::{Action, StringTable, UiEvent};

/// Something the host has to do that the shell cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    /// Leave the game.
    Quit,
    /// Start the skirmish the setup screen describes.
    LaunchSkirmish,
}

/// Everything one event changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcome {
    /// What happened to the screen stack.
    pub transition: Transition,
    /// Whether the settings in force changed, so the host must hand them to the device again.
    pub settings_in_force: bool,
    /// What the host must do itself.
    pub request: Option<Request>,
}

impl Outcome {
    /// Nothing happened.
    pub const IDLE: Self = Self {
        transition: Transition::Unchanged,
        settings_in_force: false,
        request: None,
    };

    /// Whether there is nothing for the host to act on.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        *self == Self::IDLE
    }
}

/// Why a shell could not be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellError {
    /// Screens the catalogue had no layout for.
    ///
    /// Refused at construction rather than tolerated, because a screen with no layout is a button that
    /// navigates to a blank surface — which reads as an unfinished screen rather than as a missing file.
    MissingLayouts(Vec<Screen>),
}

impl std::fmt::Display for ShellError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingLayouts(screens) => {
                write!(formatter, "no layout for")?;
                for screen in screens {
                    write!(formatter, " {}", screen.slug())?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for ShellError {}

/// The shell: which screens are open, what they remember, and what is staged.
///
/// Generic over the host's settings type for the reason [`Transaction`] is: this crate must not know
/// what a display setting is.
#[derive(Debug, Clone, PartialEq)]
pub struct Shell<T> {
    screens: Screens,
    strings: StringTable,
    stack: ScreenStack,
    settings: Transaction<T>,
    viewport: Viewport,
    drawn: Vec<(Screen, Solved)>,
}

impl<T: Clone + PartialEq> Shell<T> {
    /// Builds a shell at its root screen and solves it once.
    ///
    /// # Errors
    ///
    /// Returns [`ShellError::MissingLayouts`] when the catalogue does not cover every screen. Missing
    /// *strings* are not an error — a key renders as itself so a gap names its own fix — and are
    /// reported by [`Self::missing_strings`] for a loader or a test to check.
    pub fn new(
        screens: Screens,
        strings: StringTable,
        settings: T,
        viewport: Viewport,
        measure: &impl Measure,
    ) -> Result<Self, ShellError> {
        Self::with_motion(
            screens,
            strings,
            settings,
            viewport,
            Motion::INSTANT,
            measure,
        )
    }

    /// Builds a shell whose screens animate as they change.
    ///
    /// # Errors
    ///
    /// As [`Self::new`].
    pub fn with_motion(
        screens: Screens,
        strings: StringTable,
        settings: T,
        viewport: Viewport,
        motion: Motion,
        measure: &impl Measure,
    ) -> Result<Self, ShellError> {
        let missing = screens.missing();
        if !missing.is_empty() {
            return Err(ShellError::MissingLayouts(missing));
        }
        let mut shell = Self {
            screens,
            strings,
            stack: ScreenStack::with_motion(Screen::MainMenu, motion),
            settings: Transaction::new(settings),
            viewport,
            drawn: Vec::new(),
        };
        shell.resolve(measure);
        Ok(shell)
    }

    /// The screen taking input, which is the one drawn last.
    #[must_use]
    pub fn top(&self) -> Screen {
        self.stack.top()
    }

    /// The screen stack, for a host that wants to know what is open beneath the top.
    #[must_use]
    pub const fn stack(&self) -> &ScreenStack {
        &self.stack
    }

    /// The top screen's retained state.
    #[must_use]
    pub fn interface(&self) -> &Interface {
        self.stack.interface()
    }

    /// The top screen's retained state, for a host to seed values into or read them out of.
    pub fn interface_mut(&mut self) -> &mut Interface {
        self.stack.interface_mut()
    }

    /// The display text.
    #[must_use]
    pub const fn strings(&self) -> &StringTable {
        &self.strings
    }

    /// The settings transaction: what is in force, what is staged, and what is unconfirmed.
    #[must_use]
    pub const fn settings(&self) -> &Transaction<T> {
        &self.settings
    }

    /// The settings transaction, for a widget to stage a change into.
    pub const fn settings_mut(&mut self) -> &mut Transaction<T> {
        &mut self.settings
    }

    /// The surface the shell is solved against.
    #[must_use]
    pub const fn viewport(&self) -> Viewport {
        self.viewport
    }

    /// The solved layout of every screen currently involved in drawing, outermost first.
    ///
    /// The cache [`Self::frames`] reads from, rebuilt only when the viewport or the stack changes — a
    /// reveal changes every frame and a solved layout does not. More than one entry when a modal is open
    /// or a screen change is in flight, since both mean two screens are on screen at once.
    ///
    /// A host draws from [`Self::frames`] rather than this, because this says nothing about how much of
    /// each screen is showing.
    #[must_use]
    pub fn drawn(&self) -> &[(Screen, Solved)] {
        &self.drawn
    }

    /// The top screen's solved layout: what input is routed against and what is drawn last.
    #[must_use]
    pub fn solved(&self) -> Option<&Solved> {
        self.drawn.last().map(|(_, solved)| solved)
    }

    /// One screen's authored layout.
    #[must_use]
    pub fn layout(&self, screen: Screen) -> Option<&Layout> {
        self.screens.get(screen)
    }

    /// Text keys any screen names that the string table does not define, sorted and deduplicated.
    #[must_use]
    pub fn missing_strings(&self) -> Vec<&str> {
        self.screens.missing_strings(&self.strings)
    }

    /// Whether an input method should be enabled, for a host to drive `set_ime_allowed` from.
    #[must_use]
    pub fn ime_wanted(&self) -> bool {
        self.solved()
            .is_some_and(|solved| self.interface().ime_wanted(solved))
    }

    /// Where an input method should put its candidate window, for `set_ime_cursor_area`.
    #[must_use]
    pub fn ime_cursor_area(&self) -> Option<Rect> {
        self.interface().ime_cursor_area(self.solved()?)
    }

    /// Re-solves every open screen against a new surface.
    ///
    /// Retained state survives, because it is keyed by the ids the layout author wrote rather than by
    /// anything a re-solve replaces.
    pub fn resize(&mut self, viewport: Viewport, measure: &impl Measure) {
        self.viewport = viewport;
        self.resolve(measure);
    }

    /// Advances the settings revert window **and** any screen change in flight.
    ///
    /// `elapsed` is in seconds from the host's own clock — see [`crate::settings`] for why it must not
    /// come from scene time. A host calls this every frame whether or not a settings screen is open;
    /// the idle path does nothing.
    ///
    /// One call drives both clocks deliberately. Two methods to call each frame is one that eventually
    /// stops being called, and the failure would be silent in both directions: a revert window that never
    /// fires, or an interface stuck half-faded between two screens.
    pub fn tick(&mut self, elapsed: f32) -> Probation {
        self.stack.advance(elapsed);
        self.settings.tick(elapsed)
    }

    /// Whether a screen change is still animating, which tells a host to keep redrawing.
    #[must_use]
    pub fn is_changing(&self) -> bool {
        self.stack.is_changing()
    }

    /// How screens change.
    #[must_use]
    pub const fn motion(&self) -> Motion {
        self.stack.motion()
    }

    /// Replaces how screens change, which is what a reduce-motion setting drives.
    pub const fn set_motion(&mut self, motion: Motion) {
        self.stack.set_motion(motion);
    }

    /// Every screen to draw right now, with its solved layout and how much of it shows.
    ///
    /// What a host paints. Outermost first, so drawing them in order puts a modal over its backdrop and a
    /// screen arriving over the one it is replacing.
    #[must_use]
    pub fn frames(&self) -> Vec<(Screen, Reveal, &Solved)> {
        self.stack
            .frames()
            .into_iter()
            .filter_map(|(screen, reveal)| {
                let solved = self
                    .drawn
                    .iter()
                    .find(|(candidate, _)| *candidate == screen)
                    .map(|(_, solved)| solved)?;
                Some((screen, reveal, solved))
            })
            .collect()
    }

    /// Routes one event and reports everything it changed.
    pub fn handle(&mut self, event: UiEvent, measure: &impl Measure) -> Outcome {
        let Some((_, solved)) = self.drawn.last() else {
            return Outcome::IDLE;
        };
        let Some(action) = self.stack.interface_mut().handle(solved, event) else {
            return Outcome::IDLE;
        };
        self.act(action, measure)
    }

    /// Runs one action as though a control had raised it.
    ///
    /// Public because a host has bindings of its own — a key that opens settings, a title-screen
    /// timeout — and they should reach the same routing a button does rather than a second copy of it.
    pub fn act(&mut self, action: Action, measure: &impl Measure) -> Outcome {
        match action {
            Action::Quit => Outcome {
                request: Some(Request::Quit),
                ..Outcome::IDLE
            },
            Action::LaunchSkirmish => Outcome {
                request: Some(Request::LaunchSkirmish),
                ..Outcome::IDLE
            },
            Action::ApplySettings => Outcome {
                settings_in_force: self.settings.apply(),
                ..Outcome::IDLE
            },
            Action::ConfirmSettings => {
                // Confirming changes nothing about what is in force; it only stops the countdown that
                // would have taken it away.
                self.settings.confirm();
                Outcome::IDLE
            }
            Action::RevertSettings => Outcome {
                settings_in_force: self.settings.revert(),
                ..Outcome::IDLE
            },
            _ => self.navigate(action, measure),
        }
    }

    /// Moves the stack, re-solves, and reverts a settings change the move abandoned.
    fn navigate(&mut self, action: Action, measure: &impl Measure) -> Outcome {
        let was_open = self.stack.contains(Screen::Settings);
        let mut transition = self.stack.apply(action);
        // Nothing beneath the root, so the shell asks the question the root cannot answer. Escape on
        // the main menu doing nothing at all is the one response that reads as a broken key.
        if transition == Transition::AtRoot {
            transition = self.stack.push(Screen::QuitConfirm);
        }
        if transition == Transition::Unchanged {
            return Outcome::IDLE;
        }
        // A change nobody confirmed on a screen that is no longer open would revert minutes later
        // somewhere else. Navigating away without confirming already said what the user meant.
        let abandoned = was_open && !self.stack.contains(Screen::Settings);
        let settings_in_force = abandoned && self.settings.revert();
        self.resolve(measure);
        Outcome {
            transition,
            settings_in_force,
            request: None,
        }
    }

    /// Rebuilds the solved layout of every screen involved in what is drawn.
    ///
    /// Solved against [`ScreenStack::involved`] rather than `drawn`, so a screen on its way out has a
    /// layout for as long as it is still being drawn. Cached rather than re-solved per frame because a
    /// reveal changes every frame and a solved layout does not.
    fn resolve(&mut self, measure: &impl Measure) {
        let screens = self.stack.involved();
        let mut drawn = Vec::with_capacity(screens.len());
        for screen in screens {
            if let Some(layout) = self.screens.get(screen) {
                drawn.push((screen, solve(layout, self.viewport, measure)));
            }
        }
        self.drawn = drawn;
    }
}

#[cfg(test)]
mod tests {
    // The countdown values below are the revert window and whole seconds of it, all exact in binary.
    #![allow(clippy::float_cmp)]

    use super::{Outcome, Request, Shell, ShellError};
    use crate::layout::{FORMAT_VERSION, Layout, Node, Sizing, Widget};
    use crate::screen::{Screen, Screens, Transition};
    use crate::settings::{Probation, REVERT_WINDOW};
    use crate::solve::NoContent;
    use crate::transition::Motion;
    use crate::{Action, StringTable, UiEvent, Viewport};

    /// Stands in for a host's display settings.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct Display {
        scale: u32,
    }

    /// A screen with one full-surface button, so a click at any point hits it.
    fn screen_layout(id: &str, action: Action) -> Layout {
        Layout {
            format_version: FORMAT_VERSION,
            root: Node {
                width: Sizing::Fill(1),
                height: Sizing::Fill(1),
                children: vec![Node {
                    id: Some(id.to_owned()),
                    widget: Widget::Button,
                    width: Sizing::Fill(1),
                    height: Sizing::Fill(1),
                    action: Some(action),
                    ..Node::default()
                }],
                ..Node::default()
            },
        }
    }

    fn shell() -> Shell<Display> {
        let mut screens = Screens::new();
        screens.insert(Screen::MainMenu, screen_layout("go", Action::OpenSettings));
        screens.insert(
            Screen::Settings,
            screen_layout("apply", Action::ApplySettings),
        );
        screens.insert(
            Screen::SkirmishSetup,
            screen_layout("launch", Action::LaunchSkirmish),
        );
        screens.insert(Screen::QuitConfirm, screen_layout("yes", Action::Quit));
        Shell::new(
            screens,
            StringTable::new(),
            Display { scale: 100 },
            Viewport::new(800, 600, 1.0).expect("viewport"),
            &NoContent,
        )
        .expect("every screen has a layout")
    }

    /// A press and release on the same point, which is what fires a button.
    fn click(shell: &mut Shell<Display>, x: f32, y: f32) -> Outcome {
        shell.handle(UiEvent::PointerPressed { x, y }, &NoContent);
        shell.handle(UiEvent::PointerReleased { x, y }, &NoContent)
    }

    #[test]
    fn a_catalogue_with_a_gap_is_refused_at_construction() {
        // A screen with no layout is a button that navigates to a blank surface, which reads as an
        // unfinished screen rather than as a missing file.
        let mut screens = Screens::new();
        screens.insert(Screen::MainMenu, screen_layout("go", Action::Quit));
        let refused = Shell::new(
            screens,
            StringTable::new(),
            Display { scale: 100 },
            Viewport::new(800, 600, 1.0).expect("viewport"),
            &NoContent,
        );
        assert_eq!(
            refused,
            Err(ShellError::MissingLayouts(vec![
                Screen::Settings,
                Screen::SkirmishSetup,
                Screen::QuitConfirm
            ]))
        );
    }

    #[test]
    fn a_shell_opens_at_the_main_menu_with_it_solved() {
        let shell = shell();
        assert_eq!(shell.top(), Screen::MainMenu);
        assert_eq!(shell.drawn().len(), 1);
        assert_eq!(shell.solved().expect("solved").len(), 2);
        assert!(shell.missing_strings().is_empty());
    }

    #[test]
    fn clicking_a_navigating_button_moves_the_stack_and_re_solves() {
        // The invalidation a host would forget. Without the re-solve, clicks keep landing on the screen
        // that *was* on top.
        let mut shell = shell();
        let outcome = click(&mut shell, 400.0, 300.0);
        assert_eq!(outcome.transition, Transition::Pushed(Screen::Settings));
        assert_eq!(shell.top(), Screen::Settings);
        // The new screen's own control is what a further click reaches.
        assert_eq!(
            shell
                .solved()
                .and_then(|solved| solved.by_id("apply"))
                .map(|node| node.widget),
            Some(Widget::Button)
        );
    }

    #[test]
    fn applying_settings_stays_on_the_settings_screen() {
        // Load-bearing: the revert window is only useful while the confirm button is reachable.
        let mut shell = shell();
        shell.act(Action::OpenSettings, &NoContent);
        shell.settings_mut().staged_mut().scale = 200;
        let outcome = click(&mut shell, 400.0, 300.0);
        assert!(
            outcome.settings_in_force,
            "the host must reapply to the device"
        );
        assert_eq!(outcome.transition, Transition::Unchanged);
        assert_eq!(shell.top(), Screen::Settings);
        assert_eq!(shell.settings().in_force().scale, 200);
        assert_eq!(shell.settings().remaining(), Some(REVERT_WINDOW));
    }

    #[test]
    fn an_unconfirmed_change_reverts_itself_while_the_screen_stays_open() {
        // The whole reason the transaction exists, driven through the shell as a host would.
        let mut shell = shell();
        shell.act(Action::OpenSettings, &NoContent);
        shell.settings_mut().staged_mut().scale = 200;
        shell.act(Action::ApplySettings, &NoContent);
        assert_eq!(shell.tick(5.0), Probation::Pending(REVERT_WINDOW - 5.0));
        assert_eq!(shell.tick(REVERT_WINDOW), Probation::Lapsed);
        assert_eq!(shell.settings().in_force().scale, 100);
        assert_eq!(shell.top(), Screen::Settings);
    }

    #[test]
    fn confirming_keeps_the_change_and_stops_the_countdown() {
        let mut shell = shell();
        shell.act(Action::OpenSettings, &NoContent);
        shell.settings_mut().staged_mut().scale = 200;
        shell.act(Action::ApplySettings, &NoContent);
        let outcome = shell.act(Action::ConfirmSettings, &NoContent);
        assert!(
            outcome.is_idle(),
            "confirming changes nothing about what is in force"
        );
        assert_eq!(shell.tick(REVERT_WINDOW * 2.0), Probation::Settled);
        assert_eq!(shell.settings().in_force().scale, 200);
    }

    #[test]
    fn closing_the_settings_screen_reverts_a_change_nobody_confirmed() {
        // Nobody is going to confirm a change on a screen that is no longer open, so the countdown
        // would fire minutes later somewhere else. Navigating away without confirming already said
        // what the user meant.
        let mut shell = shell();
        shell.act(Action::OpenSettings, &NoContent);
        shell.settings_mut().staged_mut().scale = 200;
        shell.act(Action::ApplySettings, &NoContent);
        let outcome = shell.act(Action::Back, &NoContent);
        assert_eq!(outcome.transition, Transition::Popped(Screen::MainMenu));
        assert!(
            outcome.settings_in_force,
            "the host must reapply to the device"
        );
        assert_eq!(shell.settings().in_force().scale, 100);
        assert!(!shell.settings().is_unconfirmed());
    }

    #[test]
    fn a_confirmed_change_survives_closing_the_screen() {
        let mut shell = shell();
        shell.act(Action::OpenSettings, &NoContent);
        shell.settings_mut().staged_mut().scale = 200;
        shell.act(Action::ApplySettings, &NoContent);
        shell.act(Action::ConfirmSettings, &NoContent);
        let outcome = shell.act(Action::Back, &NoContent);
        assert!(!outcome.settings_in_force);
        assert_eq!(shell.settings().in_force().scale, 200);
    }

    #[test]
    fn a_modal_over_settings_does_not_count_as_closing_it() {
        // Only closing the screen abandons the change. Covering it does not, and reverting there would
        // take away a setting the user is still looking at.
        let mut shell = shell();
        shell.act(Action::OpenSettings, &NoContent);
        shell.settings_mut().staged_mut().scale = 200;
        shell.act(Action::ApplySettings, &NoContent);
        let outcome = shell.act(Action::OpenQuitConfirm, &NoContent);
        assert_eq!(outcome.transition, Transition::Pushed(Screen::QuitConfirm));
        assert!(!outcome.settings_in_force);
        assert_eq!(shell.settings().in_force().scale, 200);
        assert!(shell.settings().is_unconfirmed());
        // Both are drawn, because the modal covers only part of the surface.
        assert_eq!(
            shell
                .drawn()
                .iter()
                .map(|(screen, _)| *screen)
                .collect::<Vec<_>>(),
            vec![Screen::Settings, Screen::QuitConfirm]
        );
    }

    #[test]
    fn escape_at_the_root_asks_whether_to_leave_rather_than_doing_nothing() {
        // Escape on the main menu meaning nothing at all is the one response that reads as a broken key.
        let mut shell = shell();
        let outcome = shell.handle(UiEvent::Cancel, &NoContent);
        assert_eq!(outcome.transition, Transition::Pushed(Screen::QuitConfirm));
        assert_eq!(shell.top(), Screen::QuitConfirm);
        // And the modal's own button is what answers it.
        let answered = click(&mut shell, 400.0, 300.0);
        assert_eq!(answered.request, Some(Request::Quit));
    }

    #[test]
    fn escape_out_of_a_modal_returns_to_the_screen_it_covered() {
        let mut shell = shell();
        shell.act(Action::OpenSettings, &NoContent);
        shell.act(Action::OpenQuitConfirm, &NoContent);
        let outcome = shell.handle(UiEvent::Cancel, &NoContent);
        assert_eq!(outcome.transition, Transition::Popped(Screen::Settings));
        assert_eq!(shell.drawn().len(), 1);
    }

    #[test]
    fn what_a_host_must_do_itself_is_reported_rather_than_attempted() {
        let mut shell = shell();
        shell.act(Action::OpenSkirmishSetup, &NoContent);
        let outcome = click(&mut shell, 400.0, 300.0);
        assert_eq!(outcome.request, Some(Request::LaunchSkirmish));
        // The shell stays where it was: what to do about a launch is M6's business.
        assert_eq!(shell.top(), Screen::SkirmishSetup);
    }

    #[test]
    fn a_resize_re_solves_every_drawn_screen_and_keeps_retained_state() {
        let mut shell = shell();
        shell.act(Action::OpenSettings, &NoContent);
        shell.act(Action::OpenQuitConfirm, &NoContent);
        shell.interface_mut().set_toggle("remember", true);
        shell.resize(
            Viewport::new(1920, 1080, 2.0).expect("viewport"),
            &NoContent,
        );
        assert_eq!(shell.viewport().width(), 1920);
        for (_, solved) in shell.drawn() {
            assert_eq!(solved.nodes()[0].rect.width, 1920.0);
        }
        assert_eq!(shell.interface().toggle("remember"), Some(true));
    }

    #[test]
    fn one_tick_drives_both_the_revert_window_and_a_screen_change() {
        // Two methods to call each frame is one that eventually stops being called, and the failure would
        // be silent in both directions: a revert window that never fires, or an interface stuck half-faded
        // between two screens.
        let mut screens = Screens::new();
        for screen in Screen::ALL {
            screens.insert(*screen, screen_layout(screen.slug(), Action::Back));
        }
        let mut shell = Shell::with_motion(
            screens,
            StringTable::new(),
            Display { scale: 100 },
            Viewport::new(800, 600, 1.0).expect("viewport"),
            Motion::DEFAULT,
            &NoContent,
        )
        .expect("every screen has a layout");

        shell.act(Action::OpenSettings, &NoContent);
        shell.settings_mut().staged_mut().scale = 200;
        shell.act(Action::ApplySettings, &NoContent);
        assert!(shell.is_changing(), "the screen change is in flight");

        // One call advances both, and neither is finished after a fraction of a second.
        assert!(matches!(
            shell.tick(Motion::DEFAULT.duration / 2.0),
            Probation::Pending(_)
        ));
        assert!(shell.is_changing());
        // The change finishes long before the revert window does.
        assert!(matches!(
            shell.tick(Motion::DEFAULT.duration),
            Probation::Pending(_)
        ));
        assert!(!shell.is_changing());
        assert!(shell.settings().is_unconfirmed());
    }

    #[test]
    fn a_change_in_flight_gives_a_host_the_departing_screen_solved() {
        // What a host paints. Without the layout it cannot draw the screen the stack is keeping alive, and
        // keeping the screen without the layout would be state that cannot be used.
        let mut screens = Screens::new();
        for screen in Screen::ALL {
            screens.insert(*screen, screen_layout(screen.slug(), Action::Back));
        }
        let mut shell = Shell::with_motion(
            screens,
            StringTable::new(),
            Display { scale: 100 },
            Viewport::new(800, 600, 1.0).expect("viewport"),
            Motion::DEFAULT,
            &NoContent,
        )
        .expect("every screen has a layout");
        shell.act(Action::OpenSettings, &NoContent);
        let frames = shell.frames();
        assert_eq!(frames.len(), 2, "the one leaving and the one arriving");
        assert_eq!(frames[0].0, Screen::MainMenu);
        assert_eq!(frames[1].0, Screen::Settings);
        assert!(frames[1].1.is_hidden(), "the arriving screen starts hidden");
        for (screen, _, solved) in shell.frames() {
            assert!(
                !solved.is_empty(),
                "{screen:?} was handed over without a layout"
            );
        }
        // Once it settles there is one frame, fully shown.
        shell.tick(Motion::DEFAULT.duration);
        let frames = shell.frames();
        assert_eq!(frames.len(), 1);
        assert!(frames[0].1.is_shown());
    }

    #[test]
    fn a_shell_with_no_motion_reports_one_fully_shown_frame() {
        // The default, and what every capture reference was rendered through.
        let mut shell = shell();
        shell.act(Action::OpenSettings, &NoContent);
        assert!(!shell.is_changing());
        let frames = shell.frames();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].0, Screen::Settings);
        assert!(frames[0].1.is_shown());
    }

    #[test]
    fn an_event_that_triggers_nothing_is_idle() {
        // A host checks this before rebuilding anything, so an idle event must not report work.
        let mut shell = shell();
        let outcome = shell.handle(UiEvent::PointerMoved { x: 10.0, y: 10.0 }, &NoContent);
        assert!(outcome.is_idle());
        assert!(shell.handle(UiEvent::PointerLeft, &NoContent).is_idle());
    }
}
