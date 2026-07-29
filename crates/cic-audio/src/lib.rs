//! Audio: cues, buses, spatialisation, DSP, and a replaceable mixing backend.
//!
//! # What is here
//!
//! - [`backend`] — the trait a replaceable implementation sits behind, and the conformance properties
//!   any implementation must satisfy.
//! - [`mixer`] — the in-tree implementation of it, written from scratch and depending on nothing.
//! - [`engine`] — the frontend that does not change when the backend does: cues, budgets, the mix.
//! - [`bank`] — the authored format: what a sound *event* is, as distinct from what a file is.
//! - [`bus`] — routing, the player's volume settings, and snapshots.
//! - [`spatial`] — listener, emitters, distance curves, panning, Doppler, occlusion.
//! - [`dsp`] — filters, reverb, and dynamics, and the descriptions a bank holds them as.
//! - [`music`] — layered score playback and transitions.
//! - [`sample`] — decoded PCM.
//! - [`wav`] — a bounded RIFF/WAVE decoder.
//!
//! # The backend is switchable, and the boundary is the decision
//!
//! Recorded in [ADR 6001](../../../docs/adr/6001-audio-backend-boundary.md). The boundary is a
//! *command* interface — play this clip, on this bus, at this priority, at this position — and not a
//! stream of finished samples, because FMOD and OpenAL are complete mixers rather than devices and a
//! sample sink would throw away everything anyone adopts either of them for.
//!
//! What that buys is that engine policy is written once. What it costs is that two backends do not
//! produce identical output, so the properties asserted of *any* backend are behavioural
//! ([`backend::conformance`]) and the numeric tests target [`mixer::SoftwareMixer`] by name.
//!
//! The in-tree mixer is the default for reasons that are as much licensing as engineering: FMOD is
//! proprietary and licensed per title, and OpenAL Soft is LGPL, which constrains static linking.
//! Neither can be what a permissively licensed engine *requires* in order to make a sound.
//!
//! # The one rule this crate is bound by
//!
//! Audio is presentation, so nothing here is required to be deterministic and floating point is used
//! freely. The rule that does bind it runs the other way: **audio must never draw from a simulation
//! random stream.** Drawing from one is part of that simulation's state transition, so a machine whose
//! audio consumed an extra number has desynced. [`bank::AudioRandom`] exists so it never has to — see
//! [the determinism invariants](../../../docs/invariants/determinism.md).
//!
//! # Nothing here opens a file
//!
//! Clips arrive as bytes and banks arrive as bytes, both from the caller, for the reason every decoder
//! in this project does: it is what makes a loose development file, a package entry, and a mod override
//! interchangeable.
//!
//! # Example
//!
//! ```
//! use cic_audio::{AudioEngine, SoftwareMixer, SoundBank};
//! use cic_audio::sample::{Clip, ClipLimits};
//! use std::sync::Arc;
//!
//! let bank = SoundBank::from_json(
//!     br#"{
//!       "format_version": 1,
//!       "cues": {
//!         "unit.rifle.fire": {
//!           "bus": "effects",
//!           "variants": [{ "clip": "audio/rifle_a.wav" }],
//!           "pitch_range": [0.94, 1.06],
//!           "attenuation": { "kind": "inverse", "reference": 8.0, "far": 400.0, "rolloff": 1.0 }
//!         }
//!       }
//!     }"#,
//! )?;
//!
//! // The host loads whatever the bank says it needs.
//! assert_eq!(bank.clip_paths(), vec!["audio/rifle_a.wav"]);
//!
//! let mut engine = AudioEngine::new(SoftwareMixer::with_defaults(), bank, 0);
//! let clip = Arc::new(Clip::new(48_000, 1, vec![0.0; 480], ClipLimits::DEFAULT)?);
//! engine.bind_clip("audio/rifle_a.wav", clip)?;
//!
//! assert!(engine.missing_clips().is_empty());
//! assert!(engine.play("unit.rifle.fire", Some([12.0, 0.0, -30.0])).is_some());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod backend;
pub mod bank;
pub mod bus;
pub mod dsp;
pub mod engine;
pub mod mixer;
pub mod music;
pub mod sample;
pub mod spatial;
pub mod voice;
pub mod wav;

pub use backend::{Backend, BackendError, Capabilities, RenderToBuffer, VoiceEvent};
pub use bank::{AudioRandom, BankError, Cue, SoundBank, Variant};
pub use bus::{BusId, BusSettings, Mix, Snapshot};
pub use dsp::{Effect, EffectSpec, FilterKind};
pub use engine::{AudioEngine, TriggerFailure};
pub use mixer::SoftwareMixer;
pub use music::{MusicDirector, MusicLayer, MusicPhase, MusicTrack};
pub use sample::{Clip, ClipError, ClipLimits, DEFAULT_SAMPLE_RATE};
pub use spatial::{Attenuation, Cone, Emitter, Listener, Spatialization};
pub use voice::{Priority, SoundId, VoiceId, VoiceParams, VoiceSpec};
pub use wav::WavError;
