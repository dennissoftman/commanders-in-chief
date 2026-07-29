//! The effects a bus or a voice can be run through, and the state each one carries.
//!
//! # Two types, and the split matters
//!
//! [`EffectSpec`] is what a bank file holds: a description, serialisable, sample-rate independent, with
//! no buffers in it. [`Effect`] is what a mixer runs: filter histories, delay lines, envelope followers,
//! all sized for one specific sample rate. `EffectSpec::instantiate` is the only way to get from the
//! first to the second.
//!
//! Collapsing them into one type is the obvious simplification and it costs the thing that makes the
//! bank format work. A spec is data a designer edits and a diff shows; an effect is a megabyte of delay
//! line that a device change invalidates.
//!
//! # Everything here is written from scratch
//!
//! Reverb in particular, and the constants are derived in this file from stated arguments rather than
//! copied from a reference implementation — the same standard [the scenery sway](../../cic-render/src/scenery.rs)
//! was held to, and for the same provenance reason recorded in `LICENSING.md`.

use serde::{Deserialize, Serialize};

/// The steepest resonance a filter may be asked for.
///
/// A biquad's Q is a division by a term that goes to zero, so an unbounded Q from a data file is an
/// infinity in the coefficients and a burst of full-scale noise on the bus. Forty is far past any
/// musical resonance and finite.
const MAX_Q: f32 = 40.0;

/// What a filter does to the band around its cutoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterKind {
    /// Passes below the cutoff. The workhorse: occlusion, air absorption, "muffled" states.
    LowPass,
    /// Passes above the cutoff. Thins a sound without making it quieter.
    HighPass,
    /// Passes a band around the cutoff. Radio chatter and intercom voices.
    BandPass,
    /// Lifts or cuts a band around the cutoff, leaving everything else alone.
    Peaking,
    /// Lifts or cuts everything below the cutoff.
    LowShelf,
    /// Lifts or cuts everything above the cutoff.
    HighShelf,
}

/// A second-order filter section.
///
/// Direct form II transposed, which is the arrangement with the best numerical behaviour in single
/// precision at low cutoffs — and low cutoffs are exactly what occlusion asks for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    state: [f32; 2],
}

impl Biquad {
    /// A filter that passes everything through unchanged.
    #[must_use]
    pub const fn identity() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            state: [0.0, 0.0],
        }
    }

    /// Designs a filter of `kind` at `cutoff_hz` with resonance `q` and shelf/peak gain `gain_db`.
    ///
    /// Out-of-range arguments are clamped rather than refused: this is reached from a bank file and
    /// from a per-frame occlusion value, and a silently sane filter beats a mix that stops.
    #[must_use]
    pub fn design(
        kind: FilterKind,
        cutoff_hz: f32,
        q: f32,
        gain_db: f32,
        sample_rate: u32,
    ) -> Self {
        #[expect(
            clippy::cast_precision_loss,
            reason = "sample rates in use are at most a few hundred thousand, exact in `f32`"
        )]
        let rate = sample_rate.max(1) as f32;
        // Nyquist is the hard ceiling, and the cutoff is held below it rather than at it: the
        // bilinear transform's `tan` goes to infinity exactly there.
        let cutoff = cutoff_hz.clamp(10.0, rate * 0.49);
        let q = q.clamp(0.05, MAX_Q);

        let omega = std::f32::consts::TAU * cutoff / rate;
        let (sin, cos) = omega.sin_cos();
        let alpha = sin / (2.0 * q);
        let amplitude = 10.0f32.powf(gain_db.clamp(-60.0, 24.0) / 40.0);

        let (b0, b1, b2, a0, a1, a2) = match kind {
            FilterKind::LowPass => {
                let b1 = 1.0 - cos;
                (b1 * 0.5, b1, b1 * 0.5, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
            }
            FilterKind::HighPass => {
                let b1 = -(1.0 + cos);
                (
                    (1.0 + cos) * 0.5,
                    b1,
                    (1.0 + cos) * 0.5,
                    1.0 + alpha,
                    -2.0 * cos,
                    1.0 - alpha,
                )
            }
            FilterKind::BandPass => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            FilterKind::Peaking => (
                alpha.mul_add(amplitude, 1.0),
                -2.0 * cos,
                alpha.mul_add(-amplitude, 1.0),
                alpha.mul_add(1.0 / amplitude, 1.0),
                -2.0 * cos,
                alpha.mul_add(-(1.0 / amplitude), 1.0),
            ),
            FilterKind::LowShelf | FilterKind::HighShelf => {
                let root = 2.0 * amplitude.max(f32::EPSILON).sqrt() * alpha;
                let plus = amplitude + 1.0;
                let minus = amplitude - 1.0;
                if matches!(kind, FilterKind::LowShelf) {
                    (
                        amplitude * minus.mul_add(-cos, plus + root),
                        2.0 * amplitude * plus.mul_add(-cos, minus),
                        amplitude * minus.mul_add(-cos, plus - root),
                        minus.mul_add(cos, plus + root),
                        -2.0 * plus.mul_add(cos, minus),
                        minus.mul_add(cos, plus - root),
                    )
                } else {
                    (
                        amplitude * minus.mul_add(cos, plus + root),
                        -2.0 * amplitude * plus.mul_add(cos, minus),
                        amplitude * minus.mul_add(cos, plus - root),
                        minus.mul_add(-cos, plus + root),
                        2.0 * plus.mul_add(-cos, minus),
                        minus.mul_add(-cos, plus - root),
                    )
                }
            }
        };

        if a0.abs() <= f32::EPSILON {
            return Self::identity();
        }
        let inverse = 1.0 / a0;
        Self {
            b0: b0 * inverse,
            b1: b1 * inverse,
            b2: b2 * inverse,
            a1: a1 * inverse,
            a2: a2 * inverse,
            state: [0.0, 0.0],
        }
    }

    /// Runs one sample through, advancing the filter's history.
    pub fn process(&mut self, input: f32) -> f32 {
        let output = self.b0.mul_add(input, self.state[0]);
        self.state[0] = self.b1.mul_add(input, self.state[1]) - self.a1 * output;
        self.state[1] = self.b2.mul_add(input, -(self.a2 * output));
        // A denormal in a filter's history costs tens of times a normal multiply on some hardware,
        // and a filter fed silence decays *toward* denormals rather than to zero -- so a bus that has
        // gone quiet is exactly where it bites. Flushing anything this small is inaudible by four
        // orders of magnitude below the least significant bit of 24-bit audio.
        for value in &mut self.state {
            if value.abs() < 1e-20 {
                *value = 0.0;
            }
        }
        output
    }

    /// Discards the filter's history without changing its coefficients.
    pub const fn reset(&mut self) {
        self.state = [0.0, 0.0];
    }

    /// Adopts another filter's coefficients while keeping this one's history.
    ///
    /// This is what a *moving* filter needs, and assigning the newly designed filter over the running
    /// one is the mistake it exists to prevent: that would take the new coefficients and the new
    /// filter's empty history, which is a step discontinuity in the output. A voice whose cutoff
    /// tracks its distance from the listener redesigns constantly, so the click would be on every
    /// vehicle that drives past rather than in some rare case.
    pub const fn retune(&mut self, designed: Self) {
        self.b0 = designed.b0;
        self.b1 = designed.b1;
        self.b2 = designed.b2;
        self.a1 = designed.a1;
        self.a2 = designed.a2;
    }
}

