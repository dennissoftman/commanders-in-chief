//! One sounding instance, and the handles that name one.
//!
//! # Why a handle carries a generation
//!
//! A voice slot is recycled the moment the sound in it ends, and a game holds handles far longer than
//! the sounds they named. A unit stores the handle of its engine loop, the unit is destroyed two
//! minutes later, and it stops "its" sound — which by then is a different sound belonging to something
//! else. With a bare index that is a silent, intermittent bug that reproduces about as often as a unit
//! happens to die shortly after another one starts a sound in the same slot.
//!
//! A generation counter makes it impossible instead of unlikely. The slot's counter advances on every
//! reuse, so a handle from the previous occupant fails to match and every operation on it is ignored.
//! [`crate::backend::conformance`] asserts this of any backend, because it is exactly the kind of
//! assumption a second implementation quietly fails to make.

use crate::bus::BusId;
use crate::spatial::Emitter;

/// A clip registered with a backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SoundId {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

impl SoundId {
    /// Builds a handle. Backends construct these; callers pass them back.
    #[must_use]
    pub const fn new(index: u32, generation: u32) -> Self {
        Self { index, generation }
    }

    /// The slot this handle names.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.index
    }
}

/// A sounding instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VoiceId {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

impl VoiceId {
    /// Builds a handle. Backends construct these; callers pass them back.
    #[must_use]
    pub const fn new(index: u32, generation: u32) -> Self {
        Self { index, generation }
    }

    /// The slot this handle names.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.index
    }
}

/// How much a voice is worth keeping when the budget is full.
///
/// A small integer rather than an enum of named tiers, because the comparison that matters is against
/// *other* live voices and a designer tuning one cue against another wants to nudge it rather than
/// promote it a whole category.
pub type Priority = u8;

/// The priority a cue gets when it says nothing.
pub const DEFAULT_PRIORITY: Priority = 128;

/// Everything needed to start a voice.
#[derive(Debug, Clone, PartialEq)]
pub struct VoiceSpec {
    /// The clip to play.
    pub sound: SoundId,
    /// Where it is routed.
    pub bus: BusId,
    /// Gain applied to this voice alone, in decibels.
    pub gain_db: f32,
    /// Playback rate multiplier. Doppler multiplies this rather than replacing it.
    pub pitch: f32,
    /// Whether the clip repeats when it ends.
    pub looping: bool,
    /// Where the sound is, or `None` for a sound with no position — music, narration, interface.
    pub emitter: Option<Emitter>,
    /// What the voice is worth when the budget is full.
    pub priority: Priority,
    /// Seconds to fade in from silence. Zero starts at full gain.
    ///
    /// A looping sound started at full gain clicks, because a clip's first sample is rarely zero and
    /// starting at an arbitrary amplitude is a step discontinuity.
    pub fade_in_seconds: f32,
}

impl VoiceSpec {
    /// A voice with sensible defaults on the named bus.
    #[must_use]
    pub fn new(sound: SoundId, bus: BusId) -> Self {
        Self {
            sound,
            bus,
            gain_db: 0.0,
            pitch: 1.0,
            looping: false,
            emitter: None,
            priority: DEFAULT_PRIORITY,
            fade_in_seconds: 0.0,
        }
    }

    /// Places the voice in the world.
    #[must_use]
    pub fn at(mut self, emitter: Emitter) -> Self {
        self.emitter = Some(emitter);
        self
    }

    /// Sets the voice's own gain.
    #[must_use]
    pub const fn with_gain_db(mut self, gain_db: f32) -> Self {
        self.gain_db = gain_db;
        self
    }

    /// Sets the playback rate multiplier.
    #[must_use]
    pub const fn with_pitch(mut self, pitch: f32) -> Self {
        self.pitch = pitch;
        self
    }

    /// Makes the voice repeat.
    #[must_use]
    pub const fn looping(mut self) -> Self {
        self.looping = true;
        self
    }

