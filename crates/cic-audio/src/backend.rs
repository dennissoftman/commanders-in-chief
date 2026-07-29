//! The boundary a replaceable audio implementation sits behind.
//!
//! Recorded in full in [ADR 6001](../../../docs/adr/6001-audio-backend-boundary.md); the short version
//! is here because the shape of this trait is the decision.
//!
//! # Why the boundary is a command interface and not a sample sink
//!
//! The tempting design is to make a backend a *device*: this crate mixes, spatialises and filters, and
//! the backend receives finished stereo frames and hands them to hardware. It is simpler, it makes
//! every backend byte-identical, and it is the wrong boundary — because it can only ever have one
//! implementation worth having.
//!
//! FMOD and OpenAL are not devices. Each is a complete mixer with its own spatialisation, its own DSP
//! graph, and in FMOD's case its own authoring tool that a sound designer works in. Reducing either to
//! a sink throws away everything anyone would adopt it for, and leaves a "switchable backend" that
//! switches nothing but the last hundred microseconds of the signal path.
//!
//! So the boundary is drawn where the *engine's* concerns end instead. This crate decides which cue
//! fires, which variant of it, on which bus, at what priority, and whether the voice budget can afford
//! it — policy that belongs to the game and must not change when the audio library does. Everything
//! downstream of that decision is the backend's: mixing, spatialisation, filtering, and the device.
//!
//! # What that costs, stated plainly
//!
//! Two backends will not produce identical samples. A test asserting exact output can only be written
//! against a named implementation, which is why [`crate::mixer`] is the one the tests target and why
//! the properties asserted of *any* backend — in [`conformance`] — are behavioural rather than
//! numeric.
//!
//! # The in-tree implementation
//!
//! [`crate::mixer::SoftwareMixer`], written from scratch, with no dependency of any kind. It is the
//! default for reasons that are as much licensing as engineering: FMOD is proprietary and per-title
//! licensed, and OpenAL Soft is LGPL, which constrains static linking. Neither can be the thing a
//! permissively licensed engine requires in order to make a sound. See the ADR.

use crate::bus::BusId;
use crate::dsp::EffectSpec;
use crate::sample::Clip;
use crate::spatial::Listener;
use crate::voice::{SoundId, VoiceId, VoiceParams, VoiceSpec};

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::sync::Arc;

/// A failure from a backend.
///
/// Deliberately small. A backend that cannot start is a host problem reported once; a backend that
/// cannot play one sound must not be a `Result` the caller has to unwrap on every gunshot, which is
/// why [`Backend::play`] returns an optional handle rather than an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendError {
    /// The backend has no room for another clip.
    SoundLimit {
        /// How many clips it can hold.
        maximum: usize,
    },
    /// A sound handle did not name a loaded clip.
    UnknownSound,
    /// The backend could not start, or lost its device.
    Device {
        /// What the implementation reported.
        detail: String,
    },
    /// The backend does not implement something the caller asked for.
    Unsupported {
        /// What was asked for.
        what: &'static str,
    },
}

impl Display for BackendError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::SoundLimit { maximum } => {
                write!(
                    formatter,
                    "no room for another clip; the limit is {maximum}"
                )
            }
            Self::UnknownSound => write!(formatter, "sound handle does not name a loaded clip"),
            Self::Device { detail } => write!(formatter, "audio device failure: {detail}"),
            Self::Unsupported { what } => {
                write!(formatter, "this backend does not implement {what}")
            }
        }
    }
}

impl Error for BackendError {}

/// What a backend can do, so a host can adapt rather than assume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// Whether the backend produces frames for a caller-owned device, through [`RenderToBuffer`].
    ///
    /// False for a library that owns its own output thread, which is the FMOD and OpenAL case. A host
    /// checks this to decide whether it needs a device at all.
    pub renders_to_buffer: bool,
    /// Largest number of voices that may sound at once.
    pub max_voices: usize,
    /// The rate the backend runs at.
    pub sample_rate: u32,
    /// Whether the backend applies per-bus effect chains.
    pub supports_bus_effects: bool,
}

/// Something that happened to a voice without the caller asking for it.
///
/// Returned from [`Backend::update`] rather than through a callback, because a callback from an audio
/// thread runs on that thread — and a game that frees a unit's handle from inside one has a data race
/// that reproduces once a week.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum VoiceEvent {
    /// The voice reached the end of its clip and stopped.
    Finished(VoiceId),
    /// The voice was stopped early to make room for a higher-priority one.
    Stolen(VoiceId),
}