/// A fixed-length circular buffer of past samples.
#[derive(Debug, Clone, PartialEq)]
struct DelayLine {
    buffer: Vec<f32>,
    write: usize,
}

impl DelayLine {
    /// Allocates a line holding `length` samples, at least one.
    fn new(length: usize) -> Self {
        Self {
            buffer: vec![0.0; length.max(1)],
            write: 0,
        }
    }

    /// Returns the oldest sample and writes `input` in its place.
    fn step(&mut self, input: f32) -> f32 {
        let output = self.buffer[self.write];
        self.buffer[self.write] = input;
        self.write += 1;
        if self.write >= self.buffer.len() {
            self.write = 0;
        }
        output
    }

    fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.write = 0;
    }
}

/// A comb filter: a delay with its own output fed back into it, which is one resonant echo train.
#[derive(Debug, Clone, PartialEq)]
struct Comb {
    line: DelayLine,
    feedback: f32,
    /// One-pole low-pass inside the loop, so each successive echo is duller than the last.
    damping: f32,
    filtered: f32,
}

impl Comb {
    fn step(&mut self, input: f32) -> f32 {
        let delayed = self.line.step(0.0);
        // Real rooms absorb high frequencies on every reflection, so a reverb whose echoes keep their
        // brightness sounds metallic however carefully its delays were chosen.
        self.filtered = self.damping.mul_add(self.filtered - delayed, delayed);
        let write = self.feedback.mul_add(self.filtered, input);
        let index = if self.line.write == 0 {
            self.line.buffer.len() - 1
        } else {
            self.line.write - 1
        };
        self.line.buffer[index] = write;
        delayed
    }
}

/// An allpass section: passes every frequency at equal level and disperses them in time.
#[derive(Debug, Clone, PartialEq)]
struct Allpass {
    line: DelayLine,
    gain: f32,
}

impl Allpass {
    fn step(&mut self, input: f32) -> f32 {
        let delayed = self.line.step(0.0);
        let output = delayed - input;
        let index = if self.line.write == 0 {
            self.line.buffer.len() - 1
        } else {
            self.line.write - 1
        };
        self.line.buffer[index] = self.gain.mul_add(delayed, input);
        output
    }
}

/// A reverberator: four comb filters in parallel into two allpass sections in series, per channel.
///
/// # Where the delay lengths come from
///
/// Not from a table. The four comb delays are spread across 30 to 45 milliseconds, which is the range
/// where individual reflections have stopped being audible as echoes and have not yet fused into a
/// single resonance; each is then nudged to the nearest *prime* number of samples.
///
/// The prime step is the load-bearing one and this renderer has already been caught by its absence
/// once: five water waves at related wavelengths interfered into a visible diamond lattice. A comb
/// bank has the same failure. Two combs at 1200 and 1800 samples both reinforce at every multiple of
/// 3600, so their echo trains land on top of each other forever and the tail rings at one pitch.
/// Prime lengths share no multiple below their product, so the trains never realign inside any tail
/// anyone will hear.
///
/// The right channel's lines are offset by a further prime, because two identical channels are a mono
/// reverb in a stereo bus and the width is most of what a reverb is for.
#[derive(Debug, Clone, PartialEq)]
pub struct Reverb {
    combs: [[Comb; 4]; 2],
    allpasses: [[Allpass; 2]; 2],
    mix: f32,
    width: f32,
}

