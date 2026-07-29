//! What a screen change looks like while it is happening.
//!
//! # Why this is separate from the screen stack
//!
//! The same split as theme against layout. *Which* screens are moving is a navigation fact and belongs to
//! [`crate::screen`]; *how* the movement looks is a presentation decision, and putting it there would mean
//! a change to the easing curve touching the module that decides where a button goes.
//!
//! # Why the curve eases out rather than in and out
//!
//! An ease-in-out barely moves for the first few frames, and those frames are the ones a user is deciding
//! whether the interface responded at all — so a symmetric curve reads as latency even though it finishes
//! at the same moment. An ease-out leaves at full speed and decelerates into place, so the interface has
//! visibly reacted on the frame the click landed. That is the whole argument, and it is why this is a
//! quadratic ease-*out* rather than the smoothstep that would otherwise be the obvious default.
//!
//! # Why a duration of zero is an ordinary case
//!
//! It is two things at once: the sensible default for anything that has not thought about motion, and what
//! a reduce-motion accessibility setting maps to. Neither should reach a special code path, because a
//! special path is one nobody exercises. [`Motion::INSTANT`] runs through the same arithmetic and lands on
//! a progress of one on the first tick.
//!
//! # Why time arrives as an argument
//!
//! For the reason it does in [`crate::settings`]: nothing here reads a clock, so a whole change is
//! exercised in microseconds rather than by waiting for one. A host advances it from the same call that
//! advances the settings revert window — see [`crate::Shell::tick`], which drives both, because two clocks
//! in one frame loop is how one of them stops being called.

/// How long a screen change takes by default, in seconds.
///
/// Short enough that it never stands between a user and what they asked for — a menu is navigated in bursts
/// and anything approaching a third of a second starts to feel like the interface is thinking. Long enough
/// that the eye registers *which* direction the change went, which is the only thing the motion is for.
pub const DEFAULT_DURATION: f32 = 0.18;

/// How far a screen slides, as a fraction of the viewport's width.
///
/// A fraction rather than a distance, because this crate's rule is that nothing outside the solver knows
/// how many pixels anything is. A twelfth is enough to read as movement and small enough that the outgoing
/// screen never leaves a visible gap at the edge before it has faded out.
pub const DEFAULT_TRAVEL: f32 = 1.0 / 12.0;

/// How a screen change is animated.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Motion {
    /// Seconds the change takes. Zero changes screens instantly.
    pub duration: f32,
    /// How far a screen slides, as a fraction of the viewport's width.
    pub travel: f32,
}

impl Motion {
    /// No animation: a change completes on the tick that starts it.
    ///
    /// What a reduce-motion setting selects, and what a test that is not about motion uses.
    pub const INSTANT: Self = Self {
        duration: 0.0,
        travel: 0.0,
    };

    /// A slide and a fade over [`DEFAULT_DURATION`].
    pub const DEFAULT: Self = Self {
        duration: DEFAULT_DURATION,
        travel: DEFAULT_TRAVEL,
    };

    /// A fade with no slide, over the default duration.
    ///
    /// For a host that wants motion without lateral movement, which is the other thing a reduce-motion
    /// preference sometimes means.
    pub const FADE: Self = Self {
        duration: DEFAULT_DURATION,
        travel: 0.0,
    };

    /// The duration, with a non-finite or negative value treated as instant.
    ///
    /// Sanitised rather than refused, for the reason a display scale is: a motion setting is not worth
    /// failing over, and the honest recovery is to change screens without animating.
    #[must_use]
    pub fn seconds(&self) -> f32 {
        if self.duration.is_finite() && self.duration > 0.0 {
            self.duration
        } else {
            0.0
        }
    }

