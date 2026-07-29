//! The frontend: everything that stays the same when the backend changes.
//!
//! # What lives above the boundary and why
//!
//! Which cue fires, which of its recordings, whether the voice budget can afford it, what the bus gains
//! currently are, and what the score is doing. All of it is game policy, and a project that switched
//! from the software mixer to FMOD and had to re-decide any of it would not have a replaceable backend
//! — it would have two audio systems.
//!
//! So this type owns the bank, the cue bookkeeping, the mix and the score, and talks to the backend
//! only in [`crate::voice::VoiceSpec`]s and bus gains.
//!
//! # Time is a parameter
//!
//! [`AudioEngine::update`] takes an elapsed duration. Nothing here reads a clock, for the reason
//! nothing in the renderer does: a test that cannot control time can only assert what happens
//! eventually, and an assertion about a fade is an assertion about *when*.

use crate::backend::{Backend, BackendError, VoiceEvent};
use crate::bank::{AudioRandom, Cue, CueState, SoundBank, choose_variant, spec_for};
use crate::bus::{BusId, Mix, Snapshot};
use crate::music::MusicDirector;
use crate::sample::Clip;
use crate::spatial::Listener;
use crate::voice::{SoundId, VoiceId, VoiceParams};

use std::collections::BTreeMap;
use std::sync::Arc;

/// A voice this engine started, and what it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Live {
    voice: VoiceId,
    cue: String,
}

/// The audio system a game holds.
#[derive(Debug)]
pub struct AudioEngine<B: Backend> {
    backend: B,
    bank: SoundBank,
    clips: BTreeMap<String, SoundId>,
    cue_state: BTreeMap<String, CueState>,
    mix: Mix,
    music: MusicDirector,
    random: AudioRandom,
    live: Vec<Live>,
    music_voices: Vec<VoiceId>,
    events: Vec<VoiceEvent>,
}

impl<B: Backend> AudioEngine<B> {
    /// Builds an engine over `backend`, playing cues from `bank`.
    ///
    /// `seed` seeds the audio system's *own* random stream. It is deliberately not a simulation seed
    /// and must never be one: drawing from a simulation stream is part of that simulation's state
    /// transition, and a machine whose audio drew one extra number has desynced. See
    /// [the determinism invariants](../../../docs/invariants/determinism.md).
    pub fn new(backend: B, bank: SoundBank, seed: u64) -> Self {
        Self {
            backend,
            bank,
            clips: BTreeMap::new(),
            cue_state: BTreeMap::new(),
            mix: Mix::new(),
            music: MusicDirector::new(),
            random: AudioRandom::new(seed),
            live: Vec::new(),
            music_voices: Vec::new(),
            events: Vec::new(),
        }
    }

    /// The backend, for a host that needs to drive its device.
    pub const fn backend(&self) -> &B {
        &self.backend
    }

    /// The backend, mutably.
    pub const fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    /// The bus gains and snapshots.
    pub const fn mix(&self) -> &Mix {
        &self.mix
    }

    /// The bus gains and snapshots, mutably. Changes reach the backend on the next
    /// [`Self::update`].
    pub const fn mix_mut(&mut self) -> &mut Mix {
        &mut self.mix
    }

    /// The score.
    pub const fn music(&self) -> &MusicDirector {
        &self.music
    }

    /// The score, mutably.
    pub const fn music_mut(&mut self) -> &mut MusicDirector {
        &mut self.music
    }

    /// The bank this engine plays from.
    pub const fn bank(&self) -> &SoundBank {
        &self.bank
    }

    /// Registers a decoded clip under the virtual path a bank refers to it by.
    ///
    /// # Errors
    ///
    /// Returns whatever the backend reports when it cannot hold another clip.
    pub fn bind_clip(
        &mut self,
        path: impl Into<String>,
        clip: Arc<Clip>,
    ) -> Result<(), BackendError> {
        let path = path.into();
        let sound = self.backend.load(clip)?;
        if let Some(previous) = self.clips.insert(path, sound) {
            // Rebinding a path is how a mod overrides a sound, and leaving the old clip loaded would
            // leak a slot on every override.
            self.backend.unload(previous);
        }
        Ok(())
    }