impl Reverb {
    /// Comb delays in milliseconds, before the nudge to a prime sample count.
    const COMB_MS: [f32; 4] = [30.7, 35.1, 39.9, 44.3];
    /// Allpass delays in milliseconds. Short, because their job is to smear the comb output's
    /// individual echoes into density rather than to add time of their own.
    const ALLPASS_MS: [f32; 2] = [5.3, 1.7];
    /// The allpass feedback everyone converges on, and for a stated reason: at 0.5 the section's
    /// impulse response decays fast enough not to add its own resonance while still multiplying the
    /// echo count by enough to matter.
    const ALLPASS_GAIN: f32 = 0.5;
    /// How far the right channel's lines are shifted from the left's, in samples. A prime, so the two
    /// channels do not realign either.
    const STEREO_OFFSET: usize = 23;

    /// Builds a reverberator.
    ///
    /// `room_size` from `0.0` to `1.0` scales the tail length, `damping` from `0.0` to `1.0` sets how
    /// fast the high frequencies leave it, `width` from `0.0` to `1.0` sets the stereo spread, and
    /// `mix` from `0.0` to `1.0` is how much of the output is the reverberated signal.
    #[must_use]
    pub fn new(room_size: f32, damping: f32, width: f32, mix: f32, sample_rate: u32) -> Self {
        let room_size = room_size.clamp(0.0, 1.0);
        let damping = damping.clamp(0.0, 1.0);

        // The tail length the room size asks for, from a small room to a large hall.
        let decay_seconds = 0.35 + room_size * 3.4;

        let mut combs = [Self::silent_combs(), Self::silent_combs()];
        for (channel, bank) in combs.iter_mut().enumerate() {
            for (index, comb) in bank.iter_mut().enumerate() {
                let length = prime_at_or_below(
                    samples_for_ms(Self::COMB_MS[index], sample_rate)
                        + channel * Self::STEREO_OFFSET,
                );
                // A comb's feedback for a target decay: the loop must lose 60 dB over `decay_seconds`,
                // and it goes round once every `length` samples. Solving `g^n = 10^(-3)` for the number
                // of round trips in that time is the whole derivation.
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "a delay length is at most a few thousand samples, exact in `f32`"
                )]
                let round_trip = length as f32 / sample_rate.max(1) as f32;
                let feedback = 10.0f32.powf(-3.0 * round_trip / decay_seconds).min(0.98);
                *comb = Comb {
                    line: DelayLine::new(length),
                    feedback,
                    damping: damping * 0.4,
                    filtered: 0.0,
                };
            }
        }

        let mut allpasses = [Self::silent_allpasses(), Self::silent_allpasses()];
        for (channel, bank) in allpasses.iter_mut().enumerate() {
            for (index, allpass) in bank.iter_mut().enumerate() {
                let length = prime_at_or_below(
                    samples_for_ms(Self::ALLPASS_MS[index], sample_rate)
                        + channel * Self::STEREO_OFFSET,
                );
                *allpass = Allpass {
                    line: DelayLine::new(length),
                    gain: Self::ALLPASS_GAIN,
                };
            }
        }

        Self {
            combs,
            allpasses,
            mix: mix.clamp(0.0, 1.0),
            width: width.clamp(0.0, 1.0),
        }
    }

    fn silent_combs() -> [Comb; 4] {
        std::array::from_fn(|_| Comb {
            line: DelayLine::new(1),
            feedback: 0.0,
            damping: 0.0,
            filtered: 0.0,
        })
    }

    fn silent_allpasses() -> [Allpass; 2] {
        std::array::from_fn(|_| Allpass {
            line: DelayLine::new(1),
            gain: 0.0,
        })
    }

    /// Runs one stereo frame through.
    pub fn process(&mut self, frame: [f32; 2]) -> [f32; 2] {
        // The input to both channels is the mono sum, which is what a real room does: it does not
        // keep the two sides of a recording separate, it mixes them and returns a diffuse field.
        // Width is then applied to the *output*, where it belongs.
        let input = (frame[0] + frame[1]) * 0.5;

        let mut wet = [0.0f32; 2];
        for (channel, output) in wet.iter_mut().enumerate() {
            let mut sum = 0.0;
            for comb in &mut self.combs[channel] {
                sum += comb.step(input);
            }
            // Four parallel combs sum four uncorrelated trains, so the output is roughly twice one
            // train's amplitude. Dividing by the count keeps a reverb at full mix from being louder
            // than the signal it replaced.
            sum *= 0.25;
            for allpass in &mut self.allpasses[channel] {
                sum = allpass.step(sum);
            }
            *output = sum;
        }

        let middle = (wet[0] + wet[1]) * 0.5;
        let wide = [
            middle + (wet[0] - middle) * self.width,
            middle + (wet[1] - middle) * self.width,
        ];

        [
            self.mix.mul_add(wide[0] - frame[0], frame[0]),
            self.mix.mul_add(wide[1] - frame[1], frame[1]),
        ]
    }

    /// Clears the tail, for a scene change that must not carry the last room into the next one.
    pub fn reset(&mut self) {
        for bank in &mut self.combs {
            for comb in bank {
                comb.line.reset();
                comb.filtered = 0.0;
            }
        }
        for bank in &mut self.allpasses {
            for allpass in bank {
                allpass.line.reset();
            }
        }
    }
}

