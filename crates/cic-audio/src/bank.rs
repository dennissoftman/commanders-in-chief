//! The authored layer: what a sound *event* is, as distinct from what a clip is.
//!
//! # Why a cue is not a file name
//!
//! Because everything that makes game audio not sound like a slideshow lives between "the rifle fired"
//! and "play `rifle.wav`", and none of it belongs in the code that fires the rifle.
//!
//! - **Variants.** One clip played twenty times is not twenty rifle shots, it is a machine gun with a
//!   very obvious loop. Several recordings chosen between is the whole difference.
//! - **Pitch spread.** Even across variants, exact repetition is audible. A few percent either way is
//!   inaudible individually and removes the mechanical quality entirely.
//! - **Polyphony and cooldown.** Forty units firing on one tick is forty voices starting on the same
//!   sample, which sums to something forty times louder than one and correlated with itself — so it
//!   reads as a single loud crack rather than as a volley, and it is what actually drives a mixer into
//!   its limiter. A cue that admits four instances and refuses another within 45 milliseconds sounds
//!   like more units rather than fewer.
//!
//! Every one of those is a property of the *event*, so they live in a bank file a sound designer edits,
//! and the gameplay code says `bank.play("unit.rifle.fire", at)`.
//!
//! # Randomness here never touches a simulation stream
//!
//! [The determinism invariants](../../../docs/invariants/determinism.md) say a seeded stream must never
//! be consumed by presentation, because drawing from one is part of the simulation's state transition
//! and a machine that draws an extra number has desynced. Audio is presentation and it needs randomness
//! constantly, so it carries its **own** stream, seeded independently. That is the rule this module
//! exists on the right side of: nothing here can be reached from a simulation tick, and no simulation
//! stream is reachable from here.

use serde::{Deserialize, Serialize};

use crate::bus::BusId;
use crate::spatial::{Attenuation, Cone, Emitter};
use crate::voice::{DEFAULT_PRIORITY, Priority, SoundId, VoiceSpec};

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

/// The bank format version this build understands.
pub const FORMAT_VERSION: u32 = 1;

/// Largest number of cues one bank may declare.
const MAX_CUES: usize = 8_192;

/// Largest number of variants one cue may declare.
const MAX_VARIANTS: usize = 64;

/// A failure loading a bank.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BankError {
    /// The file was not valid JSON, or held a field the format does not define.
    Syntax {
        /// What the decoder reported.
        detail: String,
    },
    /// The file declared a version this build does not implement.
    Version {
        /// Version the file declared.
        found: u32,
        /// Version this build implements.
        expected: u32,
    },
    /// A limit was crossed.
    Limit {
        /// Name of the limited quantity.
        what: &'static str,
        /// Value the file declared.
        actual: usize,
        /// Largest value accepted.
        maximum: usize,
    },
    /// A cue was structurally invalid.
    Cue {
        /// The cue's name.
        name: String,
        /// What was wrong with it.
        detail: &'static str,
    },
}

impl Display for BankError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax { detail } => write!(formatter, "malformed sound bank: {detail}"),
            Self::Version { found, expected } => write!(
                formatter,
                "sound bank format version {found} is not the {expected} this build implements"
            ),
            Self::Limit {
                what,
                actual,
                maximum,
            } => write!(
                formatter,
                "{what} value {actual} exceeds the configured limit {maximum}"
            ),
            Self::Cue { name, detail } => write!(formatter, "cue `{name}`: {detail}"),
        }
    }
}

impl Error for BankError {}

/// One recording a cue may choose between.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Variant {
    /// Virtual path of the clip, resolved through the resource layer by the caller.
    pub clip: String,
    /// How often this variant is chosen relative to its siblings.
    #[serde(default = "default_weight")]
    pub weight: u32,
    /// Gain applied to this variant alone, for levelling one recording against another.
    #[serde(default)]
    pub gain_db: f32,
}

const fn default_weight() -> u32 {
    1
}

const fn default_polyphony() -> u32 {
    8
}