    /// Clip paths the bank refers to that have not been bound yet.
    ///
    /// A host loads these. Reported rather than assumed, because a cue whose clip was never bound
    /// fails silently at the moment it is triggered, which is the worst time to find out.
    #[must_use]
    pub fn missing_clips(&self) -> Vec<&str> {
        self.bank
            .clip_paths()
            .into_iter()
            .filter(|path| !self.clips.contains_key(*path))
            .collect()
    }

    /// Moves the ear.
    pub fn set_listener(&mut self, listener: &Listener) {
        self.backend.set_listener(listener);
    }

    /// Blends a snapshot in — ducking for a briefing, for instance.
    pub fn engage_snapshot(&mut self, name: impl Into<String>, snapshot: Snapshot) {
        self.mix.engage(name, snapshot);
    }

    /// Blends a snapshot back out.
    pub fn release_snapshot(&mut self, name: &str) {
        self.mix.release(name);
    }

    /// Triggers a cue at a world position, or with no position when `position` is `None`.
    ///
    /// Returns `None` — without it being an error — when the cue is unknown, its clip is not bound,
    /// its polyphony limit is reached, it is inside its cooldown, or the backend had no voice. Every
    /// one of those is ordinary; a caller that had to handle an error on each gunshot would end up
    /// ignoring it, which is why [`Self::trigger_failure`] exists for the case somebody does want to
    /// know.
    pub fn play(&mut self, cue_name: &str, position: Option<[f32; 3]>) -> Option<VoiceId> {
        if self.trigger_failure(cue_name).is_some() {
            return None;
        }
        let cue = self.bank.cue(cue_name)?;
        let state = self.cue_state.entry(cue_name.to_owned()).or_default();

        let variant_index = choose_variant(cue, state, &mut self.random)?;
        let clip_path = cue.variants.get(variant_index)?.clip.as_str();
        let sound = *self.clips.get(clip_path)?;

        let spec = spec_for(cue, variant_index, sound, position, &mut self.random);
        let voice = self.backend.play(&spec)?;

        let state = self.cue_state.entry(cue_name.to_owned()).or_default();
        state.live += 1;
        state.cooldown_remaining_ms = cue.cooldown_ms;
        state.last_variant = Some(variant_index);
        self.live.push(Live {
            voice,
            cue: cue_name.to_owned(),
        });
        Some(voice)
    }

    /// Why a trigger would be refused right now, or `None` if it would be accepted.
    ///
    /// Separate from [`Self::play`] so a content tool can report an unbound clip without making a
    /// sound, and so a designer tuning polyphony can see which limit is biting.
    #[must_use]
    pub fn trigger_failure(&self, cue_name: &str) -> Option<TriggerFailure> {
        let Some(cue) = self.bank.cue(cue_name) else {
            return Some(TriggerFailure::UnknownCue);
        };
        if cue
            .variants
            .iter()
            .all(|variant| !self.clips.contains_key(&variant.clip))
        {
            return Some(TriggerFailure::ClipNotBound);
        }
        let state = self.cue_state.get(cue_name);
        if let Some(state) = state {
            if state.live >= cue.polyphony {
                return Some(TriggerFailure::Polyphony);
            }
            if state.cooldown_remaining_ms > 0.0 {
                return Some(TriggerFailure::Cooldown);
            }
        }
        None
    }

    /// Stops a voice this engine started.
    pub fn stop(&mut self, voice: VoiceId) {
        self.backend.stop(voice);
        self.retire(voice);
    }

    /// Updates a playing voice — a moving unit's position, or a changing engine note.
    pub fn set_voice(&mut self, voice: VoiceId, params: &VoiceParams) {
        self.backend.set_voice(voice, params);
    }

    /// Starts a score track, replacing whatever was playing.
    ///
    /// Every layer starts on this call and none is ever restarted, which is the whole reason they stay
    /// in phase — see [`crate::music`]. A layer whose clip is not bound is skipped, and the track
    /// plays without it rather than not at all.
    pub fn play_music(&mut self, track: crate::music::MusicTrack) {
        for voice in std::mem::take(&mut self.music_voices) {
            self.backend.stop(voice);
        }
        self.music.play(track);

        let Some(incoming) = self.music.incoming() else {
            return;
        };
        let specs: Vec<_> = incoming
            .layers
            .iter()
            .filter_map(|layer| self.clips.get(&layer.clip).copied())
            .map(|sound| {
                crate::voice::VoiceSpec::new(sound, BusId::Music)
                    .looping()
                    // Layers are held at silence rather than stopped, so they start at whatever the
                    // current intensity says and are moved from there.
                    .with_gain_db(-120.0)
            })
            .collect();
        for spec in specs {
            if let Some(voice) = self.backend.play(&spec) {
                self.music_voices.push(voice);
            }
        }
    }