/// A feed-forward dynamics processor: a peak-hold envelope, a lookahead delay, and a gain computer.
///
/// One type serves as compressor and as limiter, because a limiter *is* a compressor at a high ratio
/// and a short attack. Two types would be two implementations of the same arithmetic that could drift
/// apart.
///
/// # Why there is a hold stage and a lookahead delay
///
/// Both were added because the plain textbook arrangement — smooth the rectified level, compute a gain
/// from it — failed the one assertion this type exists to satisfy. It let a sine at eight times full
/// scale out at **1.12**, and the two reasons are worth recording because neither is a tuning problem.
///
/// **A rectified sine passes through zero twice a cycle**, so an envelope that releases during the
/// trough has fallen by the time the next peak arrives. At 220 Hz with a 120 ms release the sag was
/// 2.6 dB, which is exactly the overshoot measured. The fix is a **hold**: a new peak arms a timer, and
/// the envelope may not release until it expires. Ten milliseconds covers every period down to 100 Hz.
///
/// **The attack ramp is applied to the peak that triggered it**, which is a contradiction — by the time
/// the envelope has reacted, the sample that caused it has already been multiplied by the old gain and
/// left. The fix is a **lookahead delay**: the envelope is computed from the incoming signal and the
/// gain is applied to a delayed copy, so the reduction is fully in place before the peak it was
/// computed for emerges. The delay is four attack time constants, which is when the exponential ramp
/// has covered 98% of its distance.
///
/// A limiter without both of these is not brick-wall. It is a compressor that mostly works, which on a
/// master bus means it mostly prevents clipping.
#[derive(Debug, Clone, PartialEq)]
pub struct Compressor {
    threshold_db: f32,
    ratio: f32,
    attack: f32,
    release: f32,
    makeup: f32,
    envelope_db: f32,
    held_db: f32,
    hold_remaining: u32,
    hold_samples: u32,
    lookahead: Vec<[f32; 2]>,
    write: usize,
}

impl Compressor {
    /// How long a peak is held before the envelope may start releasing, in milliseconds.
    ///
    /// One period of a 100 Hz tone. Shorter and the envelope sags between the peaks of any bass note;
    /// much longer and a quiet passage after a loud one stays ducked audibly.
    const HOLD_MS: f32 = 10.0;
    /// Lookahead as a multiple of the attack time constant. Four covers 98% of the exponential ramp.
    const LOOKAHEAD_ATTACKS: f32 = 4.0;
    /// Largest lookahead permitted, in milliseconds. Beyond this the delay itself is the problem: a
    /// gunshot arriving 50 ms after its muzzle flash is a synchronisation bug.
    const MAX_LOOKAHEAD_MS: f32 = 20.0;

    /// Builds a compressor. `attack_ms` and `release_ms` are the times to cover 63% of a step.
    #[must_use]
    pub fn new(
        threshold_db: f32,
        ratio: f32,
        attack_ms: f32,
        release_ms: f32,
        makeup_db: f32,
        sample_rate: u32,
    ) -> Self {
        let lookahead_ms =
            (attack_ms.max(0.0) * Self::LOOKAHEAD_ATTACKS).min(Self::MAX_LOOKAHEAD_MS);
        Self {
            threshold_db: threshold_db.clamp(-90.0, 0.0),
            ratio: ratio.max(1.0),
            attack: time_constant(attack_ms, sample_rate),
            release: time_constant(release_ms, sample_rate),
            makeup: decibels_to_gain(makeup_db.clamp(-24.0, 24.0)),
            envelope_db: -120.0,
            held_db: -120.0,
            hold_remaining: 0,
            hold_samples: u32::try_from(samples_for_ms(Self::HOLD_MS, sample_rate))
                .unwrap_or(u32::MAX),
            lookahead: vec![[0.0, 0.0]; samples_for_ms(lookahead_ms, sample_rate)],
            write: 0,
        }
    }

    /// A brick-wall limiter for the master bus.
    ///
    /// This is not a mastering choice, it is what makes a float mixer safe. Fifty voices summing at
    /// once will exceed full scale, and the alternative to catching it here is hard clipping in the
    /// device's converter — which is not "loud", it is a broadband crackle across the whole mix.
    #[must_use]
    pub fn limiter(sample_rate: u32) -> Self {
        Self::new(-1.0, 20.0, 1.0, 120.0, 0.0, sample_rate)
    }

    /// How many frames of delay this processor adds.
    ///
    /// Exposed because a caller synchronising audio against anything else has to know, and because a
    /// lookahead nobody can measure is a latency bug waiting to be blamed on something else.
    #[must_use]
    pub fn latency_frames(&self) -> usize {
        self.lookahead.len()
    }