const fn default_pitch_range() -> [f32; 2] {
    [1.0, 1.0]
}

/// A named sound event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cue {
    /// Recordings this cue chooses between.
    pub variants: Vec<Variant>,
    /// Where the cue is routed.
    #[serde(default)]
    pub bus: BusId,
    /// Gain applied to every variant.
    #[serde(default)]
    pub gain_db: f32,
    /// Inclusive range the playback rate is drawn from.
    #[serde(default = "default_pitch_range")]
    pub pitch_range: [f32; 2],
    /// The distance curve, or `None` for a cue with no position.
    #[serde(default)]
    pub attenuation: Option<Attenuation>,
    /// Directionality, if any.
    #[serde(default)]
    pub cone: Option<Cone>,
    /// How wide the source is, from a point at `0.0` to everywhere at `1.0`.
    #[serde(default)]
    pub spread: f32,
    /// Doppler strength.
    #[serde(default)]
    pub doppler: f32,
    /// How many instances of this cue may sound at once.
    #[serde(default = "default_polyphony")]
    pub polyphony: u32,
    /// How long after one instance starts before another may, in milliseconds.
    #[serde(default)]
    pub cooldown_ms: f32,
    /// What an instance is worth when the voice budget is full.
    #[serde(default = "default_priority")]
    pub priority: Priority,
    /// Whether an instance repeats until stopped.
    #[serde(default)]
    pub looping: bool,
    /// Seconds to fade an instance in.
    #[serde(default)]
    pub fade_in_seconds: f32,
}

const fn default_priority() -> Priority {
    DEFAULT_PRIORITY
}

impl Cue {
    /// Checks the cue is usable, naming what is wrong rather than only that something is.
    fn validate(&self, name: &str) -> Result<(), BankError> {
        let fault = |detail| BankError::Cue {
            name: name.to_owned(),
            detail,
        };
        if self.variants.is_empty() {
            return Err(fault("no variants, so it can never make a sound"));
        }
        if self.variants.len() > MAX_VARIANTS {
            return Err(BankError::Limit {
                what: "variants per cue",
                actual: self.variants.len(),
                maximum: MAX_VARIANTS,
            });
        }
        if self.variants.iter().any(|variant| variant.clip.is_empty()) {
            return Err(fault("a variant names no clip"));
        }
        if self.variants.iter().all(|variant| variant.weight == 0) {
            // Every weight zero makes the selection a division by zero. One zero among several is
            // legitimate -- it is how a designer disables a variant without deleting it.
            return Err(fault(
                "every variant has weight zero, so none can be chosen",
            ));
        }
        if self.polyphony == 0 {
            return Err(fault("polyphony of zero, so it can never make a sound"));
        }
        let [low, high] = self.pitch_range;
        if !(low.is_finite() && high.is_finite()) || low <= 0.0 || high < low {
            return Err(fault("pitch range is not a positive ascending interval"));
        }
        Ok(())
    }

    /// Builds the emitter this cue asks for at `position`, or `None` for a cue with no position.
    #[must_use]
    pub fn emitter_at(&self, position: [f32; 3]) -> Option<Emitter> {
        let attenuation = self.attenuation?;
        Some(Emitter {
            position,
            attenuation,
            cone: self.cone,
            spread: self.spread,
            doppler: self.doppler,
            ..Emitter::default()
        })
    }
}

/// A whole bank, as loaded from JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoundBank {
    /// Version of the bank format.
    pub format_version: u32,
    /// Cues by name. Ordered, so iteration is stable.
    pub cues: BTreeMap<String, Cue>,
}

