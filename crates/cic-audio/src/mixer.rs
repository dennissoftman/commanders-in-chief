//! A mixer written from scratch, and the [`Backend`] every test in this crate runs against.
//!
//! # Why the engine ships its own rather than requiring one
//!
//! Because the alternative is that a permissively licensed engine cannot make a sound without a
//! dependency whose licence it cannot promise. FMOD is proprietary and licensed per title; OpenAL Soft
//! is LGPL, which is fine dynamically linked and a real constraint statically. Neither can be the thing
//! this project *requires*, so both are optional and the default is this.
//!
//! It is also what makes the audio system testable at all. Every assertion in this crate runs headless,
//! with no device, at whatever rate the test picks — because the mixer is a pure function from voices
//! and a listener to frames.
//!
//! # What is in the signal path
//!
//! Per voice: a linear resampler, a distance filter, a constant-power pan, and a gain ramp. Per bus: an
//! effect chain and a gain. On master: the same, ending in a limiter.
//!
//! # Two things that are easy to get wrong and are audible immediately
//!
//! **Gain is ramped across a block, not applied per block.** Spatialisation is recomputed once per
//! [`RenderToBuffer::render`] call, because doing it per frame would be recomputing a square root and
//! six dot products for a quantity that changes over milliseconds. But *applying* the new gain from the
//! first sample of the block is a step discontinuity every block boundary — a click at the block rate,
//! which at a 512-frame buffer is a 94 Hz buzz that follows the camera. So the gain interpolates from
//! the previous block's value to this one's across the block, and only the filter cutoff steps.
//!
//! **Stopping a voice fades it out.** Cutting a waveform mid-cycle is a step from wherever the sample
//! happened to be to zero, which is a click — and stopping sounds is something a game does constantly.
//! The handle dies immediately, so [`Backend::is_playing`] reports what the caller expects, while the
//! slot lingers for a five-millisecond fade.

use crate::backend::{Backend, BackendError, Capabilities, RenderToBuffer, VoiceEvent};
use crate::bus::BusId;
use crate::dsp::{Biquad, Compressor, Effect, EffectSpec, FilterKind, decibels_to_gain};
use crate::sample::{Clip, DEFAULT_SAMPLE_RATE};
use crate::spatial::{Emitter, Listener, Spatialization};
use crate::voice::{Priority, SoundId, VoiceId, VoiceParams, VoiceSpec};

use std::sync::Arc;

/// How long a stopped voice takes to fade out, in seconds.
///
/// Long enough that no waveform is cut mid-cycle at any frequency a speaker reproduces, short enough
/// that a caller stopping a sound hears it stop.
const RELEASE_SECONDS: f32 = 0.005;

/// A clip slot.
#[derive(Debug)]
struct SoundSlot {
    generation: u32,
    clip: Option<Arc<Clip>>,
}

/// A voice slot's occupant.
#[derive(Debug)]
struct Playing {
    handle: VoiceId,
    clip: Arc<Clip>,
    sound: SoundId,
    bus: BusId,
    priority: Priority,
    /// Fractional read position in the clip, in its own frames.
    position: f64,
    /// How far the position advances per output frame, before pitch.
    base_step: f64,
    looping: bool,
    gain_db: f32,
    pitch: f32,
    emitter: Option<Emitter>,
    /// Where the per-channel gain currently is, which is where the next ramp starts.
    current_gains: [f32; 2],
    /// One filter per channel, standing in for air absorption and occlusion.
    filters: [Biquad; 2],
    /// The cutoff those filters are currently designed for, so an unchanged one is not redesigned.
    cutoff: f32,
    /// Fade-in progress from zero to one, and how far it advances per frame.
    fade_in: f32,
    fade_in_step: f32,
    /// Release progress from one to zero once the voice is no longer owned.
    release: f32,
    release_step: f32,
    /// Whether a handle still names this voice. False once stopped or stolen.
    owned: bool,
    /// Whether the caller stopped it, so it does not also report finishing.
    stopped_by_caller: bool,
}

/// A bus's running state.
#[derive(Debug)]
struct BusRuntime {
    gain_db: f32,
    muted: bool,
    effects: Vec<Effect>,
    buffer: Vec<[f32; 2]>,
    /// Smoothed gain, for the same reason a voice's is smoothed: a settings slider moved during
    /// playback would otherwise step once per block.
    current_gain: f32,
    /// Whether this bus has rendered a block yet.
    ///
    /// The first block must *start* at the configured gain rather than ramp to it from unity. Without
    /// this a mixer set up with music at -20 dB before the first frame plays that first block sliding
    /// down from 0 dB — and on a bus muted before playback began, the ramp puts a full-level transient
    /// into the master limiter, which then spends its whole 120 ms release recovering from a sound
    /// that was never supposed to be audible.
    primed: bool,
}