    /// Runs one stereo frame through, keying both channels off the louder of the two.
    ///
    /// A shared key is the point rather than an economy: compressing the channels independently pulls
    /// a loud sound on one side toward the centre, so the stereo image moves whenever the compressor
    /// works. This is the same reason a mastering compressor is linked.
    pub fn process(&mut self, frame: [f32; 2]) -> [f32; 2] {
        let peak_db = gain_to_decibels(frame[0].abs().max(frame[1].abs()));

        // The hold stage. A new peak refills the timer; while it is running the envelope may rise but
        // not fall, which is what stops a rectified sine's zero crossings dragging it down.
        if peak_db >= self.held_db {
            self.held_db = peak_db;
            self.hold_remaining = self.hold_samples;
        } else if self.hold_remaining > 0 {
            self.hold_remaining -= 1;
        } else {
            self.held_db = self.release.mul_add(self.held_db - peak_db, peak_db);
        }

        let coefficient = if self.held_db > self.envelope_db {
            self.attack
        } else {
            self.release
        };
        self.envelope_db = coefficient.mul_add(self.envelope_db - self.held_db, self.held_db);

        let gain = decibels_to_gain(self.reduction_db()) * self.makeup;

        // The delayed frame is what the gain is applied to. With an empty lookahead buffer this
        // degenerates to processing the current frame, which is the correct behaviour for a
        // zero-attack setting rather than a special case.
        let delayed = if self.lookahead.is_empty() {
            frame
        } else {
            let output = self.lookahead[self.write];
            self.lookahead[self.write] = frame;
            self.write += 1;
            if self.write >= self.lookahead.len() {
                self.write = 0;
            }
            output
        };

        [delayed[0] * gain, delayed[1] * gain]
    }

    /// Returns how much the compressor is currently pulling down, in decibels. Never positive.
    ///
    /// Exposed because a mixer that is limiting hard is a mix that was built too loud, and that is a
    /// content problem which is invisible unless something reports it.
    #[must_use]
    pub fn reduction_db(&self) -> f32 {
        let over = self.envelope_db - self.threshold_db;
        if over > 0.0 {
            -over * (1.0 - 1.0 / self.ratio)
        } else {
            0.0
        }
    }

    /// Discards the envelope, the hold timer, and the lookahead buffer.
    fn reset(&mut self) {
        self.envelope_db = -120.0;
        self.held_db = -120.0;
        self.hold_remaining = 0;
        self.lookahead.fill([0.0, 0.0]);
        self.write = 0;
    }
}

/// A description of an effect, as a bank file holds it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum EffectSpec {
    /// A second-order filter.
    Filter {
        /// What the filter does to the band around the cutoff.
        filter: FilterKind,
        /// Corner frequency in hertz.
        cutoff_hz: f32,
        /// Resonance. `0.707` is the flattest response with no peak.
        #[serde(default = "default_q")]
        q: f32,
        /// Peak or shelf gain in decibels. Ignored by the pass filters.
        #[serde(default)]
        gain_db: f32,
    },
    /// A reverberator.
    Reverb {
        /// Tail length, from a small room at `0.0` to a hall at `1.0`.
        room_size: f32,
        /// How quickly the tail loses its high frequencies.
        #[serde(default = "default_damping")]
        damping: f32,
        /// Stereo spread of the tail.
        #[serde(default = "default_width")]
        width: f32,
        /// How much of the output is reverberated.
        mix: f32,
    },
    /// A compressor or limiter.
    Compressor {
        /// Level above which gain reduction begins, in decibels.
        threshold_db: f32,
        /// How much of the excess is removed. `20.0` and above is a limiter.
        ratio: f32,
        /// Time to react to a level rise, in milliseconds.
        attack_ms: f32,
        /// Time to recover after one, in milliseconds.
        release_ms: f32,
        /// Gain applied after the reduction, in decibels.
        #[serde(default)]
        makeup_db: f32,
    },
}

const fn default_q() -> f32 {
    std::f32::consts::FRAC_1_SQRT_2
}

const fn default_damping() -> f32 {
    0.5
}

const fn default_width() -> f32 {
    1.0
}

impl EffectSpec {
    /// Allocates the running state this description asks for, at `sample_rate`.
    #[must_use]
    pub fn instantiate(self, sample_rate: u32) -> Effect {
        match self {
            Self::Filter {
                filter,
                cutoff_hz,
                q,
                gain_db,
            } => Effect::Filter {
                left: Biquad::design(filter, cutoff_hz, q, gain_db, sample_rate),
                right: Biquad::design(filter, cutoff_hz, q, gain_db, sample_rate),
            },
            Self::Reverb {
                room_size,
                damping,
                width,
                mix,
            } => Effect::Reverb(Box::new(Reverb::new(
                room_size,
                damping,
                width,
                mix,
                sample_rate,
            ))),
            Self::Compressor {
                threshold_db,
                ratio,
                attack_ms,
                release_ms,
                makeup_db,
            } => Effect::Compressor(Compressor::new(
                threshold_db,
                ratio,
                attack_ms,
                release_ms,
                makeup_db,
                sample_rate,
            )),
        }
    }
}

/// A running effect, with all the state its description implied.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Effect {
    /// A stereo pair of second-order sections, one per channel.
    Filter {
        /// The left channel's section.
        left: Biquad,
        /// The right channel's section, which carries its own history.
        right: Biquad,
    },
    /// A reverberator. Boxed because its delay lines dwarf the other variants, and an enum is as
    /// large as its largest member wherever one is stored.
    Reverb(Box<Reverb>),
    /// A compressor or limiter.
    Compressor(Compressor),
}

impl Effect {
    /// Runs one stereo frame through.
    pub fn process(&mut self, frame: [f32; 2]) -> [f32; 2] {
        match self {
            Self::Filter { left, right } => [left.process(frame[0]), right.process(frame[1])],
            Self::Reverb(reverb) => reverb.process(frame),
            Self::Compressor(compressor) => compressor.process(frame),
        }
    }

    /// Discards accumulated state — filter histories, reverb tails, envelope followers.
    pub fn reset(&mut self) {
        match self {
            Self::Filter { left, right } => {
                left.reset();
                right.reset();
            }
            Self::Reverb(reverb) => reverb.reset(),
            Self::Compressor(compressor) => compressor.reset(),
        }
    }
}