    /// The travel fraction, sanitised the same way and bounded to a whole viewport.
    #[must_use]
    pub fn distance(&self) -> f32 {
        if self.travel.is_finite() {
            self.travel.clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

impl Default for Motion {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Which way a change is heading, which decides which way things slide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heading {
    /// Into a screen: the new one arrives from the trailing edge.
    Forward,
    /// Back out of one: the screen returned to arrives from the leading edge.
    Backward,
}

impl Heading {
    /// Which way the incoming screen travels: positive is from the trailing edge.
    const fn sign(self) -> f32 {
        match self {
            Self::Forward => 1.0,
            Self::Backward => -1.0,
        }
    }
}

/// How much of a screen is showing, and where it has been moved to.
///
/// Applied by [`crate::paint`], which is what keeps this crate's presentation decisions out of the
/// renderer: a transition is an opacity and an offset over a primitive list, so nothing about drawing has
/// to know that screens animate at all.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reveal {
    /// How opaque, from zero to one. Multiplies every colour's alpha.
    pub opacity: f32,
    /// Where it has moved to, as a fraction of the viewport. Positive X is rightward.
    pub offset: [f32; 2],
}

impl Reveal {
    /// Fully visible and in place, which is every screen when nothing is changing.
    pub const SHOWN: Self = Self {
        opacity: 1.0,
        offset: [0.0, 0.0],
    };

    /// Whether this is a screen at rest, so a caller can skip the arithmetic entirely.
    #[must_use]
    pub fn is_shown(&self) -> bool {
        self.opacity >= 1.0 && self.offset == [0.0, 0.0]
    }

    /// Whether nothing of this screen would be drawn.
    #[must_use]
    pub fn is_hidden(&self) -> bool {
        self.opacity <= 0.0
    }
}

/// A screen change in flight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Change {
    direction: Heading,
    motion: Motion,
    elapsed: f32,
}

impl Change {
    /// Starts a change.
    #[must_use]
    pub const fn new(direction: Heading, motion: Motion) -> Self {
        Self {
            direction,
            motion,
            elapsed: 0.0,
        }
    }

    /// Which way it is going.
    #[must_use]
    pub const fn direction(&self) -> Heading {
        self.direction
    }

    /// How far along it is, from zero to one, before easing.
    #[must_use]
    pub fn fraction(&self) -> f32 {
        let duration = self.motion.seconds();
        if duration <= 0.0 {
            return 1.0;
        }
        (self.elapsed / duration).clamp(0.0, 1.0)
    }

    /// How far along it is after easing, from zero to one.
    ///
    /// A quadratic ease-out — see the module documentation for why the asymmetry is deliberate.
    #[must_use]
    pub fn progress(&self) -> f32 {
        let remaining = 1.0 - self.fraction();
        1.0 - remaining * remaining
    }

    /// Whether it has finished.
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.fraction() >= 1.0
    }

    /// Advances by an elapsed time in seconds, reporting whether it finished on this tick.
    ///
    /// A clock that goes backwards counts as no time passing rather than as time returned. A clock
    /// reporting a non-finite delta **completes the change**, and the direction of that choice is the
    /// opposite of the one [`crate::settings::Transaction::tick`] makes about the same input — because the
    /// hazards are opposite. There, never firing leaves somebody looking at a screen they cannot use;
    /// here, never finishing leaves the interface stuck half-faded between two screens. Both resolve
    /// toward the state the user is not trapped in.
    pub fn advance(&mut self, elapsed: f32) -> bool {
        if self.is_done() {
            return false;
        }
        if !elapsed.is_finite() {
            self.elapsed = self.motion.seconds();
            return true;
        }
        self.elapsed += elapsed.max(0.0);
        self.is_done()
    }

    /// How the arriving screen looks right now.
    ///
    /// `sliding` is false for a screen that fades in place — a modal, which does not displace what it
    /// covers and would look like it was shoving it aside if it slid.
    #[must_use]
    pub fn entering(&self, sliding: bool) -> Reveal {
        let progress = self.progress();
        Reveal {
            opacity: progress,
            offset: self.travel(sliding, self.direction.sign() * (1.0 - progress)),
        }
    }

    /// How the departing screen looks right now.
    #[must_use]
    pub fn leaving(&self, sliding: bool) -> Reveal {
        let progress = self.progress();
        Reveal {
            opacity: 1.0 - progress,
            offset: self.travel(sliding, -self.direction.sign() * progress),
        }
    }

    fn travel(&self, sliding: bool, fraction: f32) -> [f32; 2] {
        if sliding {
            [self.motion.distance() * fraction, 0.0]
        } else {
            [0.0, 0.0]
        }
    }
}

#[cfg(test)]
mod tests {
    // Every figure below is a duration or a fraction of one, chosen to be exact in binary, so an exact
    // comparison is the assertion rather than a tolerance nobody picked.
    #![allow(clippy::float_cmp)]

    use super::{Change, DEFAULT_TRAVEL, Heading, Motion, Reveal};

    fn change() -> Change {
        Change::new(Heading::Forward, Motion::DEFAULT)
    }

    #[test]
    fn a_change_starts_hidden_and_ends_shown() {
        let mut change = change();
        assert_eq!(change.fraction(), 0.0);
        assert!(change.entering(true).is_hidden());
        assert!(change.leaving(true).is_shown());
        assert!(change.advance(Motion::DEFAULT.duration));
        assert!(change.is_done());
        assert!(change.entering(true).is_shown());
        assert!(change.leaving(true).is_hidden());
    }

    #[test]
    fn an_instant_motion_completes_on_the_tick_that_starts_it() {
        // Both the default for anything that has not thought about motion and what a reduce-motion setting
        // selects, so it must run through the same arithmetic rather than a special path nobody exercises.
        let change = Change::new(Heading::Forward, Motion::INSTANT);
        assert_eq!(change.fraction(), 1.0);
        assert!(change.is_done());
        assert_eq!(change.entering(true), Reveal::SHOWN);
        assert!(change.leaving(true).is_hidden());
        // And advancing a finished change reports nothing further to do.
        let mut change = change;
        assert!(!change.advance(1.0));
    }