impl BusRuntime {
    fn new() -> Self {
        Self {
            gain_db: 0.0,
            muted: false,
            effects: Vec::new(),
            buffer: Vec::new(),
            current_gain: 1.0,
            primed: false,
        }
    }

    fn target_gain(&self) -> f32 {
        if self.muted {
            0.0
        } else {
            decibels_to_gain(self.gain_db)
        }
    }
}

/// The from-scratch mixer.
#[derive(Debug)]
pub struct SoftwareMixer {
    sample_rate: u32,
    max_voices: usize,
    sounds: Vec<SoundSlot>,
    voices: Vec<Option<Playing>>,
    generations: Vec<u32>,
    buses: [BusRuntime; BusId::COUNT],
    listener: Listener,
    pending: Vec<VoiceEvent>,
}

impl SoftwareMixer {
    /// Voices the mixer will sound at once before it starts stealing.
    pub const DEFAULT_MAX_VOICES: usize = 64;

    /// Builds a mixer at `sample_rate` with room for `max_voices`.
    ///
    /// The master bus starts with a limiter on it, which is not a mastering choice — see
    /// [`Compressor::limiter`].
    #[must_use]
    pub fn new(sample_rate: u32, max_voices: usize) -> Self {
        let sample_rate = if sample_rate == 0 {
            DEFAULT_SAMPLE_RATE
        } else {
            sample_rate
        };
        let max_voices = max_voices.max(1);
        let mut buses: [BusRuntime; BusId::COUNT] = std::array::from_fn(|_| BusRuntime::new());
        buses[BusId::Master.index()]
            .effects
            .push(Effect::Compressor(Compressor::limiter(sample_rate)));

        Self {
            sample_rate,
            max_voices,
            sounds: Vec::new(),
            voices: (0..max_voices).map(|_| None).collect(),
            generations: vec![0; max_voices],
            buses,
            listener: Listener::default(),
            pending: Vec::new(),
        }
    }

    /// A mixer at the default rate and voice count.
    #[must_use]
    pub fn with_defaults() -> Self {
        Self::new(DEFAULT_SAMPLE_RATE, Self::DEFAULT_MAX_VOICES)
    }

    /// Returns the clip behind a handle, if the handle is current.
    fn clip_of(&self, sound: SoundId) -> Option<&Arc<Clip>> {
        let slot = self.sounds.get(sound.index as usize)?;
        if slot.generation == sound.generation {
            slot.clip.as_ref()
        } else {
            None
        }
    }

    /// Finds a free slot, stealing the least valuable voice if there is none.
    ///
    /// Stealing compares priority first and *loudness* second, which is the tie-break that matters:
    /// with fifty rifles at the same priority the one to drop is the one furthest away, not whichever
    /// happens to sit in the highest slot index.
    fn allocate(&mut self, priority: Priority) -> Option<usize> {
        if let Some(index) = self.voices.iter().position(Option::is_none) {
            return Some(index);
        }

        let mut worst: Option<(usize, Priority, f32)> = None;
        for (index, slot) in self.voices.iter().enumerate() {
            let Some(playing) = slot else { continue };
            if !playing.owned {
                // Already releasing, so it is not a candidate to steal -- taking it back would cut
                // the fade that exists to prevent a click.
                continue;
            }
            let loudness = playing.current_gains[0] + playing.current_gains[1];
            let replace = match worst {
                None => true,
                Some((_, worst_priority, worst_loudness)) => {
                    playing.priority < worst_priority
                        || (playing.priority == worst_priority && loudness < worst_loudness)
                }
            };
            if replace {
                worst = Some((index, playing.priority, loudness));
            }
        }

        let (index, worst_priority, _) = worst?;
        // A new sound does not get to displace something more important than itself. Without this a
        // low-priority ambient loop starting every second would evict speech.
        if worst_priority > priority {
            return None;
        }
        if let Some(playing) = self.voices[index].as_mut() {
            let handle = playing.handle;
            playing.owned = false;
            playing.stopped_by_caller = true;
            self.pending.push(VoiceEvent::Stolen(handle));
        }
        self.generations[index] = self.generations[index].wrapping_add(1);
        Some(index)
    }

    /// Marks a slot as no longer owned and begins its release fade.
    fn release_slot(&mut self, index: usize, by_caller: bool) {
        if let Some(playing) = self.voices[index].as_mut()
            && playing.owned
        {
            playing.owned = false;
            playing.stopped_by_caller = by_caller;
            self.generations[index] = self.generations[index].wrapping_add(1);
        }
    }

    /// Computes where a voice's gains and filter should be heading this block.
    fn target_of(&self, playing: &Playing) -> ([f32; 2], f32, f64) {
        spatialize(&self.listener, playing)
    }
}