/// Converts a duration to a one-pole smoothing coefficient at `sample_rate`.
fn time_constant(milliseconds: f32, sample_rate: u32) -> f32 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "sample rates in use are exact in `f32`"
    )]
    let rate = sample_rate.max(1) as f32;
    let seconds = (milliseconds.max(0.0)) / 1000.0;
    if seconds <= 0.0 {
        0.0
    } else {
        (-1.0 / (seconds * rate)).exp()
    }
}

/// Converts a linear gain to decibels, with a floor so silence is a number rather than an infinity.
#[must_use]
pub fn gain_to_decibels(gain: f32) -> f32 {
    if gain <= 1e-6 {
        -120.0
    } else {
        20.0 * gain.log10()
    }
}

/// Converts decibels to a linear gain.
#[must_use]
pub fn decibels_to_gain(decibels: f32) -> f32 {
    if decibels <= -120.0 {
        0.0
    } else {
        10.0f32.powf(decibels / 20.0)
    }
}

/// Converts a duration in milliseconds to a whole number of samples.
///
/// Zero milliseconds returns zero rather than being floored at one sample. A delay line that must be
/// non-empty applies its own floor — the reverb's lengths pass through [`prime_at_or_below`], which
/// never returns less than two — while a lookahead of genuinely no frames has to stay zero, or a
/// processor asked for no attack acquires a sample of latency it was not asked for.
fn samples_for_ms(milliseconds: f32, sample_rate: u32) -> usize {
    #[expect(
        clippy::cast_precision_loss,
        reason = "sample rates in use are exact in `f32`"
    )]
    let rate = sample_rate.max(1) as f32;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the product of a bounded delay in milliseconds and a bounded rate is a few \
                  thousand, and both inputs are non-negative"
    )]
    let samples = (milliseconds.max(0.0) / 1000.0 * rate) as usize;
    samples
}

/// Returns the largest prime at or below `value`, or 2 for anything smaller.
///
/// Trial division, called a handful of times when a reverb is built and never in the audio path.
fn prime_at_or_below(value: usize) -> usize {
    let mut candidate = value.max(2);
    while candidate > 2 && !is_prime(candidate) {
        candidate -= 1;
    }
    candidate
}