    /// Advances cooldowns, snapshot blends, the score, and reaps finished voices.
    pub fn update(&mut self, elapsed_seconds: f32) {
        let elapsed = elapsed_seconds.max(0.0);
        let elapsed_ms = elapsed * 1000.0;

        for state in self.cue_state.values_mut() {
            state.cooldown_remaining_ms = (state.cooldown_remaining_ms - elapsed_ms).max(0.0);
        }

        self.mix.advance(elapsed);
        for bus in BusId::ALL {
            self.backend
                .set_bus_gain(bus, self.mix.resolved_gain_db(bus), false);
        }

        self.music.advance(elapsed);
        let gains = self.music.incoming_gains();
        for (voice, gain) in self.music_voices.iter().zip(gains.iter()) {
            // A layer at zero gain is set to silence rather than stopped; stopping it would lose its
            // place in the bar and bringing it back would be wrong music rather than quiet music.
            let gain_db = if *gain <= 1e-4 {
                -120.0
            } else {
                crate::dsp::gain_to_decibels(*gain)
            };
            self.backend.set_voice(
                *voice,
                &VoiceParams {
                    gain_db,
                    ..VoiceParams::default()
                },
            );
        }

        self.events.clear();
        let mut events = std::mem::take(&mut self.events);
        self.backend.update(elapsed, &mut events);
        for event in &events {
            let (VoiceEvent::Finished(voice) | VoiceEvent::Stolen(voice)) = event;
            self.retire(*voice);
        }
        self.events = events;
    }

    /// Voice events raised by the last [`Self::update`].
    #[must_use]
    pub fn events(&self) -> &[VoiceEvent] {
        &self.events
    }

    /// How many voices this engine has running for a cue.
    #[must_use]
    pub fn live_count(&self, cue_name: &str) -> u32 {
        self.cue_state.get(cue_name).map_or(0, |state| state.live)
    }

    /// Stops everything on a bus — a scene change, or leaving a match.
    pub fn stop_bus(&mut self, bus: BusId) {
        self.backend.stop_bus(bus);
        let stopped: Vec<VoiceId> = self
            .live
            .iter()
            .filter(|live| {
                self.bank
                    .cue(&live.cue)
                    .is_some_and(|cue: &Cue| cue.bus == bus)
            })
            .map(|live| live.voice)
            .collect();
        for voice in stopped {
            self.retire(voice);
        }
        if bus == BusId::Music {
            self.music_voices.clear();
            self.music.stop();
        }
    }

    /// Drops the bookkeeping for a voice that is no longer sounding.
    fn retire(&mut self, voice: VoiceId) {
        let Some(position) = self.live.iter().position(|live| live.voice == voice) else {
            return;
        };
        let live = self.live.swap_remove(position);
        if let Some(state) = self.cue_state.get_mut(&live.cue) {
            state.live = state.live.saturating_sub(1);
        }
    }
}

