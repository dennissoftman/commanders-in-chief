//! Named, versioned, seeded random streams.
//!
//! The [determinism invariants](../../../docs/invariants/determinism.md) state the rule this module
//! is the mechanism for: randomness comes from explicit streams, drawing from one is part of the
//! simulation's state transition, and **no stream may be consumed by presentation, logging, or
//! diagnostics** — a machine whose audio drew one extra number has desynced. Presentation carries its
//! own generators (`cic-audio` already does), and nothing here is reachable from a frame.
//!
//! # Why streams are named
//!
//! One shared generator makes every subsystem's draws depend on every other subsystem's draw *count*,
//! so adding one roll to pathfinding changes combat outcomes — an action-at-a-distance bug that
//! reproduces only from a full replay. A stream per concern (`combat`, `spawning`) makes each
//! sequence a function of its own consumption alone.
//!
//! # Why streams are versioned
//!
//! The version is folded into the seed. When a subsystem changes *how* it consumes its stream —
//! two draws per shot instead of one — bumping the version changes the whole sequence rather than
//! leaving the old and new code half-agreeing, and a replay recorded against the old version fails
//! its hash comparison immediately instead of drifting subtly.
//!
//! # The generator
//!
//! `SplitMix64`, as published (Steele, Lea & Flood, *Fast Splittable Pseudorandom Number Generators*,
//! OOPSLA 2014; the constants are the reference implementation's). Chosen because it is tiny enough
//! to verify by eye, entirely integer — so its determinism is unconditional, with no ADR 0007
//! considerations at all — and statistically far better than this use demands. Not cryptographic,
//! and nothing here needs it to be.

use std::collections::BTreeMap;

use crate::hash::StateHasher;

/// One deterministic stream.
#[derive(Debug, Clone)]
pub struct Stream {
    state: u64,
    draws: u64,
}

impl Stream {
    /// A stream at its seed.
    fn new(seed: u64) -> Self {
        Self {
            state: seed,
            draws: 0,
        }
    }

    /// The next 64 uniformly distributed bits.
    ///
    /// `SplitMix64`'s step, exactly as published.
    pub fn next_u64(&mut self) -> u64 {
        self.draws += 1;
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A uniformly distributed `f64` in `[0, 1)`.
    ///
    /// The top 53 bits scaled by 2^-53: one shift and one multiply, both exactly specified, so the
    /// result is bit-identical everywhere. 53 bits because that is the mantissa — every representable
    /// value in the range is reachable and none is rounded.
    pub fn next_real(&mut self) -> f64 {
        // Exact: a 53-bit integer converts to f64 losslessly, and the scale is a power of two.
        #[expect(
            clippy::cast_precision_loss,
            reason = "the value is at most 53 bits, which f64 represents exactly"
        )]
        let mantissa = (self.next_u64() >> 11) as f64;
        mantissa * (1.0 / 9_007_199_254_740_992.0)
    }

    /// A uniformly distributed integer below `bound`.
    ///
    /// Lemire's multiply-shift method (*Fast Random Integer Generation in an Interval*, ACM TOMACS
    /// 2019), with the rejection step that removes the modulo bias a bare remainder has. Entirely
    /// integer, so deterministic unconditionally.
    ///
    /// # Panics
    ///
    /// Panics if `bound` is zero — there is no integer below zero, and the caller asking for one is
    /// a programming error rather than data.
    pub fn next_below(&mut self, bound: u64) -> u64 {
        assert!(bound > 0, "next_below(0) has no possible result");
        let threshold = bound.wrapping_neg() % bound;
        loop {
            let roll = self.next_u64();
            let wide = u128::from(roll) * u128::from(bound);
            #[expect(
                clippy::cast_possible_truncation,
                reason = "the low 64 bits are exactly what the rejection test needs"
            )]
            let low = wide as u64;
            if low >= threshold {
                return (wide >> 64) as u64;
            }
        }
    }

    /// How many times this stream has been drawn from.
    #[must_use]
    pub fn draws(&self) -> u64 {
        self.draws
    }
}

/// The registry of streams, keyed by name.
///
/// A `BTreeMap` rather than a hash map, following the invariant that nothing whose order can reach
/// output may depend on a hasher: the per-tick hash folds streams in iteration order.
#[derive(Debug, Clone)]
pub struct Streams {
    session_seed: u64,
    streams: BTreeMap<&'static str, Stream>,
}

impl Streams {
    /// A registry with no streams, for a session with the given seed.
    #[must_use]
    pub fn new(session_seed: u64) -> Self {
        Self {
            session_seed,
            streams: BTreeMap::new(),
        }
    }

