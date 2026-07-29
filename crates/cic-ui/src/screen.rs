//! Which screens are open, in what order, and what each of them remembers.
//!
//! # Why a stack rather than one current screen
//!
//! Because the interesting case is a modal. Opening a confirmation over a menu and closing it again
//! has to leave the menu exactly as it was — the same scroll offset, the same half-typed name, the
//! same focused control — and a single current screen means rebuilding it from nothing on the way
//! back. Rebuilding is not merely wasteful; it is *visible*, because everything the user had done
//! that the host does not separately own is gone.
//!
//! So each open screen keeps its own [`Interface`], and the one underneath keeps it while something
//! sits on top.
//!
//! # Why a screen appears at most once
//!
//! Navigation here is by *destination* — "show settings" — not by "push another settings screen".
//! Asking for a screen already open therefore unwinds to it rather than stacking a second copy, which
//! is what makes main menu → settings → main menu return to the menu that was already there instead
//! of burying it under a duplicate nobody can reach.
//!
//! It also removes a bound that would otherwise have to be invented. Input can push screens, and
//! anything input can grow without a limit is a leak reachable from a keyboard; with no duplicates the
//! depth cannot exceed the number of screens the engine defines, so the limit is structural rather
//! than a number somebody chose.
//!
//! # Why the stack does not own the layouts
//!
//! A layout is loaded once and solved many times; a stack entry is pushed and popped. Holding a
//! [`Layout`] per entry would copy one on every push, and a screen pushed twice would hold two copies
//! of an identical tree. [`Screens`] holds one of each, the stack names them, and a loader that has
//! built a [`Screens`] with a gap in it finds out at load time rather than when a button does nothing.

use std::collections::BTreeMap;

use crate::layout::Layout;
use crate::state::Interface;
use crate::transition::{Change, Heading, Motion, Reveal};
use crate::{Action, StringTable};

/// One screen of the shell.
///
/// A closed set for the reason [`Action`] is: navigation is reachable from a layout file, and data must
/// not be able to name a destination the engine does not define. The variants correspond one-to-one
/// with the `Open*` actions, which is what [`Self::from_action`] relies on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Screen {
    /// The shell's root, which is always at the bottom of the stack.
    MainMenu,
    /// Settings, including the display settings that need confirming.
    Settings,
    /// Skirmish setup, from which a map is launched.
    SkirmishSetup,
    /// A modal asking whether to leave the game.
    QuitConfirm,
    /// A modal asking whether to keep an applied settings change, or take it back.
    ///
    /// Pushed by the shell when a change is applied rather than reached by an action, for the reason
    /// [`crate::settings`] exists at all: the question is asked *because* something was applied, and a
    /// layout that could open it without applying anything would be a dialog about nothing. It closes when
    /// either button answers it, and also when the revert window runs out — which is the case the whole
    /// mechanism is built for, since a user who cannot see the screen cannot close a dialog either.
    SettingsConfirm,
}

impl Screen {
    /// Every screen, in the order they are declared.
    ///
    /// Stated as a slice so a loader can require a layout for each without a match that a new variant
    /// would silently pass.
    pub const ALL: &'static [Self] = &[
        Self::MainMenu,
        Self::Settings,
        Self::SkirmishSetup,
        Self::QuitConfirm,
        Self::SettingsConfirm,
    ];

    /// The screen an action navigates to, or nothing when the action is not navigation.
    #[must_use]
    pub const fn from_action(action: Action) -> Option<Self> {
        match action {
            Action::OpenMainMenu => Some(Self::MainMenu),
            Action::OpenSettings => Some(Self::Settings),
            Action::OpenSkirmishSetup => Some(Self::SkirmishSetup),
            Action::OpenQuitConfirm => Some(Self::QuitConfirm),
            _ => None,
        }
    }

    /// This screen's name in `snake_case`, which is what a layout file is named after.
    ///
    /// A host builds `main_menu.ciclayout.json` from it, so the mapping from a screen to the file that
    /// describes it is one rule rather than a table somebody has to keep in step.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::MainMenu => "main_menu",
            Self::Settings => "settings",
            Self::SkirmishSetup => "skirmish_setup",
            Self::QuitConfirm => "quit_confirm",
            Self::SettingsConfirm => "settings_confirm",
        }
    }

    /// Whether this screen is drawn over whatever is beneath it rather than replacing it.
    ///
    /// A modal's layout is authored to cover only part of the surface, so the screen underneath has to
    /// keep being drawn or the modal appears over nothing. This is the screen's property rather than
    /// the layout's because it decides what the *stack* draws, and a layout that happens to fill the
    /// viewport is still a modal if it is one.
    #[must_use]
    pub const fn is_modal(self) -> bool {
        matches!(self, Self::QuitConfirm | Self::SettingsConfirm)
    }
}

