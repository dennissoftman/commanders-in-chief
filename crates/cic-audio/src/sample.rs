//! Decoded audio: interleaved samples, the rate they were captured at, and how many run at once.
//!
//! # Why samples are `f32` here and integers on disk
//!
//! On disk a sample is an integer because that is what a recorder produces and what compresses; in a
//! mixer it is a float because every operation applied to it — gain, panning, filtering, summing — is
//! multiplication and addition, and doing those in fixed point means choosing a headroom policy at every
//! step. A float mixer sums to whatever it sums to and clips once, at the end, where a limiter can see
//! it.
//!
//! This is presentation, so the choice carries no determinism cost. Nothing in this crate reaches
//! simulation state; see the crate documentation for the one rule that does bind it.
//!
//! # Why the clip does not carry a name, a path, or an origin
//!
//! It arrives as bytes and it is identified by whatever the caller identified it by. A clip that knew
//! where it came from would be a clip that could only come from there, and the resource layer exists to
//! make a loose file, a package entry, and a mod override interchangeable.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

/// The rate a mixer runs at when a caller expresses no preference.
///
/// 48 kHz rather than 44.1: it is what every current device runs natively, so choosing it means the
/// common case resamples nothing, and the resampler that does exist is exercised by content rather than
/// by the output stage.
pub const DEFAULT_SAMPLE_RATE: u32 = 48_000;

/// The largest channel count a clip may declare.
///
/// Six covers 5.1. Beyond that the downmix would be guessing at a layout the format does not describe,
/// and guessing silently is worse than refusing.
pub const MAX_CHANNELS: u16 = 6;

/// Limits a caller places on a clip it is about to decode.
///
/// Caller-supplied rather than hardcoded, for the reason every other decoder in this project takes its
/// limits: an editor loading a reference recording can afford minutes of audio, and a multiplayer client
/// accepting a clip out of a downloaded mod cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipLimits {
    /// Largest number of frames the decoded clip may contain.
    pub max_frames: usize,
    /// Largest channel count accepted, capped at [`MAX_CHANNELS`] regardless.
    pub max_channels: u16,
    /// Largest sample rate accepted.
    pub max_sample_rate: u32,
}

impl ClipLimits {
    /// Limits sized for ordinary game content: about three minutes of 48 kHz stereo.
    pub const DEFAULT: Self = Self {
        max_frames: 48_000 * 200,
        max_channels: MAX_CHANNELS,
        max_sample_rate: 192_000,
    };
}

impl Default for ClipLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// A failure constructing a clip from samples that were already decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipError {
    /// A clip declared zero channels, or more than the limit allows.
    ChannelCount {
        /// Channel count the input declared.
        actual: u16,
        /// Largest count accepted.
        maximum: u16,
    },
    /// A clip declared a sample rate of zero, or one beyond the limit.
    SampleRate {
        /// Rate the input declared.
        actual: u32,
        /// Largest rate accepted.
        maximum: u32,
    },
    /// The sample count was not a whole number of frames.
    RaggedFrame {
        /// Number of samples supplied.
        samples: usize,
        /// Channels each frame holds.
        channels: u16,
    },
    /// The clip held more frames than the limit allows.
    TooManyFrames {
        /// Frames the input held.
        actual: usize,
        /// Largest count accepted.
        maximum: usize,
    },
}

impl Display for ClipError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::ChannelCount { actual, maximum } => write!(
                formatter,
                "channel count {actual} is not between 1 and the limit {maximum}"
            ),
            Self::SampleRate { actual, maximum } => write!(
                formatter,
                "sample rate {actual} is not between 1 and the limit {maximum}"
            ),
            Self::RaggedFrame { samples, channels } => write!(
                formatter,
                "{samples} samples do not divide into whole frames of {channels} channels"
            ),
            Self::TooManyFrames { actual, maximum } => write!(
                formatter,
                "clip of {actual} frames exceeds the configured limit {maximum}"
            ),
        }
    }
}

impl Error for ClipError {}

/// Decoded PCM: interleaved samples in `[-1, 1]`, a channel count, and a sample rate.
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    sample_rate: u32,
    channels: u16,
    /// Interleaved: frame `f` channel `c` is at `f * channels + c`.
    samples: Vec<f32>,
}

