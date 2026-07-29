//! Applying a settings change so that a *machine* can undo it, not only a user.
//!
//! # Why an apply is not a commit
//!
//! A display change can leave the person who made it unable to see the screen well enough to undo it.
//! A resolution the monitor cannot sync to, a scale that puts the buttons off the panel, an exclusive
//! full-screen mode that comes up black — in every one of those the interface is still there, still
//! listening, and still completely unreachable. So an undo that depends on the user clicking something
//! is not an undo at all.
//!
//! The fix is to make the *absence* of a confirmation the reverting condition. A change is applied,
//! a window opens, and if nothing confirms it before the window closes the previous settings come
//! back on their own. Confirming is one interaction; failing to confirm needs none.
//!
//! # The three values a setting has at once
//!
//! - **In force** — what the host has actually given the device. What to render with.
//! - **Staged** — what the settings screen is editing. Not live, and not yet promised.
//! - **A restore point** — what to go back to, held for as long as an applied change is unconfirmed.
//!
//! Two of those are the obvious ones. The restore point is the one that makes this work, and the
//! subtle rule about it is that a *second* apply inside the window must not overwrite it: the value
//! worth returning to is the last one somebody confirmed, not the last one somebody tried. Applying
//! two bad display modes in a row would otherwise leave the restore point holding the first bad one.
//!
//! # Why time arrives as an argument
//!
//! Nothing in this crate reads a clock, for the reason nothing in the renderer does: a countdown that
//! reads the clock itself cannot be tested without waiting, and a test that waits is a test that is
//! flaky on a loaded machine. [`Transaction::tick`] takes the elapsed time, so the whole revert window
//! is exercised in microseconds by passing the numbers a real loop would have measured.
//!
//! **Which clock a host should pass matters.** This is the one countdown in the engine that must
//! *not* come from scene time: a display mode that produces no frames also advances no frame counter,
//! and a revert that depends on rendering succeeding is a revert that cannot fire in the case it
//! exists for. A host passes real elapsed seconds, measured wherever it can still measure them.

/// How long an applied change stays on probation before it takes itself back, in seconds.
///
/// Fifteen, which is also what Windows gives a display-mode change — not a coincidence to hide: a
/// person who has just lost their picture recognises the situation in a few seconds and needs a few
/// more to decide, and past about twenty they have reached for the power button instead. Long enough
/// to read a prompt and find a button; short enough that waiting it out beats rebooting.
pub const REVERT_WINDOW: f32 = 15.0;

/// What a settings transaction is currently doing.
///
/// Returned by [`Transaction::tick`], because the host has to know when a revert has happened: the
/// values coming back into force are values it must hand to the device again.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Probation {
    /// Nothing is waiting to be confirmed.
    Settled,
    /// An applied change is unconfirmed, with this many seconds left.
    ///
    /// What a settings screen counts down, so the user can see that doing nothing is a decision.
    Pending(f32),
    /// The window ran out and the previous settings are back in force.
    ///
    /// Reported once, on the tick that crossed the boundary. The host applies
    /// [`Transaction::in_force`] to the device again.
    Lapsed,
}

/// A settings value with an apply that has to be confirmed and a revert that does not.
///
/// Generic over the settings themselves, because this crate must not know what a display setting is —
/// the same boundary that keeps a GPU out of a layout solver. A host parameterises it with its own
/// type and this module never inspects one, only clones and compares.
#[derive(Debug, Clone, PartialEq)]
pub struct Transaction<T> {
    in_force: T,
    staged: T,
    restore: Option<T>,
    remaining: f32,
}

impl<T: Clone + PartialEq> Transaction<T> {
    /// Starts from settings already in force, with nothing staged and nothing to confirm.
    pub fn new(in_force: T) -> Self {
        Self {
            staged: in_force.clone(),
            in_force,
            restore: None,
            remaining: 0.0,
        }
    }