impl Backend for SoftwareMixer {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            renders_to_buffer: true,
            max_voices: self.max_voices,
            sample_rate: self.sample_rate,
            supports_bus_effects: true,
        }
    }

    fn load(&mut self, clip: Arc<Clip>) -> Result<SoundId, BackendError> {
        if let Some(index) = self.sounds.iter().position(|slot| slot.clip.is_none()) {
            self.sounds[index].generation = self.sounds[index].generation.wrapping_add(1);
            self.sounds[index].clip = Some(clip);
            let generation = self.sounds[index].generation;
            return Ok(SoundId::new(
                u32::try_from(index).map_err(|_| BackendError::SoundLimit {
                    maximum: u32::MAX as usize,
                })?,
                generation,
            ));
        }

        let index = u32::try_from(self.sounds.len()).map_err(|_| BackendError::SoundLimit {
            maximum: u32::MAX as usize,
        })?;
        self.sounds.push(SoundSlot {
            generation: 0,
            clip: Some(clip),
        });
        Ok(SoundId::new(index, 0))
    }

    fn unload(&mut self, sound: SoundId) {
        let Some(slot) = self.sounds.get_mut(sound.index as usize) else {
            return;
        };
        if slot.generation != sound.generation {
            return;
        }
        slot.clip = None;

        // Voices reading it are released rather than dropped outright, so unloading a bank during
        // play fades rather than clicks.
        let indices: Vec<usize> = self
            .voices
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.as_ref().is_some_and(|p| p.sound == sound))
            .map(|(index, _)| index)
            .collect();
        for index in indices {
            self.release_slot(index, true);
        }
    }

    fn play(&mut self, spec: &VoiceSpec) -> Option<VoiceId> {
        let clip = self.clip_of(spec.sound)?.clone();
        if clip.is_empty() {
            return None;
        }
        let index = self.allocate(spec.priority)?;

        let generation = self.generations[index];
        let handle = VoiceId::new(u32::try_from(index).ok()?, generation);

        let base_step = f64::from(clip.sample_rate()) / f64::from(self.sample_rate);
        #[expect(
            clippy::cast_precision_loss,
            reason = "sample rates in use are exact in `f32`"
        )]
        let rate = self.sample_rate as f32;
        let fade_in_step = if spec.fade_in_seconds > 0.0 {
            1.0 / (spec.fade_in_seconds * rate)
        } else {
            1.0
        };

        let mut playing = Playing {
            handle,
            clip,
            sound: spec.sound,
            bus: spec.bus,
            priority: spec.priority,
            position: 0.0,
            base_step,
            looping: spec.looping,
            gain_db: spec.gain_db,
            pitch: spec.pitch.clamp(0.01, 16.0),
            emitter: spec.emitter,
            current_gains: [0.0, 0.0],
            filters: [Biquad::identity(), Biquad::identity()],
            cutoff: Spatialization::UNFILTERED,
            fade_in: if spec.fade_in_seconds > 0.0 { 0.0 } else { 1.0 },
            fade_in_step,
            release: 1.0,
            release_step: 1.0 / (RELEASE_SECONDS * rate),
            owned: true,
            stopped_by_caller: false,
        };

        // Start the ramp *at* the correct gain rather than at zero. A voice ramping up from silence
        // over its first block would soften the attack of every impact in the game, which is the one
        // thing a percussive sound cannot afford.
        let (gains, _, _) = self.target_of(&playing);
        playing.current_gains = gains;

        self.voices[index] = Some(playing);
        Some(handle)
    }

    fn stop(&mut self, voice: VoiceId) {
        let index = voice.index as usize;
        if self.generations.get(index).copied() != Some(voice.generation) {
            return;
        }
        self.release_slot(index, true);
    }

    fn stop_bus(&mut self, bus: BusId) {
        let indices: Vec<usize> = self
            .voices
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.as_ref().is_some_and(|p| p.bus == bus && p.owned))
            .map(|(index, _)| index)
            .collect();
        for index in indices {
            self.release_slot(index, true);
        }
    }

    fn set_voice(&mut self, voice: VoiceId, params: &VoiceParams) {
        let index = voice.index as usize;
        if self.generations.get(index).copied() != Some(voice.generation) {
            return;
        }
        if let Some(playing) = self.voices.get_mut(index).and_then(Option::as_mut)
            && playing.owned
        {
            playing.gain_db = params.gain_db;
            playing.pitch = params.pitch.clamp(0.01, 16.0);
            playing.emitter = params.emitter;
        }
    }

    fn is_playing(&self, voice: VoiceId) -> bool {
        let index = voice.index as usize;
        self.generations.get(index).copied() == Some(voice.generation)
            && self
                .voices
                .get(index)
                .and_then(Option::as_ref)
                .is_some_and(|playing| playing.owned)
    }

    fn set_listener(&mut self, listener: &Listener) {
        self.listener = *listener;
    }

    fn set_bus_gain(&mut self, bus: BusId, gain_db: f32, muted: bool) {
        let runtime = &mut self.buses[bus.index()];
        runtime.gain_db = gain_db;
        runtime.muted = muted;
    }

    fn set_bus_effects(&mut self, bus: BusId, effects: &[EffectSpec]) -> Result<(), BackendError> {
        let rate = self.sample_rate;
        let runtime = &mut self.buses[bus.index()];
        runtime.effects = effects.iter().map(|spec| spec.instantiate(rate)).collect();
        if bus == BusId::Master {
            // The master limiter is not optional and is not part of the authored chain. A bank that
            // replaced the master effects would otherwise remove the one thing standing between a
            // loud mix and the converter.
            runtime
                .effects
                .push(Effect::Compressor(Compressor::limiter(rate)));
        }
        Ok(())
    }

    fn update(&mut self, _elapsed_seconds: f32, events: &mut Vec<VoiceEvent>) {
        events.append(&mut self.pending);
    }

    fn active_voices(&self) -> usize {
        self.voices
            .iter()
            .filter(|slot| slot.as_ref().is_some_and(|playing| playing.owned))
            .count()
    }
}