/// One layout per screen, loaded once.
///
/// A `BTreeMap` rather than an array indexed by variant because the determinism rule reaches anything
/// whose iteration order is observable, and [`Self::missing`] reports in a fixed order for that reason.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Screens {
    layouts: BTreeMap<Screen, Layout>,
}

impl Screens {
    /// An empty catalogue.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces one screen's layout.
    pub fn insert(&mut self, screen: Screen, layout: Layout) -> Option<Layout> {
        self.layouts.insert(screen, layout)
    }

    /// One screen's layout.
    #[must_use]
    pub fn get(&self, screen: Screen) -> Option<&Layout> {
        self.layouts.get(&screen)
    }

    /// Which screens have no layout, in declaration order.
    ///
    /// What a loader refuses on. A missing layout is otherwise a button that navigates to nothing,
    /// which looks like an unfinished screen rather than like a missing file.
    #[must_use]
    pub fn missing(&self) -> Vec<Screen> {
        Screen::ALL
            .iter()
            .copied()
            .filter(|screen| !self.layouts.contains_key(screen))
            .collect()
    }

    /// Every `text_key` any screen names, in screen order then tree order, with duplicates kept.
    ///
    /// Feeds [`StringTable::missing`], which sorts and deduplicates, so a loader checks the whole shell
    /// against the string table once instead of discovering a blank label per screen.
    #[must_use]
    pub fn text_keys(&self) -> Vec<&str> {
        self.layouts.values().flat_map(Layout::text_keys).collect()
    }

    /// The keys this catalogue names that a table does not define, sorted and deduplicated.
    #[must_use]
    pub fn missing_strings(&self, strings: &StringTable) -> Vec<&str> {
        strings.missing(self.text_keys())
    }
}

/// What applying an action did to the stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    /// Nothing moved. Either the action was not navigation, or it is one the host acts on itself.
    Unchanged,
    /// A screen opened over the one that was on top, which is still there.
    Pushed(Screen),
    /// One or more screens closed, leaving this one on top.
    Popped(Screen),
    /// Going back was asked for at the root, which has nothing beneath it.
    ///
    /// Reported rather than ignored, because what to do about it is a shell decision — the sensible
    /// answer is to ask whether to leave the game, and that is not this type's call.
    AtRoot,
}

/// The screens that are open, innermost last, each with what it remembers.
///
/// Never empty: a shell with no screen has nothing to draw and no way to get one back, so the root is
/// established at construction and cannot be popped.
#[derive(Debug, Clone, PartialEq)]
pub struct ScreenStack {
    entries: Vec<Entry>,
    /// The screen a *backward* change is taking away, kept alive so it can still be drawn.
    ///
    /// Only ever set going backward. Going forward the screen being left is still in `entries`, one below
    /// the top, so keeping a second copy of it would be two sources of truth for one screen's state.
    leaving: Option<Entry>,
    change: Option<Change>,
    motion: Motion,
}

#[derive(Debug, Clone, PartialEq)]
struct Entry {
    screen: Screen,
    interface: Interface,
}

impl ScreenStack {
    /// Opens a stack at its root, with screens changing instantly.
    ///
    /// Instant by default because a stack is the navigation model and motion is a presentation choice — a
    /// caller that has not made one gets no animation rather than one somebody guessed at. Use
    /// [`Self::with_motion`] to animate.
    #[must_use]
    pub fn new(root: Screen) -> Self {
        Self::with_motion(root, Motion::INSTANT)
    }

    /// Opens a stack at its root, animating changes.
    #[must_use]
    pub fn with_motion(root: Screen, motion: Motion) -> Self {
        Self {
            entries: vec![Entry {
                screen: root,
                interface: Interface::new(),
            }],
            leaving: None,
            change: None,
            motion,
        }
    }

    /// How screens change.
    #[must_use]
    pub const fn motion(&self) -> Motion {
        self.motion
    }