/// The replaceable half of the audio system.
///
/// Everything above this trait is engine policy and does not change when the implementation does.
/// Everything below it is mixing, spatialisation, and the device.
pub trait Backend {
    /// Reports what this implementation can do.
    fn capabilities(&self) -> Capabilities;

    /// Registers a decoded clip and returns a handle for playing it.
    ///
    /// Takes an [`Arc`] because a backend running its own thread must be able to hold the samples for
    /// as long as a voice is reading them, and the caller must not have to guess when that is.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::SoundLimit`] when the backend has no room, or
    /// [`BackendError::Device`] when the implementation rejects the clip.
    fn load(&mut self, clip: Arc<Clip>) -> Result<SoundId, BackendError>;

    /// Releases a clip. Voices still reading it are stopped first.
    fn unload(&mut self, sound: SoundId);

    /// Starts a voice, or returns `None` if the backend had no room for one.
    ///
    /// `None` rather than an error because failing to find a voice is *ordinary*: it is what a voice
    /// budget is for, and a battle at full volume hits it constantly. A caller that had to handle an
    /// error here would end up ignoring it.
    fn play(&mut self, spec: &VoiceSpec) -> Option<VoiceId>;

    /// Stops a voice. A handle for a voice that already ended is ignored.
    fn stop(&mut self, voice: VoiceId);

    /// Stops every voice on a bus, for a scene change.
    fn stop_bus(&mut self, bus: BusId);

    /// Updates a playing voice's gain, pitch, and position.
    ///
    /// Ignored for a voice that already ended, which is the common case for a moving unit that dies
    /// mid-sound.
    fn set_voice(&mut self, voice: VoiceId, params: &VoiceParams);

    /// Whether a voice is still sounding.
    fn is_playing(&self, voice: VoiceId) -> bool;

    /// Moves the ear.
    fn set_listener(&mut self, listener: &Listener);

    /// Sets a bus's gain in decibels and whether it is silenced.
    fn set_bus_gain(&mut self, bus: BusId, gain_db: f32, muted: bool);

    /// Replaces a bus's effect chain.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::Unsupported`] from a backend that does not implement bus effects, which
    /// [`Capabilities::supports_bus_effects`] reports in advance.
    fn set_bus_effects(&mut self, bus: BusId, effects: &[EffectSpec]) -> Result<(), BackendError>;

    /// Advances the backend by one game frame and appends anything that happened to `events`.
    ///
    /// Appends rather than returns, so a caller can reuse one buffer instead of allocating a vector
    /// every frame for the common case of nothing having happened.
    fn update(&mut self, elapsed_seconds: f32, events: &mut Vec<VoiceEvent>);

    /// How many voices are sounding.
    fn active_voices(&self) -> usize;
}

/// Implemented by a backend that produces frames for a device the *caller* owns.
///
/// Separate from [`Backend`] rather than a method on it, because a backend that owns its own output
/// thread has no meaningful implementation to give — and a trait method that every FFI backend would
/// have to stub out with an error is a design that has already decided which implementation is real.
pub trait RenderToBuffer {
    /// Fills `output` with the next frames, replacing whatever was there.
    fn render(&mut self, output: &mut [[f32; 2]]);
}

/// Behavioural properties any [`Backend`] must satisfy, as a reusable test body.
///
/// # Why this exists
///
/// A second backend is the moment a trait's unwritten assumptions become bugs, and they are the kind
/// nobody finds by reading — a handle that keeps working after the voice ended, a stop that silences
/// the wrong voice after a slot is recycled. Writing them down once, as executable assertions any
/// implementation can be run through, is what makes "switchable" a claim rather than a hope.
///
/// The properties here are deliberately behavioural. Two backends do not produce the same samples and
/// asserting that they do would be asserting the boundary is in the wrong place.
///
/// # Why this is not behind `cfg(test)`
///
/// Because the implementation it most needs to check is the one that is not in this tree. An FMOD or
/// OpenAL backend lives in its own crate — it is FFI, so it cannot be under this workspace's
/// `unsafe_code = "forbid"` — and a test-only module is invisible to it. A conformance suite that only
/// the reference implementation can run is a conformance suite that checks nothing.
pub mod conformance {
    use super::{Backend, VoiceEvent};
    use crate::bus::BusId;
    use crate::sample::{Clip, ClipLimits};
    use crate::voice::{VoiceParams, VoiceSpec};