    /// The settings the host has given the device.
    pub const fn in_force(&self) -> &T {
        &self.in_force
    }

    /// The settings the screen is editing.
    pub const fn staged(&self) -> &T {
        &self.staged
    }

    /// The settings the screen is editing, for a widget to change.
    pub const fn staged_mut(&mut self) -> &mut T {
        &mut self.staged
    }

    /// Replaces what is staged.
    pub fn stage(&mut self, settings: T) {
        self.staged = settings;
    }

    /// Whether the staged settings differ from what is in force, which is what makes an apply do
    /// anything.
    ///
    /// A settings screen enables its apply button from this, so a button that would do nothing is
    /// visibly inert rather than silently so.
    pub fn is_dirty(&self) -> bool {
        self.staged != self.in_force
    }

    /// Whether an applied change is waiting to be confirmed.
    pub const fn is_unconfirmed(&self) -> bool {
        self.restore.is_some()
    }

    /// Seconds left before an unconfirmed change reverts itself, or nothing when none is waiting.
    pub const fn remaining(&self) -> Option<f32> {
        if self.restore.is_some() {
            Some(self.remaining)
        } else {
            None
        }
    }

    /// Puts the staged settings in force and opens the revert window.
    ///
    /// Returns whether anything was applied: staging what is already in force is a no-op, so a user
    /// pressing apply twice on an unchanged screen does not arm a countdown over nothing.
    ///
    /// An apply while a change is already unconfirmed **restarts the clock but keeps the original
    /// restore point**, because what is worth returning to is the last confirmed state and not the
    /// previous attempt at replacing it.
    pub fn apply(&mut self) -> bool {
        if !self.is_dirty() {
            return false;
        }
        let superseded = std::mem::replace(&mut self.in_force, self.staged.clone());
        self.restore.get_or_insert(superseded);
        self.remaining = REVERT_WINDOW;
        true
    }

    /// Keeps what is in force, closing the revert window.
    ///
    /// Returns whether there was anything to confirm. Confirming what is in force rather than what is
    /// staged is deliberate: a user may have gone on editing while the countdown ran, and confirming
    /// their unapplied edits would put settings into force that nobody had seen the effect of — which
    /// is the exact failure this whole mechanism exists to prevent.
    pub fn confirm(&mut self) -> bool {
        let confirmed = self.restore.take().is_some();
        self.remaining = 0.0;
        confirmed
    }

    /// Discards staged edits and, if a change is unconfirmed, brings back what it replaced.
    ///
    /// Returns whether the settings in force changed, which is what tells a host to hand them to the
    /// device again. Both halves are one action because both are what a user means by "no": a change
    /// they have not applied should stop being pending, and one they have should stop being live.
    pub fn revert(&mut self) -> bool {
        self.remaining = 0.0;
        let restored = self.restore.take();
        let changed = restored.is_some();
        if let Some(previous) = restored {
            self.in_force = previous;
        }
        self.staged = self.in_force.clone();
        changed
    }

    /// Advances the revert window by an elapsed time, reverting when it runs out.
    ///
    /// `elapsed` is in seconds and comes from the host's own clock — see the module documentation for
    /// why it must not come from scene time.
    ///
    /// A clock that goes backwards counts as no time passing rather than as time being given back,
    /// since a negative delta is a glitch and not an extension. A clock that reports a non-finite
    /// delta **spends the whole window**: the two ways to be wrong here are not symmetrical. Ignoring
    /// bad readings risks a countdown that never fires, which leaves somebody looking at a screen
    /// they cannot use; acting on one costs a setting the user has to choose again.
    pub fn tick(&mut self, elapsed: f32) -> Probation {
        if self.restore.is_none() {
            return Probation::Settled;
        }
        if !elapsed.is_finite() {
            self.revert();
            return Probation::Lapsed;
        }
        self.remaining -= elapsed.max(0.0);
        if self.remaining > 0.0 {
            return Probation::Pending(self.remaining);
        }
        self.revert();
        Probation::Lapsed
    }
}