    /// Replaces how screens change, which is what a reduce-motion setting drives.
    ///
    /// A change already in flight is left to finish. Cancelling it would jump the interface to its end
    /// state mid-movement, which is a worse thing to see than the last 100 milliseconds of an animation
    /// somebody just turned off.
    pub const fn set_motion(&mut self, motion: Motion) {
        self.motion = motion;
    }

    /// The change in flight, if one is.
    #[must_use]
    pub fn change(&self) -> Option<Change> {
        self.change.filter(|change| !change.is_done())
    }

    /// Whether a change is still animating, which is what tells a host to keep redrawing.
    #[must_use]
    pub fn is_changing(&self) -> bool {
        self.change().is_some()
    }

    /// Advances a change in flight, reporting whether it finished on this tick.
    ///
    /// Finishing releases what the departing screen remembered, which is why this has to be called rather
    /// than merely observed: a host that never ticks holds one screen's state forever.
    pub fn advance(&mut self, elapsed: f32) -> bool {
        let Some(change) = self.change.as_mut() else {
            return false;
        };
        if !change.advance(elapsed) {
            return false;
        }
        self.change = None;
        self.leaving = None;
        true
    }

    /// Every screen to draw right now with how much of it shows, outermost first.
    ///
    /// The same set as [`Self::drawn`] when nothing is changing. During a change it also carries the screen
    /// being left, and the reveal each one is at.
    ///
    /// Only the **topmost** screen animates in. Anything beneath it was already visible and is staying —
    /// a modal fades in over a backdrop that does not move, because a backdrop that faded with it would
    /// look like the whole interface was being replaced rather than covered.
    #[must_use]
    pub fn frames(&self) -> Vec<(Screen, Reveal)> {
        let drawn = self.drawn();
        let Some(change) = self.change() else {
            return drawn
                .into_iter()
                .map(|screen| (screen, Reveal::SHOWN))
                .collect();
        };
        let mut frames = Vec::with_capacity(drawn.len() + 1);
        // Skipped when the departing screen is *also* in the arriving set, which is what a modal opening
        // over a screen looks like: it is not being left, it is being covered.
        if let Some(screen) = self.outgoing().filter(|screen| !drawn.contains(screen)) {
            frames.push((screen, change.leaving(!screen.is_modal())));
        }
        let last = drawn.len().saturating_sub(1);
        for (index, screen) in drawn.into_iter().enumerate() {
            let reveal = if index == last {
                change.entering(!screen.is_modal())
            } else {
                Reveal::SHOWN
            };
            frames.push((screen, reveal));
        }
        frames
    }

    /// Every screen that has to be solved right now: the drawn set plus one being left.
    ///
    /// What a host caches solved layouts against, and a superset of [`Self::frames`] so a cache built from
    /// it is never missing an entry a frame asks for.
    #[must_use]
    pub fn involved(&self) -> Vec<Screen> {
        let mut screens = self.drawn();
        if let Some(screen) = self.outgoing().filter(|screen| !screens.contains(screen)) {
            screens.insert(0, screen);
        }
        screens
    }

    /// The screen a change in flight is taking away, whichever side of the stack it is on.
    fn outgoing(&self) -> Option<Screen> {
        let change = self.change()?;
        match change.direction() {
            // Still in the stack, one below the top, so there is only ever one copy of its state.
            Heading::Forward => self
                .entries
                .len()
                .checked_sub(2)
                .and_then(|at| self.entries.get(at))
                .map(|entry| entry.screen),
            Heading::Backward => self.leaving.as_ref().map(|entry| entry.screen),
        }
    }

    /// Records a change, unless motion is instant.
    ///
    /// An instant motion records nothing at all rather than a change that is immediately done: a stack
    /// nobody ticks would otherwise hold the departing screen's state forever, and "nothing to advance" is
    /// a simpler thing for a host to be correct about than "advance a change that has already finished".
    ///
    /// A second navigation while one is in flight replaces it. The abandoned screen is dropped rather than
    /// queued, because a queue of departing screens is depth in flight with no bound — the same leak the
    /// no-duplicates rule closed for the stack itself.
    fn begin(&mut self, direction: Heading, leaving: Option<Entry>) {
        self.leaving = leaving;
        if self.motion.seconds() <= 0.0 {
            self.change = None;
            self.leaving = None;
            return;
        }
        self.change = Some(Change::new(direction, self.motion));
    }