/// Why a cue would not play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TriggerFailure {
    /// No cue by that name is in the bank.
    UnknownCue,
    /// None of the cue's variants has had its clip bound.
    ClipNotBound,
    /// The cue already has as many instances as it allows.
    Polyphony,
    /// The cue fired too recently.
    Cooldown,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::{AudioEngine, TriggerFailure};
    use crate::backend::{Backend, RenderToBuffer};
    use crate::bank::SoundBank;
    use crate::bus::{BusId, Snapshot};
    use crate::mixer::SoftwareMixer;
    use crate::music::{MusicLayer, MusicTrack};
    use crate::sample::{Clip, ClipLimits};

    use std::sync::Arc;

    const RATE: u32 = 48_000;

    const BANK: &str = r#"{
      "format_version": 1,
      "cues": {
        "unit.rifle.fire": {
          "bus": "effects",
          "variants": [
            { "clip": "audio/rifle_a.wav" },
            { "clip": "audio/rifle_b.wav" }
          ],
          "polyphony": 2,
          "cooldown_ms": 50.0,
          "attenuation": { "kind": "linear", "near": 5.0, "far": 400.0 }
        },
        "ui.click": {
          "bus": "interface",
          "variants": [{ "clip": "audio/click.wav" }],
          "polyphony": 4
        },
        "missing.sound": {
          "variants": [{ "clip": "audio/absent.wav" }]
        }
      }
    }"#;

    fn clip(frames: usize) -> Arc<Clip> {
        Arc::new(Clip::new(RATE, 1, vec![0.5; frames], ClipLimits::DEFAULT).expect("build"))
    }

    fn engine() -> AudioEngine<SoftwareMixer> {
        let bank = SoundBank::from_json(BANK.as_bytes()).expect("bank");
        let mut engine = AudioEngine::new(SoftwareMixer::new(RATE, 16), bank, 1);
        for path in ["audio/rifle_a.wav", "audio/rifle_b.wav", "audio/click.wav"] {
            engine.bind_clip(path, clip(4_800)).expect("bind");
        }
        engine
    }

    #[test]
    fn a_cue_plays_and_the_engine_knows_it_is_live() {
        let mut engine = engine();
        let voice = engine
            .play("unit.rifle.fire", Some([10.0, 0.0, 0.0]))
            .expect("play");
        assert!(engine.backend().is_playing(voice));
        assert_eq!(engine.live_count("unit.rifle.fire"), 1);
    }

    #[test]
    fn an_unbound_clip_is_reported_rather_than_failing_silently() {
        // A cue whose clip was never loaded makes no sound at the moment it is triggered, which is
        // the worst possible time to find out.
        let engine = engine();
        assert_eq!(engine.missing_clips(), vec!["audio/absent.wav"]);
        assert_eq!(
            engine.trigger_failure("missing.sound"),
            Some(TriggerFailure::ClipNotBound)
        );
        assert_eq!(
            engine.trigger_failure("no.such.cue"),
            Some(TriggerFailure::UnknownCue)
        );
        assert_eq!(engine.trigger_failure("ui.click"), None);
    }

    #[test]
    fn a_cooldown_refuses_a_second_trigger_and_then_expires() {
        // Forty units firing on one tick is forty voices starting on the same sample, which sums to
        // one loud crack rather than to a volley.
        let mut engine = engine();
        assert!(engine.play("unit.rifle.fire", None).is_some());
        assert_eq!(
            engine.trigger_failure("unit.rifle.fire"),
            Some(TriggerFailure::Cooldown)
        );
        assert!(engine.play("unit.rifle.fire", None).is_none());

        engine.update(0.060);
        assert_eq!(engine.trigger_failure("unit.rifle.fire"), None);
        assert!(engine.play("unit.rifle.fire", None).is_some());
    }

    #[test]
    fn polyphony_caps_a_cue_without_capping_the_others() {
        let mut engine = engine();
        for _ in 0..2 {
            assert!(engine.play("unit.rifle.fire", None).is_some());
            engine.update(0.060);
        }
        assert_eq!(
            engine.trigger_failure("unit.rifle.fire"),
            Some(TriggerFailure::Polyphony)
        );
        assert_eq!(engine.live_count("unit.rifle.fire"), 2);

        // A different cue is unaffected, which is what makes polyphony a per-cue budget rather than
        // a global one.
        assert!(engine.play("ui.click", None).is_some());
    }

    #[test]
    fn a_finished_voice_frees_its_polyphony_slot() {
        let mut engine = engine();
        let mut output = vec![[0.0f32; 2]; 4_096];

        for _ in 0..2 {
            engine.play("unit.rifle.fire", None).expect("play");
            engine.update(0.060);
        }
        assert_eq!(engine.live_count("unit.rifle.fire"), 2);

        // Render past the end of the clips, then update so the events are reaped.
        for _ in 0..4 {
            engine.backend_mut().render(&mut output);
        }
        engine.update(0.1);
        assert_eq!(
            engine.live_count("unit.rifle.fire"),
            0,
            "a cue whose voices ended must free its budget, or it goes permanently silent"
        );
        assert!(engine.play("unit.rifle.fire", None).is_some());
    }

    #[test]
    fn stopping_a_voice_frees_its_slot_immediately() {
        let mut engine = engine();
        let voice = engine.play("unit.rifle.fire", None).expect("play");
        engine.stop(voice);
        assert_eq!(engine.live_count("unit.rifle.fire"), 0);
    }

    #[test]
    fn a_snapshot_reaches_the_backend_through_update() {
        let mut engine = engine();
        engine.engage_snapshot("briefing", Snapshot::ducking());
        engine.update(1.0);
        assert_eq!(engine.mix().resolved_gain_db(BusId::Music), -12.0);

        engine.release_snapshot("briefing");
        engine.update(2.0);
        assert_eq!(engine.mix().resolved_gain_db(BusId::Music), 0.0);
    }

    #[test]
    fn music_layers_all_start_and_none_is_ever_restarted() {
        // The rule the whole layered approach rests on: they stay in phase because they began on the
        // same frame, so a layer that is inaudible is playing at silence rather than stopped.
        let mut engine = engine();
        engine
            .bind_clip("music/bed.wav", clip(48_000))
            .expect("bind");
        engine
            .bind_clip("music/brass.wav", clip(48_000))
            .expect("bind");

        engine.play_music(MusicTrack::new(vec![
            MusicLayer::bed("music/bed.wav"),
            MusicLayer::entering("music/brass.wav", 0.7, 1.0),
        ]));

        assert_eq!(engine.music_voices.len(), 2, "both layers must be playing");
        let started = engine.music_voices.clone();

        engine.music_mut().set_intensity(1.0);
        for _ in 0..120 {
            engine.update(1.0 / 60.0);
        }
        assert_eq!(
            engine.music_voices, started,
            "raising the intensity must not have restarted a layer"
        );
        for voice in &started {
            assert!(engine.backend().is_playing(*voice));
        }
    }

    #[test]
    fn a_music_layer_whose_clip_is_missing_is_skipped_rather_than_stopping_the_track() {
        let mut engine = engine();
        engine
            .bind_clip("music/bed.wav", clip(48_000))
            .expect("bind");
        engine.play_music(MusicTrack::new(vec![
            MusicLayer::bed("music/bed.wav"),
            MusicLayer::bed("music/absent.wav"),
        ]));
        assert_eq!(engine.music_voices.len(), 1);
    }

    #[test]
    fn rebinding_a_path_replaces_the_clip_rather_than_leaking_a_slot() {
        // How a mod overrides a sound. Leaving the old clip loaded would leak a backend slot on every
        // override, which in a heavily modded load is every sound in the game.
        let mut engine = engine();
        for _ in 0..32 {
            engine.bind_clip("audio/click.wav", clip(64)).expect("bind");
        }
        assert!(engine.play("ui.click", None).is_some());
    }

    #[test]
    fn stopping_the_music_bus_clears_the_score() {
        let mut engine = engine();
        engine
            .bind_clip("music/bed.wav", clip(48_000))
            .expect("bind");
        engine.play_music(MusicTrack::new(vec![MusicLayer::bed("music/bed.wav")]));
        engine.stop_bus(BusId::Music);
        assert!(engine.music_voices.is_empty());
    }

    #[test]
    fn the_audio_random_stream_is_reproducible_from_its_seed() {
        // Not for lockstep. For being able to reproduce a report that one cue sounds wrong.
        let sequence = |seed| {
            let bank = SoundBank::from_json(BANK.as_bytes()).expect("bank");
            let mut engine = AudioEngine::new(SoftwareMixer::new(RATE, 16), bank, seed);
            for path in ["audio/rifle_a.wav", "audio/rifle_b.wav"] {
                engine.bind_clip(path, clip(64)).expect("bind");
            }
            let mut chosen = Vec::new();
            for _ in 0..16 {
                engine.play("unit.rifle.fire", None);
                engine.update(0.1);
                chosen.push(
                    engine
                        .cue_state
                        .get("unit.rifle.fire")
                        .and_then(|state| state.last_variant),
                );
            }
            chosen
        };
        assert_eq!(sequence(99), sequence(99));
        assert_ne!(sequence(99), sequence(100));
    }
}