    use std::sync::Arc;

    /// A one-second mono tone, which is long enough that a voice playing it is still going on the
    /// next frame and short enough to run out inside a test.
    ///
    /// # Panics
    ///
    /// Panics if the tone exceeds [`ClipLimits::DEFAULT`], which a second of audio cannot.
    #[must_use]
    pub fn tone(sample_rate: u32) -> Arc<Clip> {
        let frames = sample_rate as usize;
        let samples = (0..frames)
            .map(|index| {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "a second of audio is below 2^24"
                )]
                let phase = std::f32::consts::TAU * 440.0 * index as f32 / sample_rate as f32;
                phase.sin() * 0.5
            })
            .collect();
        Arc::new(Clip::new(sample_rate, 1, samples, ClipLimits::DEFAULT).expect("build tone"))
    }

    /// Runs every property against `backend`.
    ///
    /// # Panics
    ///
    /// Panics when `backend` violates any of the properties, which is the point of calling it.
    pub fn check(backend: &mut impl Backend) {
        let rate = backend.capabilities().sample_rate;
        let sound = backend.load(tone(rate)).expect("load");
        let spec = VoiceSpec::new(sound, BusId::Effects);

        a_played_voice_is_reported_as_playing(backend, &spec);
        a_stopped_voice_stops(backend, &spec);
        a_stale_handle_does_not_control_a_recycled_voice(backend, &spec);
        stopping_a_bus_leaves_other_buses_alone(backend, sound);
        setting_a_dead_voice_is_ignored(backend, &spec);
    }

    fn a_played_voice_is_reported_as_playing(backend: &mut impl Backend, spec: &VoiceSpec) {
        let voice = backend.play(spec).expect("a fresh backend has room");
        assert!(backend.is_playing(voice));
        assert!(backend.active_voices() >= 1);
        backend.stop(voice);
    }

    fn a_stopped_voice_stops(backend: &mut impl Backend, spec: &VoiceSpec) {
        let voice = backend.play(spec).expect("room");
        backend.stop(voice);
        assert!(!backend.is_playing(voice));

        // Stopping twice must be harmless: a caller that stops a unit's sound when it dies and again
        // when it is removed from the world is doing something entirely reasonable.
        backend.stop(voice);
    }

    fn a_stale_handle_does_not_control_a_recycled_voice(
        backend: &mut impl Backend,
        spec: &VoiceSpec,
    ) {
        // The bug a bare index would produce, and the reason `VoiceId` carries a generation. A unit
        // holding the handle of a sound that finished must not be able to silence whatever sound
        // landed in the slot afterwards.
        let first = backend.play(spec).expect("room");
        backend.stop(first);
        let second = backend.play(spec).expect("room");
        assert_ne!(first, second, "a recycled slot must not reissue its handle");

        backend.stop(first);
        assert!(
            backend.is_playing(second),
            "a stale handle silenced a live voice"
        );
        backend.stop(second);
    }

    fn stopping_a_bus_leaves_other_buses_alone(
        backend: &mut impl Backend,
        sound: crate::voice::SoundId,
    ) {
        let effects = backend
            .play(&VoiceSpec::new(sound, BusId::Effects))
            .expect("room");
        let music = backend
            .play(&VoiceSpec::new(sound, BusId::Music))
            .expect("room");

        backend.stop_bus(BusId::Effects);
        assert!(!backend.is_playing(effects));
        assert!(backend.is_playing(music), "the wrong bus was stopped");
        backend.stop(music);
    }

    fn setting_a_dead_voice_is_ignored(backend: &mut impl Backend, spec: &VoiceSpec) {
        // A moving unit that dies mid-sound is the common case, not an edge case: the game will
        // update the emitter position on a voice that ended a frame ago every time it happens.
        let voice = backend.play(spec).expect("room");
        backend.stop(voice);
        backend.set_voice(voice, &VoiceParams::default());

        let mut events = Vec::new();
        backend.update(1.0 / 60.0, &mut events);
        assert!(
            !events.contains(&VoiceEvent::Finished(voice)),
            "a voice stopped by the caller must not also report finishing"
        );
    }
}