    /// The screen on top: the one input goes to and the one drawn last.
    #[must_use]
    pub fn top(&self) -> Screen {
        self.entry().screen
    }

    /// The root screen, which is what remains when everything else is closed.
    #[must_use]
    pub fn root(&self) -> Screen {
        self.entries
            .first()
            .map_or(Screen::MainMenu, |entry| entry.screen)
    }

    /// How many screens are open.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.entries.len()
    }

    /// Every open screen, outermost first.
    pub fn screens(&self) -> impl Iterator<Item = Screen> + '_ {
        self.entries.iter().map(|entry| entry.screen)
    }

    /// The screens to draw, outermost first, starting at the last one that covers what is beneath it.
    ///
    /// A modal needs the screen behind it drawn; a full screen does not, and drawing it would be work
    /// nobody sees. Everything below the last non-modal screen is therefore skipped.
    #[must_use]
    pub fn drawn(&self) -> Vec<Screen> {
        let base = self
            .entries
            .iter()
            .rposition(|entry| !entry.screen.is_modal())
            .unwrap_or(0);
        self.entries[base..]
            .iter()
            .map(|entry| entry.screen)
            .collect()
    }

    /// Whether a screen is open.
    #[must_use]
    pub fn contains(&self, screen: Screen) -> bool {
        self.entries.iter().any(|entry| entry.screen == screen)
    }

    /// What the top screen remembers.
    ///
    /// Only the top one, because only the top one takes input. That is what makes a modal modal: the
    /// screen underneath keeps its state and receives nothing.
    #[must_use]
    pub fn interface(&self) -> &Interface {
        &self.entry().interface
    }

    /// What the top screen remembers, for a host to seed values into or read them out of.
    pub fn interface_mut(&mut self) -> &mut Interface {
        &mut self.entry_mut().interface
    }

    /// What a particular open screen remembers, whether or not it is on top.
    ///
    /// For a host reading a value off a screen it has navigated away from but not closed — and for one
    /// *drawing* a screen that is on its way out, which is still there for as long as the change lasts.
    #[must_use]
    pub fn interface_for(&self, screen: Screen) -> Option<&Interface> {
        self.entries
            .iter()
            .chain(self.leaving.as_ref())
            .find(|entry| entry.screen == screen)
            .map(|entry| &entry.interface)
    }

    /// Opens a screen, or unwinds to it when it is already open.
    ///
    /// Returns [`Transition::Pushed`] in the first case and [`Transition::Popped`] in the second. See
    /// the module documentation for why a screen never appears twice.
    pub fn push(&mut self, screen: Screen) -> Transition {
        if let Some(at) = self.entries.iter().position(|entry| entry.screen == screen) {
            // Unwinding is going back, however it was asked for, so it animates backward. What leaves is
            // the screen that was on *top* — what the user was actually looking at — rather than the
            // outermost of the ones being closed. Unwinding several levels therefore shows one screen
            // leaving rather than a stack of them, which is a simplification and the honest one: the
            // intermediate screens were never visible.
            let departing = (at + 1 < self.entries.len())
                .then(|| self.entries.last().cloned())
                .flatten();
            self.entries.truncate(at + 1);
            self.begin(Heading::Backward, departing);
            return Transition::Popped(screen);
        }
        self.entries.push(Entry {
            screen,
            interface: Interface::new(),
        });
        // Nothing is handed over: the screen being left is still one below the top.
        self.begin(Heading::Forward, None);
        Transition::Pushed(screen)
    }

    /// Closes the top screen, unless it is the root.
    ///
    /// Returns the screen left on top, or nothing when there was only the root. What the closed screen
    /// remembered is discarded once the change finishes, which is the point: reopening settings should show
    /// what is in force rather than the edits somebody abandoned. It survives until then only so it can be
    /// drawn on its way out.
    pub fn pop(&mut self) -> Option<Screen> {
        if self.entries.len() <= 1 {
            return None;
        }
        let departing = self.entries.pop();
        self.begin(Heading::Backward, departing);
        Some(self.top())
    }

    /// Closes everything but the root.
    pub fn reset(&mut self) -> Screen {
        if self.entries.len() > 1 {
            let departing = self.entries.last().cloned();
            self.entries.truncate(1);
            self.begin(Heading::Backward, departing);
        }
        self.top()
    }

    /// Applies an action's navigation, leaving everything else alone.
    ///
    /// An action that does not leave the screen cannot move the stack, whatever else it does — see
    /// [`Action::leaves_screen`]. That is load-bearing for settings: applying a display change has to
    /// keep the settings screen on top, because the revert window is only useful while the confirm
    /// button is somewhere the user can reach.
    pub fn apply(&mut self, action: Action) -> Transition {
        if !action.leaves_screen() {
            return Transition::Unchanged;
        }
        if let Some(screen) = Screen::from_action(action) {
            return self.push(screen);
        }
        match action {
            Action::Back => self.pop().map_or(Transition::AtRoot, Transition::Popped),
            // Leaving the game and starting a match both leave the shell rather than move within it,
            // so the stack has nothing to say and the host acts on the action itself.
            _ => Transition::Unchanged,
        }
    }

    /// The top entry. The stack is never empty, so this is total in practice; the fallback exists so a
    /// bug here cannot become a panic in a host's frame loop.
    fn entry(&self) -> &Entry {
        self.entries
            .last()
            .expect("a screen stack always holds its root")
    }

    fn entry_mut(&mut self) -> &mut Entry {
        self.entries
            .last_mut()
            .expect("a screen stack always holds its root")
    }
}