#[cfg(test)]
mod tests {
    // The countdown arithmetic below is over values chosen to be exact in binary -- the window itself
    // and halves and quarters of a second -- so an exact comparison is the assertion being made rather
    // than a tolerance nobody chose.
    #![allow(clippy::float_cmp)]

    use super::{Probation, REVERT_WINDOW, Transaction};

    /// Stands in for a host's display settings: something clonable and comparable that this module
    /// deliberately knows nothing about.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct Display {
        scale: u32,
    }

    fn transaction() -> Transaction<Display> {
        Transaction::new(Display { scale: 100 })
    }

    #[test]
    fn a_fresh_transaction_stages_what_is_in_force_and_has_nothing_to_confirm() {
        let settings = transaction();
        assert_eq!(settings.in_force(), settings.staged());
        assert!(!settings.is_dirty());
        assert!(!settings.is_unconfirmed());
        assert_eq!(settings.remaining(), None);
    }

    #[test]
    fn applying_puts_the_staged_value_in_force_and_opens_the_window() {
        let mut settings = transaction();
        settings.stage(Display { scale: 200 });
        assert!(settings.is_dirty());
        assert!(settings.apply());
        assert_eq!(settings.in_force().scale, 200);
        assert!(settings.is_unconfirmed());
        assert_eq!(settings.remaining(), Some(REVERT_WINDOW));
        // Applied and staged now agree, so the screen has nothing further to offer.
        assert!(!settings.is_dirty());
    }

    #[test]
    fn applying_what_is_already_in_force_arms_no_countdown() {
        // Otherwise pressing apply on an untouched screen starts a revert timer over nothing, and the
        // user is asked to confirm a change they did not make.
        let mut settings = transaction();
        assert!(!settings.apply());
        assert!(!settings.is_unconfirmed());
        assert_eq!(settings.remaining(), None);
    }

    #[test]
    fn an_unconfirmed_change_reverts_itself_when_the_window_runs_out() {
        // The whole reason this type exists. Nobody clicks anything, and the settings come back.
        let mut settings = transaction();
        settings.stage(Display { scale: 200 });
        settings.apply();
        assert_eq!(settings.tick(5.0), Probation::Pending(REVERT_WINDOW - 5.0));
        assert_eq!(settings.tick(5.0), Probation::Pending(REVERT_WINDOW - 10.0));
        assert_eq!(settings.tick(5.0), Probation::Lapsed);
        assert_eq!(settings.in_force().scale, 100);
        // The screen has to show the truth afterwards, not the value that was taken away.
        assert_eq!(settings.staged().scale, 100);
        assert!(!settings.is_unconfirmed());
        // Reported once. A second tick has nothing left to revert.
        assert_eq!(settings.tick(5.0), Probation::Settled);
    }

    #[test]
    fn confirming_closes_the_window_and_keeps_the_change() {
        let mut settings = transaction();
        settings.stage(Display { scale: 200 });
        settings.apply();
        assert!(settings.confirm());
        assert_eq!(settings.in_force().scale, 200);
        assert!(!settings.is_unconfirmed());
        // Ticking past the whole window cannot take a confirmed change back.
        assert_eq!(settings.tick(REVERT_WINDOW * 10.0), Probation::Settled);
        assert_eq!(settings.in_force().scale, 200);
        // Nothing to confirm a second time.
        assert!(!settings.confirm());
    }

    #[test]
    fn confirming_keeps_what_is_in_force_rather_than_what_is_staged() {
        // A user can go on editing while the countdown runs. Confirming their unapplied edits would
        // put settings into force that nobody has seen the effect of, which is the failure this whole
        // mechanism exists to prevent.
        let mut settings = transaction();
        settings.stage(Display { scale: 200 });
        settings.apply();
        settings.stage(Display { scale: 400 });
        settings.confirm();
        assert_eq!(settings.in_force().scale, 200);
        // And the edit survives as an edit, so the screen still offers to apply it.
        assert_eq!(settings.staged().scale, 400);
        assert!(settings.is_dirty());
    }

    #[test]
    fn a_second_apply_inside_the_window_keeps_the_original_restore_point() {
        // The subtle one. Two bad display modes in a row must not leave the restore point holding the
        // first bad one: what is worth returning to is the last state somebody confirmed.
        let mut settings = transaction();
        settings.stage(Display { scale: 200 });
        settings.apply();
        settings.tick(5.0);
        settings.stage(Display { scale: 400 });
        assert!(settings.apply());
        // The clock restarts, because the second change deserves its own window to be judged in.
        assert_eq!(settings.remaining(), Some(REVERT_WINDOW));
        assert_eq!(settings.tick(REVERT_WINDOW), Probation::Lapsed);
        assert_eq!(settings.in_force().scale, 100);
    }

    #[test]
    fn reverting_takes_back_an_applied_change_and_discards_staged_edits_together() {
        // Both halves are what a user means by "no": what they have not applied should stop being
        // pending, and what they have should stop being live.
        let mut settings = transaction();
        settings.stage(Display { scale: 200 });
        settings.apply();
        settings.stage(Display { scale: 400 });
        assert!(settings.revert());
        assert_eq!(settings.in_force().scale, 100);
        assert_eq!(settings.staged().scale, 100);
        assert!(!settings.is_dirty());
        assert!(!settings.is_unconfirmed());
    }

    #[test]
    fn reverting_with_nothing_applied_still_discards_staged_edits() {
        let mut settings = transaction();
        settings.stage(Display { scale: 200 });
        // Nothing came into force, so the host has nothing to hand the device again.
        assert!(!settings.revert());
        assert_eq!(settings.staged().scale, 100);
        assert!(!settings.is_dirty());
    }

    #[test]
    fn a_clock_that_goes_backwards_does_not_extend_the_window() {
        // A negative delta is a glitch, not time being given back. Treating it as an extension would
        // make a misbehaving clock into a countdown that never finishes.
        let mut settings = transaction();
        settings.stage(Display { scale: 200 });
        settings.apply();
        settings.tick(10.0);
        assert_eq!(
            settings.tick(-100.0),
            Probation::Pending(REVERT_WINDOW - 10.0)
        );
        assert_eq!(settings.tick(5.0), Probation::Lapsed);
    }

    #[test]
    fn a_clock_reporting_nonsense_spends_the_whole_window_rather_than_none_of_it() {
        // The two ways to be wrong are not symmetrical. Ignoring a bad reading risks a countdown that
        // never fires, which leaves somebody looking at a screen they cannot use; acting on one costs
        // a setting they have to choose again.
        for broken in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut settings = transaction();
            settings.stage(Display { scale: 200 });
            settings.apply();
            assert_eq!(settings.tick(broken), Probation::Lapsed, "{broken}");
            assert_eq!(settings.in_force().scale, 100);
        }
    }

    #[test]
    fn ticking_with_nothing_unconfirmed_is_inert() {
        // A host ticks every frame whether or not a settings screen is open, so the idle path has to
        // be the cheap and harmless one.
        let mut settings = transaction();
        assert_eq!(settings.tick(1.0), Probation::Settled);
        settings.stage(Display { scale: 200 });
        assert_eq!(settings.tick(1.0), Probation::Settled);
        // And staging is untouched by a tick that had nothing to do.
        assert_eq!(settings.staged().scale, 200);
    }

    #[test]
    fn staging_through_a_borrow_is_the_same_as_replacing() {
        // What a slider does: reach in and change one field rather than build a whole settings value.
        let mut settings = transaction();
        settings.staged_mut().scale = 150;
        assert!(settings.is_dirty());
        assert!(settings.apply());
        assert_eq!(settings.in_force().scale, 150);
    }
}