impl Clip {
    /// Builds a clip from interleaved samples, checking it against `limits`.
    ///
    /// # Errors
    ///
    /// Returns [`ClipError`] when the channel count, sample rate, or frame count is outside the
    /// limits, or when the sample count is not a whole number of frames.
    pub fn new(
        sample_rate: u32,
        channels: u16,
        samples: Vec<f32>,
        limits: ClipLimits,
    ) -> Result<Self, ClipError> {
        let maximum_channels = limits.max_channels.min(MAX_CHANNELS);
        if channels == 0 || channels > maximum_channels {
            return Err(ClipError::ChannelCount {
                actual: channels,
                maximum: maximum_channels,
            });
        }
        if sample_rate == 0 || sample_rate > limits.max_sample_rate {
            return Err(ClipError::SampleRate {
                actual: sample_rate,
                maximum: limits.max_sample_rate,
            });
        }

        let channels_usize = usize::from(channels);
        if !samples.len().is_multiple_of(channels_usize) {
            return Err(ClipError::RaggedFrame {
                samples: samples.len(),
                channels,
            });
        }

        let frames = samples.len() / channels_usize;
        if frames > limits.max_frames {
            return Err(ClipError::TooManyFrames {
                actual: frames,
                maximum: limits.max_frames,
            });
        }

        Ok(Self {
            sample_rate,
            channels,
            samples,
        })
    }

    /// Returns the rate the clip was captured at.
    #[must_use]
    pub const fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Returns how many channels each frame holds.
    #[must_use]
    pub const fn channels(&self) -> u16 {
        self.channels
    }

    /// Returns how many frames the clip holds.
    #[must_use]
    pub fn frames(&self) -> usize {
        self.samples.len() / usize::from(self.channels)
    }

    /// Returns whether the clip holds no frames.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Returns the clip's duration in seconds.
    #[must_use]
    #[expect(
        clippy::cast_precision_loss,
        reason = "a duration in seconds is displayed and compared against tolerances, never used to \
                  index; the frame count would have to exceed 2^24 frames -- about six minutes -- \
                  before the loss reaches one frame, which is below the precision anything reads it at"
    )]
    pub fn duration_seconds(&self) -> f32 {
        self.frames() as f32 / self.sample_rate as f32
    }

    /// Returns the raw interleaved samples.
    #[must_use]
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    /// Reads frame `index`, folded to stereo.
    ///
    /// Out of range reads return silence rather than panicking, because the caller reading frames is a
    /// mixer running under a deadline and a clip that ended is the ordinary case rather than an error.
    #[must_use]
    pub fn stereo_frame(&self, index: usize) -> [f32; 2] {
        let channels = usize::from(self.channels);
        // Checked rather than plain arithmetic, because "returns silence rather than panicking" has
        // to hold for *every* index and not merely for the ones a caller is likely to pass. A debug
        // build multiplying `usize::MAX` by the channel count aborts before the slice bound is ever
        // consulted, which is how this read as safe while not being.
        let Some(start) = index.checked_mul(channels) else {
            return [0.0, 0.0];
        };
        let Some(end) = start.checked_add(channels) else {
            return [0.0, 0.0];
        };
        let Some(frame) = self.samples.get(start..end) else {
            return [0.0, 0.0];
        };
        fold_to_stereo(frame)
    }

    /// Reads a stereo frame at a fractional position, interpolating linearly between neighbours.
    ///
    /// Linear interpolation rather than a windowed sinc, and the reason is where this is used: a voice
    /// resamples because it was pitched or because its clip was recorded at another rate, and both are
    /// small ratios on short sounds. A sinc resampler belongs at an output stage converting a whole mix
    /// once, not per voice per frame.
    #[must_use]
    pub fn stereo_frame_at(&self, position: f64) -> [f32; 2] {
        if position < 0.0 {
            return [0.0, 0.0];
        }
        let index = position.trunc();
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "guarded immediately below: a position past the clip returns silence, and \
                      `index` is non-negative because `position` is"
        )]
        let whole = index as usize;
        if whole >= self.frames() {
            return [0.0, 0.0];
        }

        #[expect(
            clippy::cast_possible_truncation,
            reason = "the fraction is in [0, 1) by construction, so `f32` represents it to well \
                      beyond the resolution a sample interpolation resolves"
        )]
        let fraction = (position - index) as f32;
        let current = self.stereo_frame(whole);
        let next = self.stereo_frame(whole + 1);
        [
            fraction.mul_add(next[0] - current[0], current[0]),
            fraction.mul_add(next[1] - current[1], current[1]),
        ]
    }
}