#[cfg(test)]
mod tests {
    // The durations below are the motion's own and halves of it, all exact in binary, and the offsets
    // compared are the zeroes a screen that does not slide reports exactly.
    #![allow(clippy::float_cmp)]

    use super::{Screen, ScreenStack, Screens, Transition};
    use crate::layout::{Layout, Node, Widget};
    use crate::state::Value;
    use crate::transition::{Motion, Reveal};
    use crate::{Action, StringTable};

    fn layout(key: &str) -> Layout {
        Layout {
            format_version: crate::layout::FORMAT_VERSION,
            root: Node {
                widget: Widget::Label,
                text_key: Some(key.to_owned()),
                ..Node::default()
            },
        }
    }

    fn stack() -> ScreenStack {
        ScreenStack::new(Screen::MainMenu)
    }

    /// A stack whose screens animate, for the tests that are about motion.
    fn animated() -> ScreenStack {
        ScreenStack::with_motion(Screen::MainMenu, Motion::DEFAULT)
    }

    #[test]
    fn every_open_action_names_a_screen_and_nothing_else_does() {
        // The correspondence `Screen::from_action` relies on. A new `Open*` action without a screen
        // would navigate nowhere, which is a button that silently does nothing.
        assert_eq!(
            Screen::from_action(Action::OpenMainMenu),
            Some(Screen::MainMenu)
        );
        assert_eq!(
            Screen::from_action(Action::OpenSettings),
            Some(Screen::Settings)
        );
        assert_eq!(
            Screen::from_action(Action::OpenSkirmishSetup),
            Some(Screen::SkirmishSetup)
        );
        assert_eq!(
            Screen::from_action(Action::OpenQuitConfirm),
            Some(Screen::QuitConfirm)
        );
        for action in [
            Action::Quit,
            Action::Back,
            Action::ApplySettings,
            Action::ConfirmSettings,
            Action::RevertSettings,
            Action::LaunchSkirmish,
        ] {
            assert_eq!(Screen::from_action(action), None, "{action:?}");
        }
    }

    #[test]
    fn a_slug_is_unique_so_it_can_name_a_file() {
        let mut slugs: Vec<&str> = Screen::ALL.iter().map(|screen| screen.slug()).collect();
        let count = slugs.len();
        slugs.sort_unstable();
        slugs.dedup();
        assert_eq!(slugs.len(), count, "two screens share a slug");
    }

    #[test]
    fn a_catalogue_reports_the_screens_it_has_no_layout_for() {
        // A loader refuses on this. Otherwise a missing file is a button that navigates to nothing,
        // which reads as an unfinished screen rather than as a packaging mistake.
        let mut screens = Screens::new();
        assert_eq!(screens.missing(), Screen::ALL.to_vec());
        for screen in Screen::ALL {
            screens.insert(*screen, layout(screen.slug()));
        }
        assert!(screens.missing().is_empty());
        assert!(screens.get(Screen::Settings).is_some());
    }