impl RenderToBuffer for SoftwareMixer {
    fn render(&mut self, output: &mut [[f32; 2]]) {
        let frames = output.len();
        if frames == 0 {
            return;
        }
        for bus in &mut self.buses {
            bus.buffer.clear();
            bus.buffer.resize(frames, [0.0, 0.0]);
        }
        #[expect(
            clippy::cast_precision_loss,
            reason = "a block is at most a few thousand frames"
        )]
        let inverse_frames = 1.0 / frames as f32;
        self.mix_voices(inverse_frames);
        self.sum_buses(inverse_frames, output);
    }
}

impl SoftwareMixer {
    /// Mixes every live voice into the buffer of the bus it is routed to.
    fn mix_voices(&mut self, inverse_frames: f32) {
        let sample_rate = self.sample_rate;
        // Split the borrow so voices can write into bus buffers while both are fields of `self`.
        let Self {
            voices,
            buses,
            listener,
            pending,
            generations,
            ..
        } = self;

        for (index, slot) in voices.iter_mut().enumerate() {
            let Some(playing) = slot.as_mut() else {
                continue;
            };

            // Spatialisation once per block. The gain it produces is ramped across the block; the
            // filter cutoff steps instead, which is inaudible because a filter's output is continuous
            // in its coefficients while a gain's is not.
            let (target_gains, cutoff, doppler) = spatialize(listener, playing);
            playing.retune(cutoff, sample_rate);

            let buffer = &mut buses[playing.bus.index()].buffer;
            let finished = playing.mix_into(buffer, inverse_frames, target_gains, doppler);

            if finished {
                if playing.owned {
                    pending.push(VoiceEvent::Finished(playing.handle));
                    generations[index] = generations[index].wrapping_add(1);
                }
                *slot = None;
            }
        }
    }

    /// Applies each bus's gain and effects, sums them into master, and writes master to `output`.
    fn sum_buses(&mut self, inverse_frames: f32, output: &mut [[f32; 2]]) {
        let master = BusId::Master.index();
        // `BusId::ALL` puts master last, so this needs one pass rather than two.
        for id in BusId::ALL {
            let index = id.index();
            let target = self.buses[index].target_gain();
            if !self.buses[index].primed {
                self.buses[index].current_gain = target;
                self.buses[index].primed = true;
            }
            let start = self.buses[index].current_gain;

            {
                let bus = &mut self.buses[index];
                for (frame_index, frame) in bus.buffer.iter_mut().enumerate() {
                    #[expect(
                        clippy::cast_precision_loss,
                        reason = "a block is at most a few thousand frames"
                    )]
                    let blend = frame_index as f32 * inverse_frames;
                    let gain = blend.mul_add(target - start, start);
                    frame[0] *= gain;
                    frame[1] *= gain;
                }
                for effect in &mut bus.effects {
                    for frame in &mut bus.buffer {
                        *frame = effect.process(*frame);
                    }
                }
                bus.current_gain = target;
            }

            if index == master {
                output.copy_from_slice(&self.buses[master].buffer);
            } else {
                let (into, from) = if master < index {
                    let (left, right) = self.buses.split_at_mut(index);
                    (&mut left[master], &right[0])
                } else {
                    let (left, right) = self.buses.split_at_mut(master);
                    (&mut right[0], &left[index])
                };
                for (accumulator, contribution) in into.buffer.iter_mut().zip(from.buffer.iter()) {
                    accumulator[0] += contribution[0];
                    accumulator[1] += contribution[1];
                }
            }
        }
    }
}