/// Folds one frame of any supported channel count down to stereo.
///
/// The centre channel of a 5.1 frame is spread at -3 dB rather than at unity, because a centre summed
/// at full amplitude into both sides is 6 dB louder than it was and dialogue is what lives there.
fn fold_to_stereo(frame: &[f32]) -> [f32; 2] {
    /// -3 dB, which is the power-preserving share of one source sent to two outputs.
    const HALF_POWER: f32 = std::f32::consts::FRAC_1_SQRT_2;

    match frame {
        [] => [0.0, 0.0],
        [mono] => [*mono, *mono],
        [left, right] => [*left, *right],
        // Three and four channels are read as stereo plus extras, which is what a quadraphonic or
        // LCR file means, rather than as a surround layout it does not declare.
        [left, right, rest @ ..] => {
            let mut folded = [*left, *right];
            for (index, sample) in rest.iter().enumerate() {
                // The first extra channel of a 5.1 stream is centre and belongs in both; anything
                // after it is a surround or an effects channel and is folded at the same share.
                let _ = index;
                folded[0] += sample * HALF_POWER;
                folded[1] += sample * HALF_POWER;
            }
            folded
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::{Clip, ClipError, ClipLimits, MAX_CHANNELS};

    fn limits() -> ClipLimits {
        ClipLimits::DEFAULT
    }

    #[test]
    fn a_mono_clip_reads_as_the_same_sample_in_both_ears() {
        let clip = Clip::new(48_000, 1, vec![0.25, -0.5], limits()).expect("build");
        assert_eq!(clip.frames(), 2);
        assert_eq!(clip.stereo_frame(0), [0.25, 0.25]);
        assert_eq!(clip.stereo_frame(1), [-0.5, -0.5]);
    }

    #[test]
    fn reading_past_the_end_returns_silence_rather_than_panicking() {
        // A mixer reads frames under a deadline and a clip that ended is the ordinary case. This is
        // the property that lets the voice loop stay branch-free at the edges.
        let clip = Clip::new(48_000, 2, vec![1.0, 1.0], limits()).expect("build");
        assert_eq!(clip.stereo_frame(1), [0.0, 0.0]);
        assert_eq!(clip.stereo_frame(usize::MAX), [0.0, 0.0]);
        assert_eq!(clip.stereo_frame_at(-1.0), [0.0, 0.0]);
        assert_eq!(clip.stereo_frame_at(9.5), [0.0, 0.0]);
    }

    #[test]
    fn a_fractional_position_interpolates_between_neighbours() {
        let clip = Clip::new(48_000, 1, vec![0.0, 1.0], limits()).expect("build");
        assert!((clip.stereo_frame_at(0.5)[0] - 0.5).abs() < 1e-6);
        assert!((clip.stereo_frame_at(0.25)[0] - 0.25).abs() < 1e-6);
    }

    #[test]
    fn a_five_point_one_centre_channel_is_folded_at_half_power() {
        // Summing centre into both sides at unity makes dialogue 6 dB louder than it was recorded.
        let mut samples = vec![0.0; 6];
        samples[2] = 1.0;
        let clip = Clip::new(48_000, 6, samples, limits()).expect("build");
        let frame = clip.stereo_frame(0);
        assert!((frame[0] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert_eq!(frame[0], frame[1]);
    }

    #[test]
    fn a_ragged_sample_count_is_refused() {
        let error = Clip::new(48_000, 2, vec![1.0, 1.0, 1.0], limits()).expect_err("refuse");
        assert_eq!(
            error,
            ClipError::RaggedFrame {
                samples: 3,
                channels: 2
            }
        );
    }

    #[test]
    fn each_limit_is_checked() {
        assert!(matches!(
            Clip::new(48_000, 0, vec![], limits()),
            Err(ClipError::ChannelCount { .. })
        ));
        assert!(matches!(
            Clip::new(48_000, MAX_CHANNELS + 1, vec![], limits()),
            Err(ClipError::ChannelCount { .. })
        ));
        assert!(matches!(
            Clip::new(0, 1, vec![], limits()),
            Err(ClipError::SampleRate { .. })
        ));
        assert!(matches!(
            Clip::new(400_000, 1, vec![], limits()),
            Err(ClipError::SampleRate { .. })
        ));

        let strict = ClipLimits {
            max_frames: 1,
            ..limits()
        };
        assert!(matches!(
            Clip::new(48_000, 1, vec![0.0, 0.0], strict),
            Err(ClipError::TooManyFrames {
                actual: 2,
                maximum: 1
            })
        ));
    }

    #[test]
    fn the_channel_limit_cannot_be_raised_past_the_supported_maximum() {
        // A caller passing a generous limit must not be able to talk the decoder into a layout the
        // downmix has no rule for.
        let permissive = ClipLimits {
            max_channels: 64,
            ..limits()
        };
        let error = Clip::new(48_000, 8, vec![0.0; 8], permissive).expect_err("refuse");
        assert_eq!(
            error,
            ClipError::ChannelCount {
                actual: 8,
                maximum: MAX_CHANNELS
            }
        );
    }
}