    #[test]
    fn a_catalogue_checks_every_screens_text_against_one_table() {
        // One error naming every absent key beats one blank label found per screen.
        let mut screens = Screens::new();
        screens.insert(Screen::MainMenu, layout("menu.title"));
        screens.insert(Screen::Settings, layout("settings.title"));
        let mut strings = StringTable::new();
        strings.set("menu.title", "Commanders in Chief");
        assert_eq!(screens.missing_strings(&strings), vec!["settings.title"]);
        strings.set("settings.title", "Settings");
        assert!(screens.missing_strings(&strings).is_empty());
    }

    #[test]
    fn a_stack_starts_at_its_root_and_cannot_be_emptied() {
        // A shell with no screen has nothing to draw and no way to get one back.
        let mut stack = stack();
        assert_eq!(stack.top(), Screen::MainMenu);
        assert_eq!(stack.root(), Screen::MainMenu);
        assert_eq!(stack.depth(), 1);
        assert_eq!(stack.pop(), None);
        assert_eq!(stack.depth(), 1);
    }

    #[test]
    fn the_screen_underneath_keeps_what_it_remembered() {
        // The charter's own wording, and the reason this is a stack: a modal must not cost the menu
        // beneath it everything the user had done there.
        let mut stack = stack();
        stack.interface_mut().set("host", Value::Select(3));
        assert_eq!(
            stack.push(Screen::QuitConfirm),
            Transition::Pushed(Screen::QuitConfirm)
        );
        // The modal starts blank, and the menu's state is untouched but not reachable from the top.
        assert_eq!(stack.interface().get("host"), None);
        assert_eq!(
            stack
                .interface_for(Screen::MainMenu)
                .and_then(|interface| interface.selection("host")),
            Some(3)
        );
        assert_eq!(stack.pop(), Some(Screen::MainMenu));
        assert_eq!(stack.interface().selection("host"), Some(3));
    }

    #[test]
    fn a_closed_screen_forgets_what_it_held() {
        // Reopening settings has to show what is in force, not the edits somebody walked away from.
        let mut stack = stack();
        stack.push(Screen::Settings);
        stack.interface_mut().set_text("name", "half typed");
        stack.pop();
        stack.push(Screen::Settings);
        assert!(stack.interface().text("name").is_none());
    }

    #[test]
    fn asking_for_an_open_screen_unwinds_to_it_rather_than_duplicating_it() {
        // Main menu, settings, main menu returns to the menu that was already there. Stacking a second
        // copy would bury one nobody can reach, and would make depth grow with every press.
        let mut stack = stack();
        stack.interface_mut().set("host", Value::Select(7));
        stack.push(Screen::Settings);
        stack.push(Screen::QuitConfirm);
        assert_eq!(stack.depth(), 3);
        assert_eq!(
            stack.push(Screen::MainMenu),
            Transition::Popped(Screen::MainMenu)
        );
        assert_eq!(stack.depth(), 1);
        assert_eq!(stack.interface().selection("host"), Some(7));
    }

    #[test]
    fn depth_is_bounded_by_the_screens_that_exist_rather_than_by_a_chosen_number() {
        // Input can push screens, and anything input can grow without a limit is a leak reachable from
        // a keyboard. No duplicates makes the bound structural.
        let mut stack = stack();
        for _ in 0..64 {
            for screen in Screen::ALL {
                stack.push(*screen);
            }
        }
        assert!(
            stack.depth() <= Screen::ALL.len(),
            "depth {} exceeded the {} screens that exist",
            stack.depth(),
            Screen::ALL.len()
        );
    }

    #[test]
    fn only_a_modal_leaves_the_screen_beneath_it_drawn() {
        let mut stack = stack();
        assert_eq!(stack.drawn(), vec![Screen::MainMenu]);
        stack.push(Screen::Settings);
        // Settings covers the surface, so the menu underneath is work nobody would see.
        assert_eq!(stack.drawn(), vec![Screen::Settings]);
        stack.push(Screen::QuitConfirm);
        assert_eq!(
            stack.drawn(),
            vec![Screen::Settings, Screen::QuitConfirm],
            "a modal needs what is behind it drawn or it appears over nothing"
        );
    }

    #[test]
    fn navigation_actions_move_the_stack_and_back_unwinds_it() {
        let mut stack = stack();
        assert_eq!(
            stack.apply(Action::OpenSettings),
            Transition::Pushed(Screen::Settings)
        );
        assert_eq!(
            stack.apply(Action::Back),
            Transition::Popped(Screen::MainMenu)
        );
        // At the root there is nothing beneath, which is reported rather than ignored so a shell can
        // offer to leave the game instead.
        assert_eq!(stack.apply(Action::Back), Transition::AtRoot);
        assert_eq!(stack.depth(), 1);
    }