    #[test]
    fn the_curve_eases_out_so_the_first_frames_move_most() {
        // The asymmetry is the point: an ease-in-out barely moves for the frames a user is deciding whether
        // the interface responded, so it reads as latency even though it finishes at the same moment.
        let quarter = Motion::DEFAULT.duration / 4.0;
        let mut change = change();
        let mut covered = [0.0f32; 4];
        let mut previous = 0.0;
        for step in &mut covered {
            change.advance(quarter);
            *step = change.progress() - previous;
            previous = change.progress();
        }
        assert!(
            covered[0] > 0.25,
            "the first quarter covered only {}",
            covered[0]
        );
        // Every quarter covers less ground than the one before it, which is what deceleration means.
        for pair in covered.windows(2) {
            assert!(pair[1] < pair[0], "{covered:?} is not decelerating");
        }
        // And it still arrives exactly.
        assert_eq!(previous, 1.0);
    }

    #[test]
    fn a_forward_change_arrives_from_the_trailing_edge_and_back_from_the_leading_one() {
        let forward = Change::new(Heading::Forward, Motion::DEFAULT);
        assert_eq!(forward.entering(true).offset[0], DEFAULT_TRAVEL);
        assert_eq!(forward.leaving(true).offset[0], 0.0);
        let backward = Change::new(Heading::Backward, Motion::DEFAULT);
        assert_eq!(backward.entering(true).offset[0], -DEFAULT_TRAVEL);
        // Halfway through, the two screens are on opposite sides of where they are going.
        let mut halfway = Change::new(Heading::Forward, Motion::DEFAULT);
        halfway.advance(Motion::DEFAULT.duration / 2.0);
        let (entering, leaving) = (halfway.entering(true), halfway.leaving(true));
        assert!(
            entering.offset[0] > 0.0,
            "the arriving screen is still right"
        );
        assert!(leaving.offset[0] < 0.0, "the departing one has gone left");
    }

    #[test]
    fn a_screen_that_does_not_slide_only_fades() {
        // A modal does not displace what it covers, and would look like it was shoving it aside if it slid.
        let mut change = change();
        change.advance(Motion::DEFAULT.duration / 2.0);
        assert_eq!(change.entering(false).offset, [0.0, 0.0]);
        assert_eq!(change.leaving(false).offset, [0.0, 0.0]);
        // The fade still happens.
        assert!(change.entering(false).opacity > 0.0);
        assert!(change.entering(false).opacity < 1.0);
    }

    #[test]
    fn a_clock_that_goes_backwards_does_not_rewind_a_change() {
        let mut change = change();
        change.advance(Motion::DEFAULT.duration / 2.0);
        let halfway = change.progress();
        assert!(!change.advance(-10.0));
        assert_eq!(change.progress(), halfway);
    }

    #[test]
    fn a_clock_reporting_nonsense_completes_the_change_rather_than_stalling_it() {
        // The opposite choice from the settings revert window on the same input, because the hazards are
        // opposite: never firing there leaves somebody unable to see, and never finishing here leaves the
        // interface stuck half-faded between two screens.
        for broken in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut change = change();
            assert!(change.advance(broken), "{broken}");
            assert!(change.is_done());
            assert_eq!(change.entering(true), Reveal::SHOWN);
        }
    }

    #[test]
    fn a_motion_nobody_could_mean_animates_nothing_rather_than_failing() {
        // A motion setting is not worth failing a launch over, and the honest recovery is to change screens
        // without animating.
        for duration in [f32::NAN, f32::INFINITY, -1.0, 0.0] {
            let motion = Motion {
                duration,
                travel: 0.5,
            };
            assert_eq!(motion.seconds(), 0.0, "{duration}");
            assert!(Change::new(Heading::Forward, motion).is_done());
        }
        for travel in [f32::NAN, 2.0, -1.0] {
            let motion = Motion {
                duration: 0.2,
                travel,
            };
            assert!(
                (0.0..=1.0).contains(&motion.distance()),
                "{travel} gave {}",
                motion.distance()
            );
        }
    }

    #[test]
    fn a_reveal_reports_the_two_states_a_caller_can_shortcut_on() {
        assert!(Reveal::SHOWN.is_shown());
        assert!(!Reveal::SHOWN.is_hidden());
        let moved = Reveal {
            opacity: 1.0,
            offset: [0.1, 0.0],
        };
        assert!(!moved.is_shown(), "an offset screen is not at rest");
        let gone = Reveal {
            opacity: 0.0,
            offset: [0.0, 0.0],
        };
        assert!(gone.is_hidden());
    }
}