    /// Sets what the voice is worth against the budget.
    #[must_use]
    pub const fn with_priority(mut self, priority: Priority) -> Self {
        self.priority = priority;
        self
    }

    /// Fades the voice in rather than starting it at full gain.
    #[must_use]
    pub const fn faded_in(mut self, seconds: f32) -> Self {
        self.fade_in_seconds = seconds;
        self
    }
}

/// What a caller may change about a voice that is already sounding.
///
/// Deliberately a subset of [`VoiceSpec`]. A voice's bus, clip and loop flag are fixed for its
/// lifetime — changing any of them means stopping and starting, and offering them here would be
/// offering an operation that either silently restarts the sound or silently does nothing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoiceParams {
    /// Gain in decibels.
    pub gain_db: f32,
    /// Playback rate multiplier.
    pub pitch: f32,
    /// Where the sound now is, for a moving emitter.
    pub emitter: Option<Emitter>,
}

impl Default for VoiceParams {
    fn default() -> Self {
        Self {
            gain_db: 0.0,
            pitch: 1.0,
            emitter: None,
        }
    }
}

impl VoiceParams {
    /// The parameters a spec starts with, so a caller updating one field does not have to restate the
    /// others and accidentally reset them.
    #[must_use]
    pub fn from_spec(spec: &VoiceSpec) -> Self {
        Self {
            gain_db: spec.gain_db,
            pitch: spec.pitch,
            emitter: spec.emitter,
        }
    }

    /// Moves the emitter, leaving gain and pitch alone.
    #[must_use]
    pub const fn moved_to(mut self, position: [f32; 3]) -> Self {
        if let Some(emitter) = self.emitter.as_mut() {
            emitter.position = position;
        }
        self
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::{DEFAULT_PRIORITY, SoundId, VoiceId, VoiceParams, VoiceSpec};
    use crate::bus::BusId;
    use crate::spatial::Emitter;

    #[test]
    fn two_generations_of_one_slot_are_different_handles() {
        // The whole reason the generation exists. A unit holding a stale handle must not be able to
        // name the sound that landed in the slot after its own ended.
        let first = VoiceId::new(3, 0);
        let second = VoiceId::new(3, 1);
        assert_ne!(first, second);
        assert_eq!(first.index(), second.index());

        let sound = SoundId::new(7, 0);
        assert_ne!(sound, SoundId::new(7, 1));
    }

    #[test]
    fn a_spec_builds_up_without_restating_its_defaults() {
        let spec = VoiceSpec::new(SoundId::new(0, 0), BusId::Effects)
            .with_gain_db(-6.0)
            .with_pitch(1.2)
            .looping()
            .faded_in(0.25);
        assert_eq!(spec.gain_db, -6.0);
        assert_eq!(spec.pitch, 1.2);
        assert!(spec.looping);
        assert_eq!(spec.fade_in_seconds, 0.25);
        assert_eq!(spec.priority, DEFAULT_PRIORITY);
        assert_eq!(spec.bus, BusId::Effects);
    }

    #[test]
    fn params_taken_from_a_spec_carry_its_values_rather_than_defaults() {
        // A caller nudging a moving unit's position must not silently reset the gain the cue set.
        let spec = VoiceSpec::new(SoundId::new(0, 0), BusId::Effects)
            .with_gain_db(-12.0)
            .at(Emitter::default());
        let params = VoiceParams::from_spec(&spec);
        assert_eq!(params.gain_db, -12.0);

        let moved = params.moved_to([5.0, 0.0, 5.0]);
        assert_eq!(moved.gain_db, -12.0);
        assert_eq!(moved.emitter.expect("placed").position, [5.0, 0.0, 5.0]);
    }

    #[test]
    fn moving_a_voice_that_has_no_position_is_harmless() {
        let params = VoiceParams::default().moved_to([1.0, 2.0, 3.0]);
        assert!(params.emitter.is_none());
    }
}