    #[test]
    fn a_settings_action_cannot_move_the_stack() {
        // Load-bearing: the revert window is only useful while the confirm button is somewhere the user
        // can reach, so applying a display change has to leave the settings screen on top.
        let mut stack = stack();
        stack.push(Screen::Settings);
        for action in [
            Action::ApplySettings,
            Action::ConfirmSettings,
            Action::RevertSettings,
        ] {
            assert_eq!(stack.apply(action), Transition::Unchanged, "{action:?}");
            assert_eq!(stack.top(), Screen::Settings);
            assert_eq!(stack.depth(), 2);
        }
    }

    #[test]
    fn leaving_the_shell_is_the_hosts_business_and_not_the_stacks() {
        let mut stack = stack();
        stack.push(Screen::SkirmishSetup);
        assert_eq!(stack.apply(Action::LaunchSkirmish), Transition::Unchanged);
        assert_eq!(stack.apply(Action::Quit), Transition::Unchanged);
        assert_eq!(stack.top(), Screen::SkirmishSetup);
    }

    #[test]
    fn an_instant_stack_records_no_change_at_all() {
        // Not "a change that is already finished": a stack nobody ticks would then hold the departing
        // screen's state forever, and "nothing to advance" is simpler for a host to be correct about.
        let mut stack = stack();
        stack.push(Screen::Settings);
        assert!(!stack.is_changing());
        assert_eq!(stack.change(), None);
        assert_eq!(stack.frames(), vec![(Screen::Settings, Reveal::SHOWN)]);
        assert_eq!(stack.involved(), vec![Screen::Settings]);
        assert!(!stack.advance(1.0), "there is nothing to advance");
    }

    #[test]
    fn a_change_keeps_the_departing_screen_drawable_until_it_finishes() {
        // The load-bearing part. `pop` drops the entry, so without this the screen being left is gone
        // before it can be drawn on its way out, and a host that kept its own copy would be duplicating
        // state the stack had just discarded.
        let mut stack = animated();
        stack.push(Screen::Settings);
        stack.interface_mut().set_text("name", "half typed");
        assert_eq!(stack.pop(), Some(Screen::MainMenu));
        assert!(stack.is_changing());
        // Still drawable, and still holding what it held.
        assert_eq!(
            stack
                .interface_for(Screen::Settings)
                .and_then(|interface| interface.text("name"))
                .map(|field| field.text().to_owned()),
            Some("half typed".to_owned())
        );
        assert!(stack.involved().contains(&Screen::Settings));
        let frames = stack.frames();
        assert_eq!(frames.len(), 2, "{frames:?}");
        assert_eq!(frames[0].0, Screen::Settings, "the departing one first");
        assert_eq!(frames[1].0, Screen::MainMenu);
        // Finishing releases it, which is why advancing has to be called rather than merely observed.
        assert!(stack.advance(Motion::DEFAULT.duration));
        assert!(!stack.is_changing());
        assert!(stack.interface_for(Screen::Settings).is_none());
        assert_eq!(stack.frames(), vec![(Screen::MainMenu, Reveal::SHOWN)]);
    }

    #[test]
    fn input_reaches_the_arriving_screen_at_once_and_the_departing_one_never() {
        // Two rules that would go wrong if the animation lived outside the stack. A click during a change
        // must not land on a screen that is fading out, and the arriving screen must not spend the
        // transition's duration ignoring input — that would add the animation to the latency of every
        // navigation. Both fall out of routing to the top entry, because the departing screen is not one.
        let mut stack = animated();
        stack.push(Screen::Settings);
        assert_eq!(
            stack.top(),
            Screen::Settings,
            "input goes to the new screen"
        );
        stack.interface_mut().set_toggle("arrived", true);
        stack.pop();
        assert_eq!(stack.top(), Screen::MainMenu);
        stack.interface_mut().set_toggle("returned", true);
        // Neither reached the other.
        assert_eq!(stack.interface().toggle("arrived"), None);
        assert_eq!(
            stack
                .interface_for(Screen::Settings)
                .and_then(|interface| interface.toggle("returned")),
            None
        );
    }