/// Returns the gains, filter cutoff, and Doppler ratio a voice should be heading toward.
fn spatialize(listener: &Listener, playing: &Playing) -> ([f32; 2], f32, f64) {
    let voice_gain = decibels_to_gain(playing.gain_db);
    match playing.emitter {
        None => ([voice_gain, voice_gain], Spatialization::UNFILTERED, 1.0),
        Some(emitter) => {
            let placed = Spatialization::of(listener, &emitter);
            (
                [placed.gains[0] * voice_gain, placed.gains[1] * voice_gain],
                placed.low_pass_hz,
                f64::from(placed.pitch),
            )
        }
    }
}

impl Playing {
    /// Points the distance filter at a new cutoff, keeping the history it has accumulated.
    ///
    /// Keeping the history is the whole reason [`Biquad::retune`] exists. Assigning a freshly designed
    /// filter over a running one discards two samples of state, which is a step discontinuity in the
    /// output -- so a voice moving smoothly away from the listener would click every time its cutoff
    /// moved far enough to be worth redesigning, which for a unit driving past is constantly.
    fn retune(&mut self, cutoff: f32, sample_rate: u32) {
        if (cutoff - self.cutoff).abs() <= 1.0 {
            return;
        }
        self.cutoff = cutoff;
        if cutoff >= Spatialization::UNFILTERED {
            return;
        }
        let designed = Biquad::design(
            FilterKind::LowPass,
            cutoff,
            std::f32::consts::FRAC_1_SQRT_2,
            0.0,
            sample_rate,
        );
        for filter in &mut self.filters {
            filter.retune(designed);
        }
    }