    /// Registers a stream. Registration order does not matter; the seed depends only on the
    /// session seed, the name, and the version.
    ///
    /// # Panics
    ///
    /// Panics on a repeated name — two subsystems sharing a stream by accident is exactly the
    /// action-at-a-distance coupling named streams exist to prevent, so it fails loudly at
    /// construction rather than subtly at run time.
    pub fn register(&mut self, name: &'static str, version: u32) {
        // The seed derivation: FNV-1a over the session seed, the name, and the version. Any
        // deterministic mix would do; this one is already in the crate and already pinned.
        let mut mixer = StateHasher::new();
        mixer.write_u64(self.session_seed);
        mixer.write_str(name);
        mixer.write_u64(u64::from(version));
        let previous = self.streams.insert(name, Stream::new(mixer.finish()));
        assert!(
            previous.is_none(),
            "random stream `{name}` is already registered"
        );
    }

    /// The stream with the given name.
    ///
    /// # Panics
    ///
    /// Panics on an unregistered name. Registration at construction is what keeps the set of
    /// streams closed and reviewable; a stream that could be conjured mid-run by a typo would be a
    /// sequence nobody registered and no review saw.
    pub fn stream(&mut self, name: &str) -> &mut Stream {
        self.streams
            .get_mut(name)
            .unwrap_or_else(|| panic!("random stream `{name}` was never registered"))
    }

    /// Folds every stream's state into a tick hash, in name order.
    pub fn write_state(&self, hasher: &mut StateHasher) {
        for (name, stream) in &self.streams {
            hasher.write_str(name);
            hasher.write_u64(stream.state);
            hasher.write_u64(stream.draws);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::{Stream, Streams};

    #[test]
    fn splitmix64_matches_the_reference_sequence() {
        // The first three outputs of the reference implementation seeded with zero. If these move,
        // the generator is no longer SplitMix64 and every recorded replay's hashes are invalid.
        let mut stream = Stream::new(0);
        assert_eq!(stream.next_u64(), 0xE220_A839_7B1D_CDAF);
        assert_eq!(stream.next_u64(), 0x6E78_9E6A_A1B9_65F4);
        assert_eq!(stream.next_u64(), 0x06C4_5D18_8009_454F);
    }

    #[test]
    fn a_real_is_in_the_unit_interval_and_reproducible() {
        let mut stream = Stream::new(42);
        let mut again = Stream::new(42);
        for _ in 0..1_000 {
            let value = stream.next_real();
            assert!((0.0..1.0).contains(&value));
            assert_eq!(value.to_bits(), again.next_real().to_bits());
        }
    }

    #[test]
    fn next_below_stays_below_and_reaches_everything() {
        let mut stream = Stream::new(7);
        let mut seen = [false; 5];
        for _ in 0..500 {
            let roll = stream.next_below(5);
            assert!(roll < 5);
            #[expect(
                clippy::cast_possible_truncation,
                reason = "the roll was just asserted below five"
            )]
            {
                seen[roll as usize] = true;
            }
        }
        assert!(seen.iter().all(|&hit| hit), "a value below 5 never came up");
    }

    #[test]
    fn streams_are_independent_of_registration_order() {
        let mut forward = Streams::new(99);
        forward.register("combat", 1);
        forward.register("spawning", 1);

        let mut backward = Streams::new(99);
        backward.register("spawning", 1);
        backward.register("combat", 1);

        assert_eq!(
            forward.stream("combat").next_u64(),
            backward.stream("combat").next_u64()
        );
        assert_eq!(
            forward.stream("spawning").next_u64(),
            backward.stream("spawning").next_u64()
        );
    }

    #[test]
    fn a_version_bump_changes_the_whole_sequence() {
        let mut old = Streams::new(1);
        old.register("combat", 1);
        let mut new = Streams::new(1);
        new.register("combat", 2);
        assert_ne!(
            old.stream("combat").next_u64(),
            new.stream("combat").next_u64()
        );
    }

    #[test]
    fn one_streams_draws_do_not_move_another() {
        // The action-at-a-distance failure, prevented: draining one stream leaves its neighbour's
        // sequence exactly where it was.
        let mut isolated = Streams::new(5);
        isolated.register("combat", 1);
        isolated.register("pathfinding", 1);
        for _ in 0..100 {
            isolated.stream("pathfinding").next_u64();
        }

        let mut untouched = Streams::new(5);
        untouched.register("combat", 1);
        untouched.register("pathfinding", 1);

        assert_eq!(
            isolated.stream("combat").next_u64(),
            untouched.stream("combat").next_u64()
        );
    }

    #[test]
    #[should_panic(expected = "never registered")]
    fn an_unregistered_stream_is_a_loud_failure() {
        let mut streams = Streams::new(0);
        streams.register("combat", 1);
        let _ = streams.stream("cmbat");
    }
}