    #[test]
    fn a_modal_fades_in_over_a_backdrop_that_does_not_move() {
        // Only the topmost screen animates. A backdrop that faded with the modal would look like the whole
        // interface was being replaced rather than covered, and the screen underneath is not going anywhere.
        let mut stack = animated();
        stack.push(Screen::Settings);
        stack.advance(Motion::DEFAULT.duration);
        stack.push(Screen::QuitConfirm);
        let frames = stack.frames();
        assert_eq!(frames.len(), 2, "{frames:?}");
        assert_eq!(frames[0], (Screen::Settings, Reveal::SHOWN));
        assert_eq!(frames[1].0, Screen::QuitConfirm);
        assert!(frames[1].1.is_hidden(), "the modal starts invisible");
        // And it fades rather than sliding, because it displaces nothing.
        stack.advance(Motion::DEFAULT.duration / 2.0);
        let frames = stack.frames();
        assert_eq!(frames[1].1.offset, [0.0, 0.0]);
        assert!(frames[1].1.opacity > 0.0 && frames[1].1.opacity < 1.0);
    }

    #[test]
    fn opening_a_screen_slides_forward_and_going_back_slides_the_other_way() {
        let mut stack = animated();
        stack.push(Screen::Settings);
        let forward = stack.frames();
        let arriving = forward
            .iter()
            .find(|(screen, _)| *screen == Screen::Settings)
            .expect("the new screen");
        assert!(arriving.1.offset[0] > 0.0, "it arrives from the right");
        stack.advance(Motion::DEFAULT.duration);
        stack.pop();
        let back = stack.frames();
        let returning = back
            .iter()
            .find(|(screen, _)| *screen == Screen::MainMenu)
            .expect("the screen returned to");
        assert!(returning.1.offset[0] < 0.0, "it comes back from the left");
    }

    #[test]
    fn a_second_navigation_replaces_a_change_rather_than_queueing_it() {
        // A queue of departing screens is depth in flight with no bound, which is the same leak the
        // no-duplicates rule closed for the stack itself.
        let mut stack = animated();
        stack.push(Screen::Settings);
        stack.push(Screen::QuitConfirm);
        assert!(stack.is_changing());
        // Only ever one screen is on its way out, so a frame list stays small however fast somebody clicks.
        assert!(stack.frames().len() <= Screen::ALL.len());
        stack.pop();
        assert!(stack.frames().len() <= Screen::ALL.len());
        stack.advance(Motion::DEFAULT.duration);
        assert!(!stack.is_changing());
        assert_eq!(stack.frames().len(), 1);
    }

    #[test]
    fn unwinding_several_levels_shows_the_screen_that_was_on_top_leaving() {
        // The intermediate screens were never visible, so animating them out would be motion for something
        // nobody saw arrive.
        let mut stack = animated();
        stack.push(Screen::Settings);
        stack.push(Screen::QuitConfirm);
        stack.advance(Motion::DEFAULT.duration);
        stack.push(Screen::MainMenu);
        let frames = stack.frames();
        assert_eq!(frames.len(), 2, "{frames:?}");
        assert_eq!(
            frames[0].0,
            Screen::QuitConfirm,
            "what the user was looking at"
        );
        assert_eq!(frames[1].0, Screen::MainMenu);
    }

    #[test]
    fn turning_motion_off_leaves_a_change_in_flight_to_finish() {
        // Cancelling it would jump the interface to its end state mid-movement, which is worse to look at
        // than the last hundred milliseconds of an animation somebody has just turned off.
        let mut stack = animated();
        stack.push(Screen::Settings);
        stack.set_motion(Motion::INSTANT);
        assert!(stack.is_changing(), "the one in flight still finishes");
        stack.advance(Motion::DEFAULT.duration);
        // And the next change does not animate.
        stack.push(Screen::SkirmishSetup);
        assert!(!stack.is_changing());
    }

    #[test]
    fn resetting_leaves_the_root_and_its_state() {
        let mut stack = stack();
        stack.interface_mut().set_toggle("seen", true);
        stack.push(Screen::Settings);
        stack.push(Screen::QuitConfirm);
        assert_eq!(stack.reset(), Screen::MainMenu);
        assert_eq!(stack.depth(), 1);
        assert_eq!(stack.interface().toggle("seen"), Some(true));
        assert!(!stack.contains(Screen::Settings));
    }
}