    /// Adds this voice into `buffer`, returning whether it has finished.
    fn mix_into(
        &mut self,
        buffer: &mut [[f32; 2]],
        inverse_frames: f32,
        target_gains: [f32; 2],
        doppler: f64,
    ) -> bool {
        let step = self.base_step * f64::from(self.pitch) * doppler;
        let start_gains = self.current_gains;
        let filtered = self.cutoff < Spatialization::UNFILTERED;
        #[expect(
            clippy::cast_precision_loss,
            reason = "a clip is bounded by `ClipLimits`, well below the 2^52 an `f64` holds exactly"
        )]
        let length = self.clip.frames() as f64;

        let mut ended = false;
        for (frame_index, out) in buffer.iter_mut().enumerate() {
            #[expect(
                clippy::cast_precision_loss,
                reason = "a block is at most a few thousand frames"
            )]
            let blend = frame_index as f32 * inverse_frames;

            let frame = self.clip.stereo_frame_at(self.position);
            self.position += step;
            if self.position >= length {
                if self.looping && length > 0.0 {
                    // Wrap rather than reset, so a loop whose length is not a whole number of output
                    // frames does not accumulate a drift of up to one frame every cycle.
                    self.position -= length * (self.position / length).floor();
                } else {
                    ended = true;
                }
            }

            if self.fade_in < 1.0 {
                self.fade_in = (self.fade_in + self.fade_in_step).min(1.0);
            }
            if !self.owned {
                self.release = (self.release - self.release_step).max(0.0);
            }

            let envelope = self.fade_in * self.release;
            let left = blend.mul_add(target_gains[0] - start_gains[0], start_gains[0]);
            let right = blend.mul_add(target_gains[1] - start_gains[1], start_gains[1]);

            let (sample_left, sample_right) = if filtered {
                (
                    self.filters[0].process(frame[0]),
                    self.filters[1].process(frame[1]),
                )
            } else {
                (frame[0], frame[1])
            };

            out[0] += sample_left * left * envelope;
            out[1] += sample_right * right * envelope;

            if ended || (!self.owned && self.release <= 0.0) {
                break;
            }
        }

        self.current_gains = target_gains;
        ended || (!self.owned && self.release <= 0.0)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp, clippy::cast_precision_loss)]

    use super::SoftwareMixer;
    use crate::backend::{Backend, RenderToBuffer, VoiceEvent, conformance};
    use crate::bus::BusId;
    use crate::dsp::EffectSpec;
    use crate::sample::{Clip, ClipLimits};
    use crate::spatial::{Attenuation, Emitter, Listener};
    use crate::voice::{VoiceParams, VoiceSpec};

    use std::sync::Arc;

    const RATE: u32 = 48_000;

    /// A clip of constant full-scale samples, which makes a gain measurable directly.
    fn constant(frames: usize) -> Arc<Clip> {
        Arc::new(Clip::new(RATE, 1, vec![1.0; frames], ClipLimits::DEFAULT).expect("build"))
    }

    fn peak(frames: &[[f32; 2]]) -> [f32; 2] {
        frames.iter().fold([0.0f32, 0.0f32], |acc, frame| {
            [acc[0].max(frame[0].abs()), acc[1].max(frame[1].abs())]
        })
    }

    #[test]
    fn the_software_mixer_satisfies_the_backend_contract() {
        // The properties any implementation must have. A second backend is where a trait's unwritten
        // assumptions turn into bugs, so they are written down as assertions rather than as prose.
        let mut mixer = SoftwareMixer::new(RATE, 8);
        conformance::check(&mut mixer);
    }

    #[test]
    fn a_played_voice_reaches_the_output() {
        let mut mixer = SoftwareMixer::new(RATE, 4);
        let sound = mixer.load(constant(4_800)).expect("load");
        mixer
            .play(&VoiceSpec::new(sound, BusId::Effects))
            .expect("room");

        let mut output = vec![[0.0f32; 2]; 256];
        mixer.render(&mut output);
        let level = peak(&output);
        assert!(level[0] > 0.5 && level[1] > 0.5, "silence: {level:?}");
    }

    #[test]
    fn rendering_before_anything_plays_produces_exact_silence() {
        let mut mixer = SoftwareMixer::with_defaults();
        let mut output = vec![[1.0f32; 2]; 128];
        mixer.render(&mut output);
        assert!(output.iter().all(|frame| frame == &[0.0, 0.0]));
    }

    #[test]
    fn a_zero_length_block_is_not_an_error() {
        let mut mixer = SoftwareMixer::with_defaults();
        mixer.render(&mut []);
    }

    #[test]
    fn a_voice_that_runs_out_reports_finishing_exactly_once() {
        let mut mixer = SoftwareMixer::new(RATE, 4);
        let sound = mixer.load(constant(64)).expect("load");
        let voice = mixer
            .play(&VoiceSpec::new(sound, BusId::Effects))
            .expect("room");

        let mut output = vec![[0.0f32; 2]; 256];
        mixer.render(&mut output);

        let mut events = Vec::new();
        mixer.update(1.0 / 60.0, &mut events);
        assert_eq!(events, vec![VoiceEvent::Finished(voice)]);
        assert!(!mixer.is_playing(voice));

        events.clear();
        mixer.render(&mut output);
        mixer.update(1.0 / 60.0, &mut events);
        assert!(events.is_empty(), "reported twice: {events:?}");
    }

    #[test]
    fn a_looping_voice_does_not_run_out() {
        let mut mixer = SoftwareMixer::new(RATE, 4);
        let sound = mixer.load(constant(64)).expect("load");
        let voice = mixer
            .play(&VoiceSpec::new(sound, BusId::Effects).looping())
            .expect("room");

        let mut output = vec![[0.0f32; 2]; 1_024];
        for _ in 0..4 {
            mixer.render(&mut output);
        }
        assert!(mixer.is_playing(voice));
        assert!(peak(&output)[0] > 0.5, "a loop went silent");
    }

    #[test]
    fn the_master_limiter_delays_the_whole_mix_and_the_amount_is_known() {
        // Worth its own test because it is the first thing to confuse anyone measuring the mixer:
        // the master limiter looks ahead, so the first frames out of a fresh mixer are the empty
        // lookahead buffer rather than a bug. Two tests below were written wrong before this was.
        let mut mixer = SoftwareMixer::new(RATE, 4);
        let sound = mixer.load(constant(48_000)).expect("load");
        mixer
            .play(&VoiceSpec::new(sound, BusId::Effects))
            .expect("room");

        let mut output = vec![[0.0f32; 2]; 512];
        mixer.render(&mut output);

        let first_sound = output
            .iter()
            .position(|frame| frame[0].abs() > 1e-6)
            .expect("the mixer produced nothing at all");
        assert_eq!(
            first_sound, 192,
            "the master latency is four attack constants at 1 ms and 48 kHz"
        );
    }

    #[test]
    fn stopping_a_voice_fades_rather_than_cutting() {
        // A waveform cut mid-cycle is a step to zero, which is a click -- and stopping sounds is
        // something a game does constantly. The claim is specifically that the descent is gradual,
        // so the test measures how many frames it takes rather than only that it happens.
        let mut mixer = SoftwareMixer::new(RATE, 4);
        let sound = mixer.load(constant(48_000)).expect("load");
        let voice = mixer
            .play(&VoiceSpec::new(sound, BusId::Effects))
            .expect("room");

        let mut output = vec![[0.0f32; 2]; 1_024];
        mixer.render(&mut output);
        mixer.stop(voice);

        // The handle dies immediately, which is what a caller expects to observe.
        assert!(!mixer.is_playing(voice));

        mixer.render(&mut output);
        let loud = output
            .iter()
            .rposition(|frame| frame[0].abs() > 0.5)
            .expect("the block held no full-level audio");
        let quiet = output
            .iter()
            .position(|frame| frame[0].abs() < 0.01)
            .expect("the voice never reached silence");
        assert!(quiet > loud, "silence came before the fade");
        assert!(
            quiet - loud > 100,
            "the descent took {} frames, which is a cut rather than a fade",
            quiet - loud
        );
    }

    #[test]
    fn gain_is_ramped_across_a_block_rather_than_stepped_at_its_start() {
        // Applying a new gain from the first sample of a block is a discontinuity at the block rate:
        // at 512 frames that is a 94 Hz buzz that follows the camera.
        let mut mixer = SoftwareMixer::new(RATE, 4);
        let sound = mixer.load(constant(48_000)).expect("load");
        let voice = mixer
            .play(&VoiceSpec::new(sound, BusId::Effects).with_gain_db(0.0))
            .expect("room");

        let mut output = vec![[0.0f32; 2]; 512];
        mixer.render(&mut output);

        mixer.set_voice(
            voice,
            &VoiceParams {
                gain_db: -40.0,
                ..VoiceParams::default()
            },
        );
        mixer.render(&mut output);

        let first = output[0][0].abs();
        let middle = output[256][0].abs();
        let last = output[511][0].abs();
        assert!(
            first > middle && middle > last,
            "the gain stepped instead of ramping: {first}, {middle}, {last}"
        );
        assert!(first > 0.5, "the ramp did not start from the old gain");
    }

    #[test]
    fn a_positioned_voice_is_panned_and_attenuated() {
        let mut mixer = SoftwareMixer::new(RATE, 4);
        let sound = mixer.load(constant(48_000)).expect("load");
        mixer.set_listener(&Listener::default());
        mixer
            .play(&VoiceSpec::new(sound, BusId::Effects).at(Emitter {
                position: [50.0, 0.0, 0.0],
                attenuation: Attenuation::None,
                ..Emitter::default()
            }))
            .expect("room");

        let mut output = vec![[0.0f32; 2]; 256];
        mixer.render(&mut output);
        let level = peak(&output);
        assert!(
            level[1] > level[0],
            "a source on the right is not panned right"
        );
    }

    #[test]
    fn a_voice_beyond_its_falloff_is_silent() {
        let mut mixer = SoftwareMixer::new(RATE, 4);
        let sound = mixer.load(constant(48_000)).expect("load");
        mixer
            .play(&VoiceSpec::new(sound, BusId::Effects).at(Emitter {
                position: [0.0, 0.0, -10_000.0],
                attenuation: Attenuation::Linear {
                    near: 5.0,
                    far: 100.0,
                },
                ..Emitter::default()
            }))
            .expect("room");

        let mut output = vec![[0.0f32; 2]; 256];
        mixer.render(&mut output);
        assert!(peak(&output)[0] < 1e-6);
    }

    #[test]
    fn a_muted_bus_silences_its_voices_and_nothing_else() {
        let mut mixer = SoftwareMixer::new(RATE, 4);
        let sound = mixer.load(constant(48_000)).expect("load");
        mixer
            .play(&VoiceSpec::new(sound, BusId::Music))
            .expect("room");
        mixer
            .play(&VoiceSpec::new(sound, BusId::Effects))
            .expect("room");
        mixer.set_bus_gain(BusId::Music, 0.0, true);

        let mut output = vec![[0.0f32; 2]; 512];
        // Enough blocks to clear both the bus gain ramp and the master limiter's lookahead.
        for _ in 0..4 {
            mixer.render(&mut output);
        }
        let with_effects = peak(&output)[0];

        mixer.set_bus_gain(BusId::Effects, 0.0, true);
        for _ in 0..4 {
            mixer.render(&mut output);
        }
        assert!(with_effects > 0.5, "the unmuted bus was already silent");
        assert!(peak(&output)[0] < 1e-3, "muting both left sound behind");
    }

    #[test]
    fn the_master_limiter_survives_a_bank_replacing_the_master_chain() {
        // A bank that could remove the master limiter could remove the only thing between a loud mix
        // and hard clipping in the converter.
        let mut mixer = SoftwareMixer::new(RATE, 64);
        let sound = mixer.load(constant(48_000)).expect("load");
        mixer
            .set_bus_effects(
                BusId::Master,
                &[EffectSpec::Filter {
                    filter: crate::dsp::FilterKind::HighShelf,
                    cutoff_hz: 8_000.0,
                    q: 0.707,
                    gain_db: 6.0,
                }],
            )
            .expect("set");

        for _ in 0..40 {
            mixer
                .play(&VoiceSpec::new(sound, BusId::Effects))
                .expect("room");
        }

        let mut output = vec![[0.0f32; 2]; 4_096];
        mixer.render(&mut output);
        mixer.render(&mut output);
        let level = peak(&output);
        assert!(
            level[0] <= 1.05 && level[1] <= 1.05,
            "forty voices reached {level:?}"
        );
    }

    #[test]
    fn a_full_mixer_steals_the_quietest_voice_of_equal_priority() {
        // With fifty rifles at one priority the one to drop is the furthest away, not whichever
        // happens to sit in the highest slot index.
        let mut mixer = SoftwareMixer::new(RATE, 2);
        let sound = mixer.load(constant(48_000)).expect("load");

        let near = mixer
            .play(&VoiceSpec::new(sound, BusId::Effects).with_gain_db(0.0))
            .expect("room");
        let far = mixer
            .play(&VoiceSpec::new(sound, BusId::Effects).with_gain_db(-40.0))
            .expect("room");

        let mut output = vec![[0.0f32; 2]; 64];
        mixer.render(&mut output);

        let third = mixer
            .play(&VoiceSpec::new(sound, BusId::Effects))
            .expect("stolen a slot");
        assert!(mixer.is_playing(near), "the loud voice was stolen");
        assert!(!mixer.is_playing(far), "the quiet voice survived");
        assert!(mixer.is_playing(third));

        let mut events = Vec::new();
        mixer.update(0.0, &mut events);
        assert!(events.contains(&VoiceEvent::Stolen(far)));
    }

    #[test]
    fn a_low_priority_sound_cannot_displace_a_higher_priority_one() {
        // Without this a looping ambience restarting every second would evict speech.
        let mut mixer = SoftwareMixer::new(RATE, 1);
        let sound = mixer.load(constant(48_000)).expect("load");
        let speech = mixer
            .play(&VoiceSpec::new(sound, BusId::Speech).with_priority(250))
            .expect("room");
        assert!(
            mixer
                .play(&VoiceSpec::new(sound, BusId::Ambience).with_priority(10))
                .is_none(),
            "a low-priority sound took the slot"
        );
        assert!(mixer.is_playing(speech));
    }

    #[test]
    fn unloading_a_clip_releases_the_voices_reading_it() {
        let mut mixer = SoftwareMixer::new(RATE, 4);
        let sound = mixer.load(constant(48_000)).expect("load");
        let voice = mixer
            .play(&VoiceSpec::new(sound, BusId::Effects))
            .expect("room");
        mixer.unload(sound);
        assert!(!mixer.is_playing(voice));
        assert!(mixer.play(&VoiceSpec::new(sound, BusId::Effects)).is_none());
    }

    #[test]
    fn a_reloaded_slot_does_not_answer_to_the_old_handle() {
        let mut mixer = SoftwareMixer::new(RATE, 4);
        let first = mixer.load(constant(128)).expect("load");
        mixer.unload(first);
        let second = mixer.load(constant(128)).expect("load");
        assert_eq!(first.index(), second.index(), "the slot was reused");
        assert_ne!(first, second);
        assert!(mixer.play(&VoiceSpec::new(first, BusId::Effects)).is_none());
        assert!(
            mixer
                .play(&VoiceSpec::new(second, BusId::Effects))
                .is_some()
        );
    }

    #[test]
    fn a_clip_at_another_sample_rate_plays_at_the_right_speed() {
        // A 24 kHz clip in a 48 kHz mixer must last twice as many output frames, not sound an octave
        // high. This is the resampler's whole job and it is silent when it is wrong.
        let mut mixer = SoftwareMixer::new(RATE, 4);
        let half_rate =
            Arc::new(Clip::new(RATE / 2, 1, vec![1.0; 100], ClipLimits::DEFAULT).expect("build"));
        let sound = mixer.load(half_rate).expect("load");
        let voice = mixer
            .play(&VoiceSpec::new(sound, BusId::Effects))
            .expect("room");

        let mut output = vec![[0.0f32; 2]; 150];
        mixer.render(&mut output);
        assert!(
            mixer.is_playing(voice),
            "100 frames at half rate must fill more than 150 output frames"
        );

        let mut rest = vec![[0.0f32; 2]; 100];
        mixer.render(&mut rest);
        let mut events = Vec::new();
        mixer.update(0.0, &mut events);
        assert_eq!(events, vec![VoiceEvent::Finished(voice)]);
    }

    #[test]
    fn a_pitch_of_zero_or_a_negative_one_cannot_stall_or_reverse_a_voice() {
        // Both come from data -- a bank's pitch range or a script -- and a zero step is a voice that
        // never ends while holding a slot.
        let mut mixer = SoftwareMixer::new(RATE, 4);
        let sound = mixer.load(constant(64)).expect("load");
        let voice = mixer
            .play(&VoiceSpec::new(sound, BusId::Effects).with_pitch(0.0))
            .expect("room");

        let mut output = vec![[0.0f32; 2]; 4_096];
        for _ in 0..64 {
            mixer.render(&mut output);
        }
        assert!(!mixer.is_playing(voice), "a zero pitch stalled the voice");
    }
}