impl SoundBank {
    /// Decodes and validates a bank.
    ///
    /// # Errors
    ///
    /// Returns [`BankError`] for malformed JSON, an unknown field, a wrong version, a limit being
    /// crossed, or a cue that could never make a sound.
    pub fn from_json(bytes: &[u8]) -> Result<Self, BankError> {
        let bank: Self = serde_json::from_slice(bytes).map_err(|error| BankError::Syntax {
            detail: error.to_string(),
        })?;

        // A version bump means existing fields may have changed meaning, which is not something to
        // guess at -- the same rule the binary formats follow.
        if bank.format_version != FORMAT_VERSION {
            return Err(BankError::Version {
                found: bank.format_version,
                expected: FORMAT_VERSION,
            });
        }
        if bank.cues.len() > MAX_CUES {
            return Err(BankError::Limit {
                what: "cues per bank",
                actual: bank.cues.len(),
                maximum: MAX_CUES,
            });
        }
        for (name, cue) in &bank.cues {
            cue.validate(name)?;
        }
        Ok(bank)
    }

    /// Returns a cue by name.
    #[must_use]
    pub fn cue(&self, name: &str) -> Option<&Cue> {
        self.cues.get(name)
    }

    /// Every distinct clip path the bank refers to, sorted.
    ///
    /// This is what a host loads. Sorted and deduplicated so the same bank produces the same load
    /// order on every machine, which keeps a diagnostic listing comparable between two of them.
    #[must_use]
    pub fn clip_paths(&self) -> Vec<&str> {
        let mut paths: Vec<&str> = self
            .cues
            .values()
            .flat_map(|cue| cue.variants.iter())
            .map(|variant| variant.clip.as_str())
            .collect();
        paths.sort_unstable();
        paths.dedup();
        paths
    }
}

/// The audio system's own random stream.
///
/// This is the `SplitMix64` finaliser, a published algorithm whose constants are documented rather than
/// invented. Named here rather than hidden because a reader is entitled to know whether a magic number
/// was reasoned about; the alternative — writing an ad-hoc generator so every constant is "ours" — is
/// worse engineering for no licensing benefit, since an algorithm is not a copyrightable work.
///
/// Its role is what matters more than its quality: it exists so audio never has to draw from a
/// simulation stream. See the module documentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioRandom {
    state: u64,
}

impl AudioRandom {
    /// Seeds the stream.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Returns the next value.
    pub const fn next_u64(&mut self) -> u64 {
        // The increment is the 64-bit golden-ratio constant: odd, so the sequence has full period, and
        // with well-distributed bits, so successive states differ everywhere rather than only low down.
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        value ^ (value >> 31)
    }

    /// Returns a value in `[0, 1)`.
    pub fn next_unit(&mut self) -> f32 {
        // The top 24 bits, which is exactly an `f32` mantissa: taking the low bits instead is the
        // classic mistake, since the low bits of many generators are the weakest.
        #[expect(
            clippy::cast_precision_loss,
            reason = "24 bits is exactly the `f32` mantissa, so this conversion is lossless"
        )]
        let scaled = (self.next_u64() >> 40) as f32;
        scaled / 16_777_216.0
    }

    /// Returns a value in `[low, high]`.
    pub fn next_range(&mut self, low: f32, high: f32) -> f32 {
        let unit = self.next_unit();
        unit.mul_add(high - low, low)
    }

    /// Chooses an index in proportion to `weights`, skipping zero-weight entries.
    ///
    /// Returns `None` only when every weight is zero, which a bank fails to load for — so a cue that
    /// came out of [`SoundBank::from_json`] cannot reach it.
    pub fn weighted_index(&mut self, weights: &[u32]) -> Option<usize> {
        let total: u64 = weights.iter().map(|&weight| u64::from(weight)).sum();
        if total == 0 {
            return None;
        }
        let mut target = self.next_u64() % total;
        for (index, &weight) in weights.iter().enumerate() {
            let weight = u64::from(weight);
            if target < weight {
                return Some(index);
            }
            target -= weight;
        }
        // Unreachable while the sum is consistent with the loop, and returning the last non-zero
        // entry is better than an index the caller would use to slice.
        weights.iter().rposition(|&weight| weight > 0)
    }
}

/// What a cue remembers between triggers: how many instances are live, when the last one started, and
/// which variant it was.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct CueState {
    pub(crate) live: u32,
    pub(crate) cooldown_remaining_ms: f32,
    pub(crate) last_variant: Option<usize>,
}

