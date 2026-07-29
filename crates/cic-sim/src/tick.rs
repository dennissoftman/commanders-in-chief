//! The fixed-timestep accumulator: how a variable frame rate drives a fixed tick rate.
//!
//! This is the one piece of this crate that lives on the *presentation* side of the line. A host
//! feeds it wall-clock frame times; it answers "advance this many whole ticks, then render this far
//! between the last two states". The kernel never sees a frame time — which is the point, and why
//! this type exists here: every host needs the same loop, and a host writing its own is one
//! `while accumulated >= step` bug away from a simulation whose speed depends on the display.
//!
//! The pattern is the standard fixed-timestep game loop, described in many places (Glenn Fiedler's
//! *Fix Your Timestep!* is the usual citation).
//!
//! Wall time is presentation data. It never reaches simulation state — the tick *count* is what the
//! kernel advances by, and that count is what lockstep agrees on.

/// Turns variable frame times into whole ticks plus an interpolation fraction.
#[derive(Debug, Clone)]
pub struct TickAccumulator {
    tick_seconds: f64,
    accumulated: f64,
    limit: u32,
}

impl TickAccumulator {
    /// An accumulator for the given tick length, advancing at most `limit` ticks per frame.
    ///
    /// The limit is the spiral-of-death guard: a machine that cannot simulate in real time
    /// accumulates debt every frame, and paying all of it back each frame makes the next frame
    /// slower still. Past the limit, the debt is *dropped* — the simulation slows down honestly
    /// instead of freezing while it chases itself.
    ///
    /// # Panics
    ///
    /// Panics if `tick_seconds` is not finite and positive, or if `limit` is zero — each is a
    /// configuration error, not a state.
    #[must_use]
    pub fn new(tick_seconds: f64, limit: u32) -> Self {
        assert!(
            tick_seconds.is_finite() && tick_seconds > 0.0,
            "a tick must have positive finite length"
        );
        assert!(limit > 0, "a limit of zero ticks can never advance");
        Self {
            tick_seconds,
            accumulated: 0.0,
            limit,
        }
    }

    /// Feeds one frame's elapsed wall time and returns how many whole ticks to advance.
    ///
    /// A non-finite or negative elapsed time contributes nothing: a clock that jumped backwards or
    /// produced garbage should stall the simulation for a frame, not corrupt the accumulator.
    #[must_use]
    pub fn frame(&mut self, elapsed_seconds: f64) -> u32 {
        if elapsed_seconds.is_finite() && elapsed_seconds > 0.0 {
            self.accumulated += elapsed_seconds;
        }

        let mut ticks = 0;
        while self.accumulated >= self.tick_seconds && ticks < self.limit {
            self.accumulated -= self.tick_seconds;
            ticks += 1;
        }
        if ticks == self.limit && self.accumulated >= self.tick_seconds {
            // Debt beyond the limit is dropped, not owed: see `new`.
            self.accumulated %= self.tick_seconds;
        }
        ticks
    }

    /// How far between the last computed tick and the next one the current moment sits, in
    /// `[0, 1)`. What a renderer interpolates by.
    #[must_use]
    pub fn alpha(&self) -> f64 {
        self.accumulated / self.tick_seconds
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::TickAccumulator;

    #[test]
    fn whole_ticks_come_out_and_the_remainder_stays() {
        let mut accumulator = TickAccumulator::new(0.1, 100);
        assert_eq!(accumulator.frame(0.25), 2);
        assert!((accumulator.alpha() - 0.5).abs() < 1e-9);
        // 0.06 rather than 0.05: the residue of 0.25 is a hair under 0.05 in `f64`, and landing a
        // frame exactly on the tick boundary is the one case wall time never produces. A tick that
        // arrives a frame late by a rounding error is the accumulator working, not failing.
        assert_eq!(accumulator.frame(0.06), 1);
    }

    #[test]
    fn a_fast_frame_advances_nothing_and_loses_nothing() {
        let mut accumulator = TickAccumulator::new(0.1, 100);
        assert_eq!(accumulator.frame(0.04), 0);
        assert_eq!(accumulator.frame(0.04), 0);
        // The third frame's four hundredths tip the accumulated 0.12 over one tick.
        assert_eq!(accumulator.frame(0.04), 1);
    }

    #[test]
    fn the_limit_drops_debt_rather_than_owing_it() {
        let mut accumulator = TickAccumulator::new(0.1, 4);
        // A two-second stall at ten ticks a second is twenty ticks of debt; four are paid.
        assert_eq!(accumulator.frame(2.0), 4);
        // The debt is gone: the next ordinary frame owes at most its own time.
        assert_eq!(accumulator.frame(0.1), 1);
    }

    #[test]
    fn garbage_clock_readings_contribute_nothing() {
        let mut accumulator = TickAccumulator::new(0.1, 100);
        assert_eq!(accumulator.frame(f64::NAN), 0);
        assert_eq!(accumulator.frame(f64::INFINITY), 0);
        assert_eq!(accumulator.frame(-5.0), 0);
        assert_eq!(accumulator.alpha(), 0.0);
    }
}