/// Trial division by 2, 3, and then the 6k +/- 1 candidates.
fn is_prime(value: usize) -> bool {
    if value < 2 {
        return false;
    }
    if value.is_multiple_of(2) {
        return value == 2;
    }
    if value.is_multiple_of(3) {
        return value == 3;
    }
    let mut divisor: usize = 5;
    while divisor.saturating_mul(divisor) <= value {
        if value.is_multiple_of(divisor) || value.is_multiple_of(divisor + 2) {
            return false;
        }
        divisor += 6;
    }
    true
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp, clippy::cast_precision_loss)]

    use super::{
        Biquad, Compressor, Effect, EffectSpec, FilterKind, Reverb, decibels_to_gain,
        gain_to_decibels, is_prime, prime_at_or_below,
    };

    const RATE: u32 = 48_000;

    /// Root-mean-square level of a signal, which is the only honest way to compare two of them.
    fn rms(samples: &[f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        #[expect(
            clippy::cast_precision_loss,
            reason = "test buffers are thousands long"
        )]
        let count = samples.len() as f32;
        (samples.iter().map(|s| s * s).sum::<f32>() / count).sqrt()
    }

    /// A sine at `hz`, run through `filter`, measured after the transient has passed.
    fn filtered_level(mut filter: Biquad, hz: f32) -> f32 {
        let mut output = Vec::with_capacity(4096);
        for index in 0..8192 {
            #[expect(clippy::cast_precision_loss, reason = "index is below 2^24")]
            let phase = std::f32::consts::TAU * hz * index as f32 / RATE as f32;
            let sample = filter.process(phase.sin());
            if index >= 4096 {
                output.push(sample);
            }
        }
        rms(&output)
    }

    #[test]
    fn a_low_pass_passes_below_its_cutoff_and_stops_above_it() {
        let filter = Biquad::design(FilterKind::LowPass, 1_000.0, 0.707, 0.0, RATE);
        let low = filtered_level(filter, 100.0);
        let high = filtered_level(filter, 10_000.0);
        assert!(
            low > 0.6,
            "a decade below the cutoff should pass, got {low}"
        );
        assert!(high < 0.02, "two decades above should stop, got {high}");
    }

    #[test]
    fn a_high_pass_does_the_opposite() {
        let filter = Biquad::design(FilterKind::HighPass, 1_000.0, 0.707, 0.0, RATE);
        assert!(filtered_level(filter, 100.0) < 0.02);
        assert!(filtered_level(filter, 10_000.0) > 0.6);
    }

    #[test]
    fn a_cutoff_at_or_past_nyquist_is_clamped_rather_than_dividing_by_zero() {
        // The bilinear transform's tangent goes to infinity exactly at Nyquist, so this is the input
        // that turns a bank file into a bus full of NaN.
        for cutoff in [f32::MAX, RATE as f32, RATE as f32 / 2.0, -100.0, 0.0] {
            let mut filter = Biquad::design(FilterKind::LowPass, cutoff, 0.707, 0.0, RATE);
            for _ in 0..64 {
                assert!(
                    filter.process(0.5).is_finite(),
                    "cutoff {cutoff} produced a non-finite sample"
                );
            }
        }
    }

    #[test]
    fn an_unbounded_resonance_from_a_data_file_cannot_explode_the_bus() {
        let mut filter = Biquad::design(FilterKind::BandPass, 1_000.0, f32::MAX, 0.0, RATE);
        let mut peak = 0.0f32;
        for index in 0..48_000 {
            #[expect(clippy::cast_precision_loss, reason = "index is below 2^24")]
            let phase = std::f32::consts::TAU * 1_000.0 * index as f32 / RATE as f32;
            peak = peak.max(filter.process(phase.sin()).abs());
        }
        assert!(peak.is_finite() && peak < 100.0, "peak reached {peak}");
    }

    #[test]
    fn a_filter_fed_silence_settles_to_exact_zero_rather_than_to_denormals() {
        // Denormals cost tens of times a normal multiply on some hardware, and a decaying filter
        // heads straight into them -- so a bus that has just gone quiet is where it would bite.
        let mut filter = Biquad::design(FilterKind::LowPass, 200.0, 0.707, 0.0, RATE);
        filter.process(1.0);
        for _ in 0..10_000 {
            filter.process(0.0);
        }
        assert_eq!(filter.state, [0.0, 0.0]);
    }

    #[test]
    fn the_reverb_delay_lengths_are_prime_and_therefore_do_not_realign() {
        // The lattice failure this project has already been caught by once, in the water waves. Two
        // combs at related lengths reinforce at every common multiple and the tail rings at a pitch.
        let reverb = Reverb::new(0.7, 0.5, 1.0, 0.4, RATE);
        for bank in &reverb.combs {
            for comb in bank {
                assert!(
                    is_prime(comb.line.buffer.len()),
                    "comb length {} is not prime",
                    comb.line.buffer.len()
                );
            }
        }
        // And they are all different, which primality alone would not guarantee.
        let mut lengths: Vec<usize> = reverb.combs[0]
            .iter()
            .chain(reverb.combs[1].iter())
            .map(|comb| comb.line.buffer.len())
            .collect();
        lengths.sort_unstable();
        let count = lengths.len();
        lengths.dedup();
        assert_eq!(lengths.len(), count, "two comb lines share a length");
    }

    #[test]
    fn the_reverb_tail_outlives_the_sound_that_caused_it_and_then_decays() {
        let mut reverb = Reverb::new(0.8, 0.4, 1.0, 1.0, RATE);
        for _ in 0..64 {
            reverb.process([1.0, 1.0]);
        }

        let mut early = Vec::new();
        for _ in 0..4_800 {
            early.push(reverb.process([0.0, 0.0])[0]);
        }
        let mut late = Vec::new();
        for _ in 0..4_800 {
            late.push(reverb.process([0.0, 0.0])[0]);
        }

        assert!(rms(&early) > 1e-4, "there is no tail at all");
        assert!(rms(&late) < rms(&early), "the tail is not decaying");
    }

    #[test]
    fn the_reverb_is_stereo_rather_than_two_copies_of_one_channel() {
        // Two identical channels are a mono reverb in a stereo bus, and width is most of the point.
        let mut reverb = Reverb::new(0.7, 0.5, 1.0, 1.0, RATE);
        reverb.process([1.0, 1.0]);
        let mut difference = 0.0f32;
        for _ in 0..4_800 {
            let frame = reverb.process([0.0, 0.0]);
            difference = difference.max((frame[0] - frame[1]).abs());
        }
        assert!(difference > 1e-5, "the two channels are identical");
    }

    #[test]
    fn resetting_a_reverb_leaves_no_tail() {
        let mut reverb = Reverb::new(0.9, 0.3, 1.0, 1.0, RATE);
        for _ in 0..256 {
            reverb.process([1.0, -1.0]);
        }
        reverb.reset();
        assert_eq!(reverb.process([0.0, 0.0]), [0.0, 0.0]);
    }

    /// Runs a sine of the given amplitude through a processor, returning the peak output after the
    /// settling window. Anything before it is the attack ramp, which a limiter is entitled to.
    fn sine_peak(processor: &mut Compressor, amplitude: f32, hz: f32) -> f32 {
        let settle = RATE / 20;
        let mut peak = 0.0f32;
        for index in 0..RATE {
            let phase = std::f32::consts::TAU * hz * index as f32 / RATE as f32;
            let frame = processor.process([phase.sin() * amplitude; 2]);
            if index > settle {
                peak = peak.max(frame[0].abs());
            }
        }
        peak
    }

    #[test]
    fn the_limiter_holds_a_sum_of_many_voices_below_full_scale() {
        // The property that makes a float mixer safe. Without it fifty simultaneous voices clip in
        // the converter, which is a broadband crackle rather than a loud mix.
        //
        // This assertion is why the hold stage and the lookahead delay exist: the textbook
        // arrangement -- smooth the level, compute a gain -- let 1.12 through here.
        let mut limiter = Compressor::limiter(RATE);
        let peak = sine_peak(&mut limiter, 8.0, 220.0);
        assert!(peak <= 1.0, "the limiter let {peak} through");
    }

    #[test]
    fn the_limiter_holds_across_the_frequency_range_rather_than_at_one_pitch() {
        // The failure the hold stage fixes is frequency-dependent -- it is the envelope sagging
        // between the zero crossings of a rectified sine, so it is worst at low frequencies. Testing
        // one pitch would have passed with the bug present at another.
        for hz in [55.0, 110.0, 220.0, 880.0, 4_000.0f32] {
            let mut limiter = Compressor::limiter(RATE);
            let peak = sine_peak(&mut limiter, 8.0, hz);
            assert!(peak <= 1.0, "at {hz} Hz the limiter let {peak} through");
        }
    }

    #[test]
    fn the_limiter_is_transparent_below_its_threshold() {
        // Compared as a level rather than sample by sample, because the lookahead delays the output
        // and an identity check against the undelayed input would be measuring the delay.
        let mut limiter = Compressor::limiter(RATE);
        let peak = sine_peak(&mut limiter, 0.25, 220.0);
        assert!(
            (peak - 0.25).abs() < 1e-3,
            "a signal below the threshold came out at {peak}"
        );
        assert_eq!(limiter.reduction_db(), 0.0);
    }

    #[test]
    fn the_lookahead_delay_is_reported_rather_than_hidden() {
        // A caller synchronising audio against anything else has to know, and a latency nobody can
        // measure gets blamed on whatever is easiest to blame.
        let limiter = Compressor::limiter(RATE);
        assert_eq!(
            limiter.latency_frames(),
            192,
            "four attack constants at 1 ms"
        );

        // A processor with no attack has no lookahead and no delay at all, which must not be a
        // special case that misbehaves.
        let mut instant = Compressor::new(-6.0, 4.0, 0.0, 50.0, 0.0, RATE);
        assert_eq!(instant.latency_frames(), 0);
        assert!(instant.process([0.1, 0.1])[0].is_finite());
    }

    #[test]
    fn the_compressor_keys_both_channels_off_the_louder_one() {
        // Independent per-channel compression pulls a loud sound toward the centre, so the stereo
        // image moves whenever the compressor works.
        let mut compressor = Compressor::new(-20.0, 8.0, 1.0, 50.0, 0.0, RATE);
        let mut ratios = Vec::new();
        for _ in 0..4_800 {
            let frame = compressor.process([1.0, 0.1]);
            if frame[1].abs() > 1e-9 {
                ratios.push(frame[0] / frame[1]);
            }
        }
        let last = *ratios.last().expect("some output");
        assert!(
            (last - 10.0).abs() < 0.1,
            "the 10:1 input ratio became {last}, so the image moved"
        );
        assert!(compressor.reduction_db() < -6.0, "nothing was compressed");
    }

    #[test]
    fn decibel_conversion_round_trips_and_floors_at_silence() {
        for db in [-60.0, -20.0, -6.0, 0.0, 6.0f32] {
            let round_tripped = gain_to_decibels(decibels_to_gain(db));
            assert!(
                (round_tripped - db).abs() < 1e-3,
                "{db} became {round_tripped}"
            );
        }
        assert_eq!(decibels_to_gain(-200.0), 0.0);
        assert_eq!(gain_to_decibels(0.0), -120.0);
    }

    #[test]
    fn a_spec_round_trips_through_json_and_instantiates() {
        let spec = EffectSpec::Reverb {
            room_size: 0.6,
            damping: 0.4,
            width: 0.9,
            mix: 0.25,
        };
        let encoded = serde_json::to_string(&spec).expect("encode");
        assert!(encoded.contains("\"kind\":\"reverb\""));
        let decoded: EffectSpec = serde_json::from_str(&encoded).expect("decode");
        assert_eq!(decoded, spec);
        assert!(matches!(decoded.instantiate(RATE), Effect::Reverb(_)));
    }

    #[test]
    fn an_effect_the_engine_does_not_define_fails_to_load() {
        // The same rule the interface layer's action set enforces: data may not name an effect the
        // engine has no implementation of.
        assert!(serde_json::from_str::<EffectSpec>(r#"{"kind":"bitcrusher","bits":4}"#).is_err());
    }

    #[test]
    fn optional_effect_fields_take_documented_defaults() {
        let spec: EffectSpec =
            serde_json::from_str(r#"{"kind":"filter","filter":"low_pass","cutoff_hz":800}"#)
                .expect("decode");
        assert_eq!(
            spec,
            EffectSpec::Filter {
                filter: FilterKind::LowPass,
                cutoff_hz: 800.0,
                q: std::f32::consts::FRAC_1_SQRT_2,
                gain_db: 0.0,
            }
        );
    }

    #[test]
    fn resetting_an_effect_clears_its_state() {
        let mut effect = EffectSpec::Filter {
            filter: FilterKind::LowPass,
            cutoff_hz: 500.0,
            q: 0.707,
            gain_db: 0.0,
        }
        .instantiate(RATE);
        effect.process([1.0, 1.0]);
        effect.reset();
        assert_eq!(effect.process([0.0, 0.0]), [0.0, 0.0]);
    }

    #[test]
    fn the_prime_search_returns_primes() {
        assert_eq!(prime_at_or_below(0), 2);
        assert_eq!(prime_at_or_below(1), 2);
        assert_eq!(prime_at_or_below(2), 2);
        assert_eq!(prime_at_or_below(10), 7);
        assert_eq!(prime_at_or_below(1_000), 997);
        assert!(is_prime(2) && is_prime(3) && is_prime(5) && is_prime(997));
        assert!(!is_prime(1) && !is_prime(4) && !is_prime(9) && !is_prime(1_000));
    }
}