/// Chooses a variant, avoiding an immediate repeat where the cue has an alternative.
///
/// The avoidance is the point. With three variants chosen uniformly, the same one comes up twice in a
/// row about a third of the time — and a repeat is the single most audible thing in a sound set,
/// because it is the one case where a listener has an exact reference to compare against.
pub(crate) fn choose_variant(
    cue: &Cue,
    state: &CueState,
    random: &mut AudioRandom,
) -> Option<usize> {
    let mut weights: Vec<u32> = cue.variants.iter().map(|variant| variant.weight).collect();
    if let Some(last) = state.last_variant {
        let alternatives = weights
            .iter()
            .enumerate()
            .filter(|&(index, &weight)| weight > 0 && index != last)
            .count();
        // Only suppress the repeat when something else could actually be chosen. A one-variant cue,
        // or one where every other variant is disabled, must still play.
        if alternatives > 0
            && let Some(weight) = weights.get_mut(last)
        {
            *weight = 0;
        }
    }
    random.weighted_index(&weights)
}

/// Turns a cue and a chosen variant into a voice request.
pub(crate) fn spec_for(
    cue: &Cue,
    variant: usize,
    sound: SoundId,
    position: Option<[f32; 3]>,
    random: &mut AudioRandom,
) -> VoiceSpec {
    let variant_gain = cue
        .variants
        .get(variant)
        .map_or(0.0, |variant| variant.gain_db);
    let [low, high] = cue.pitch_range;
    let pitch = if (high - low).abs() <= f32::EPSILON {
        low
    } else {
        random.next_range(low, high)
    };

    let mut spec = VoiceSpec::new(sound, cue.bus)
        .with_gain_db(cue.gain_db + variant_gain)
        .with_pitch(pitch)
        .with_priority(cue.priority)
        .faded_in(cue.fade_in_seconds);
    spec.looping = cue.looping;
    spec.emitter = position.and_then(|position| cue.emitter_at(position));
    spec
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::{
        AudioRandom, BankError, CueState, FORMAT_VERSION, SoundBank, choose_variant, spec_for,
    };
    use crate::bus::BusId;
    use crate::voice::SoundId;

    const SAMPLE: &str = r#"{
      "format_version": 1,
      "cues": {
        "unit.rifle.fire": {
          "bus": "effects",
          "variants": [
            { "clip": "audio/rifle_a.wav", "weight": 2 },
            { "clip": "audio/rifle_b.wav", "gain_db": -1.5 }
          ],
          "gain_db": -3.0,
          "pitch_range": [0.94, 1.06],
          "attenuation": { "kind": "inverse", "reference": 8.0, "far": 400.0, "rolloff": 1.0 },
          "polyphony": 4,
          "cooldown_ms": 45.0,
          "priority": 140
        },
        "ui.button.press": {
          "bus": "interface",
          "variants": [{ "clip": "audio/click.wav" }]
        }
      }
    }"#;

    fn bank() -> SoundBank {
        SoundBank::from_json(SAMPLE.as_bytes()).expect("load")
    }

    #[test]
    fn a_bank_loads_and_its_defaults_are_the_documented_ones() {
        let bank = bank();
        let rifle = bank.cue("unit.rifle.fire").expect("present");
        assert_eq!(rifle.bus, BusId::Effects);
        assert_eq!(rifle.polyphony, 4);
        assert_eq!(rifle.priority, 140);

        let click = bank.cue("ui.button.press").expect("present");
        assert_eq!(click.bus, BusId::Interface);
        assert_eq!(click.pitch_range, [1.0, 1.0]);
        assert_eq!(click.polyphony, 8);
        assert_eq!(click.priority, super::DEFAULT_PRIORITY);
        assert!(
            click.attenuation.is_none(),
            "an interface sound has no position"
        );
    }

    #[test]
    fn clip_paths_are_sorted_and_deduplicated() {
        // A host loads what this returns, so the order has to be the same on every machine for two
        // diagnostic listings to be comparable.
        let bank = bank();
        let paths = bank.clip_paths();
        assert_eq!(
            paths,
            vec!["audio/click.wav", "audio/rifle_a.wav", "audio/rifle_b.wav"]
        );
    }

    #[test]
    fn a_wrong_version_is_refused_rather_than_read_anyway() {
        let text = SAMPLE.replace("\"format_version\": 1", "\"format_version\": 2");
        assert_eq!(
            SoundBank::from_json(text.as_bytes()),
            Err(BankError::Version {
                found: 2,
                expected: FORMAT_VERSION
            })
        );
    }

    #[test]
    fn an_unknown_field_is_refused() {
        // The same rule the layout format follows: a typo must fail at load, naming the file, rather
        // than silently doing nothing at the moment somebody triggers the sound.
        let text = SAMPLE.replace("\"gain_db\": -3.0", "\"gian_db\": -3.0");
        assert!(matches!(
            SoundBank::from_json(text.as_bytes()),
            Err(BankError::Syntax { .. })
        ));
    }

    #[test]
    fn a_bus_the_engine_does_not_define_is_refused() {
        let text = SAMPLE.replace("\"bus\": \"effects\"", "\"bus\": \"secret\"");
        assert!(matches!(
            SoundBank::from_json(text.as_bytes()),
            Err(BankError::Syntax { .. })
        ));
    }

    #[test]
    fn a_cue_that_could_never_make_a_sound_is_refused_at_load() {
        let cases = [
            (r#""variants": []"#, "no variants"),
            (r#""variants": [{"clip": ""}]"#, "an empty path"),
            (
                r#""variants": [{"clip": "a.wav", "weight": 0}]"#,
                "every weight zero",
            ),
        ];
        for (variants, what) in cases {
            let text = format!(r#"{{"format_version":1,"cues":{{"broken":{{{variants}}}}}}}"#);
            assert!(
                matches!(
                    SoundBank::from_json(text.as_bytes()),
                    Err(BankError::Cue { .. })
                ),
                "{what} was accepted"
            );
        }
    }

    #[test]
    fn a_polyphony_of_zero_and_a_reversed_pitch_range_are_refused() {
        for broken in [
            r#""variants":[{"clip":"a.wav"}],"polyphony":0"#,
            r#""variants":[{"clip":"a.wav"}],"pitch_range":[1.4,0.9]"#,
            r#""variants":[{"clip":"a.wav"}],"pitch_range":[0.0,1.0]"#,
        ] {
            let text = format!(r#"{{"format_version":1,"cues":{{"broken":{{{broken}}}}}}}"#);
            assert!(
                matches!(
                    SoundBank::from_json(text.as_bytes()),
                    Err(BankError::Cue { .. })
                ),
                "accepted: {broken}"
            );
        }
    }

    #[test]
    fn a_weighted_choice_follows_its_weights() {
        let mut random = AudioRandom::new(1);
        let mut counts = [0u32; 3];
        for _ in 0..30_000 {
            let index = random
                .weighted_index(&[1, 3, 0])
                .expect("a weight is non-zero");
            counts[index] += 1;
        }
        assert_eq!(counts[2], 0, "a zero weight was chosen");
        let ratio = f64::from(counts[1]) / f64::from(counts[0]);
        assert!(
            (ratio - 3.0).abs() < 0.15,
            "the 3:1 weighting produced {ratio}"
        );
    }

    #[test]
    fn every_weight_zero_chooses_nothing_rather_than_dividing_by_zero() {
        let mut random = AudioRandom::new(1);
        assert_eq!(random.weighted_index(&[0, 0]), None);
        assert_eq!(random.weighted_index(&[]), None);
    }

    #[test]
    fn a_variant_is_not_repeated_immediately_when_there_is_an_alternative() {
        // The most audible thing in a sound set, because a repeat is the one case where a listener
        // has an exact reference to compare against.
        let bank = bank();
        let cue = bank.cue("unit.rifle.fire").expect("present");
        let mut random = AudioRandom::new(7);
        let mut state = CueState::default();

        for _ in 0..200 {
            let chosen = choose_variant(cue, &state, &mut random).expect("a variant");
            assert_ne!(
                Some(chosen),
                state.last_variant,
                "the same variant played twice in a row"
            );
            state.last_variant = Some(chosen);
        }
    }

    #[test]
    fn a_single_variant_cue_still_plays_despite_the_repeat_rule() {
        // Suppressing the repeat unconditionally would silence every cue that has only one recording,
        // which is most of them.
        let bank = bank();
        let cue = bank.cue("ui.button.press").expect("present");
        let mut random = AudioRandom::new(3);
        let state = CueState {
            last_variant: Some(0),
            ..CueState::default()
        };
        assert_eq!(choose_variant(cue, &state, &mut random), Some(0));
    }

    #[test]
    fn a_pitch_range_is_drawn_from_and_a_fixed_pitch_is_not() {
        let bank = bank();
        let mut random = AudioRandom::new(11);
        let rifle = bank.cue("unit.rifle.fire").expect("present");

        let mut seen = Vec::new();
        for _ in 0..32 {
            let spec = spec_for(rifle, 0, SoundId::new(0, 0), None, &mut random);
            assert!(
                (0.94..=1.06).contains(&spec.pitch),
                "{} out of range",
                spec.pitch
            );
            seen.push(spec.pitch);
        }
        assert!(
            seen.windows(2).any(|pair| pair[0] != pair[1]),
            "every draw returned the same pitch"
        );

        let click = bank.cue("ui.button.press").expect("present");
        let spec = spec_for(click, 0, SoundId::new(0, 0), None, &mut random);
        assert_eq!(spec.pitch, 1.0, "a fixed range must not be randomised");
    }

    #[test]
    fn a_variants_own_gain_adds_to_the_cues() {
        let bank = bank();
        let mut random = AudioRandom::new(5);
        let rifle = bank.cue("unit.rifle.fire").expect("present");
        let quiet = spec_for(rifle, 1, SoundId::new(0, 0), None, &mut random);
        assert_eq!(
            quiet.gain_db, -4.5,
            "-3.0 from the cue and -1.5 from the variant"
        );
    }

    #[test]
    fn a_positioned_trigger_gets_an_emitter_and_an_unpositioned_cue_never_does() {
        let bank = bank();
        let mut random = AudioRandom::new(5);
        let rifle = bank.cue("unit.rifle.fire").expect("present");
        let placed = spec_for(
            rifle,
            0,
            SoundId::new(0, 0),
            Some([1.0, 2.0, 3.0]),
            &mut random,
        );
        assert_eq!(placed.emitter.expect("placed").position, [1.0, 2.0, 3.0]);

        // An interface cue declares no attenuation, so triggering it at a position must still give a
        // sound that plays at full level in both ears rather than one that fades with the camera.
        let click = bank.cue("ui.button.press").expect("present");
        let at_a_point = spec_for(
            click,
            0,
            SoundId::new(0, 0),
            Some([9.0, 9.0, 9.0]),
            &mut random,
        );
        assert!(at_a_point.emitter.is_none());
    }

    #[test]
    fn the_same_seed_produces_the_same_sequence() {
        // Not for lockstep -- audio is presentation and must never be in a state hash. For being able
        // to reproduce a report that one cue sounds wrong.
        let mut first = AudioRandom::new(42);
        let mut second = AudioRandom::new(42);
        for _ in 0..64 {
            assert_eq!(first.next_u64(), second.next_u64());
        }
    }

    #[test]
    fn unit_draws_stay_inside_the_half_open_interval() {
        let mut random = AudioRandom::new(9);
        for _ in 0..10_000 {
            let value = random.next_unit();
            assert!((0.0..1.0).contains(&value), "{value} is outside [0, 1)");
        }
    }
}
