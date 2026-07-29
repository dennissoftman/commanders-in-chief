//! The mixer's routing: which bus a sound goes to, what each bus does to it, and how a whole mix is
//! moved at once.
//!
//! # Why the bus set is a closed enum
//!
//! For the same reason [the interface layer's action set](../../cic-ui/src/action.rs) is one: a bank
//! file is data, and data must not be able to name a destination the engine did not define. An
//! arbitrary string would defer a typo from load time to the moment somebody triggers the sound, and
//! would make the settings screen's list of volume sliders depend on what content happened to be
//! installed.
//!
//! It is also the shape the *player* needs. A volume control panel is a fixed list — music, effects,
//! speech — and every one of them has to exist whether or not any content currently routes to it.
//!
//! # Snapshots, and why ducking is not a special case
//!
//! When a briefing plays, music has to get out of the way. Implementing that as "the speech system
//! turns the music down" puts a rule about the mix inside the thing that triggered it, and the second
//! feature to want it — an in-game pause, a cinematic, a low-health filter — either duplicates the rule
//! or fights the first one over who restores the gain.
//!
//! A [`Snapshot`] is instead a named set of offsets applied on top of the player's settings, blended in
//! and out over a stated time. Several can be active at once and they sum, so a pause during a briefing
//! is the two offsets together rather than a conflict.

use serde::{Deserialize, Serialize};

use crate::dsp::EffectSpec;

use std::collections::BTreeMap;

/// Every destination a sound may be routed to.
///
/// Ordered — and therefore usable as a `BTreeMap` key — because a snapshot's offsets are iterated and
/// the determinism invariants forbid an iteration order that can reach output from being a hash order.
/// That rule binds the simulation rather than this crate, but a mix that differs run to run is a
/// support burden even where it is not a desync.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum BusId {
    /// Everything ends up here. Carries the limiter that keeps the sum inside full scale.
    #[default]
    Master,
    /// Score and ambient beds.
    Music,
    /// The world: weapons, engines, impacts, construction.
    Effects,
    /// Spoken lines — unit responses, briefings, the advisor.
    Speech,
    /// Buttons, notifications, and anything else that belongs to the shell rather than the world.
    Interface,
    /// Wind, rain, and the rest of the weather bed.
    Ambience,
}

impl BusId {
    /// Every bus, in a fixed order, master last.
    ///
    /// Master last because the mixer sums the others into it, so processing them in this order needs
    /// no second pass.
    pub const ALL: [Self; 6] = [
        Self::Music,
        Self::Effects,
        Self::Speech,
        Self::Interface,
        Self::Ambience,
        Self::Master,
    ];

    /// A stable index for array storage.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Master => 0,
            Self::Music => 1,
            Self::Effects => 2,
            Self::Speech => 3,
            Self::Interface => 4,
            Self::Ambience => 5,
        }
    }

    /// How many buses exist, which is the size of any array indexed by [`Self::index`].
    pub const COUNT: usize = 6;

    /// The name a settings screen shows, behind a string-table key rather than as display text.
    #[must_use]
    pub const fn string_key(self) -> &'static str {
        match self {
            Self::Master => "audio.bus.master",
            Self::Music => "audio.bus.music",
            Self::Effects => "audio.bus.effects",
            Self::Speech => "audio.bus.speech",
            Self::Interface => "audio.bus.interface",
            Self::Ambience => "audio.bus.ambience",
        }
    }
}

/// What a bus does to everything routed through it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BusSettings {
    /// The player's setting for this bus, in decibels. Zero is unity.
    #[serde(default)]
    pub gain_db: f32,
    /// Whether the bus is silenced outright.
    #[serde(default)]
    pub muted: bool,
    /// Effects every sound on this bus passes through, in order.
    #[serde(default)]
    pub effects: Vec<EffectSpec>,
}

impl Default for BusSettings {
    fn default() -> Self {
        Self {
            gain_db: 0.0,
            muted: false,
            effects: Vec::new(),
        }
    }
}

/// A named set of gain offsets applied on top of the player's settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    /// How far each bus moves while this snapshot is fully applied, in decibels. Buses absent from
    /// the map are unaffected.
    pub offsets: BTreeMap<BusId, f32>,
    /// Seconds to blend in.
    #[serde(default = "default_blend_seconds")]
    pub attack_seconds: f32,
    /// Seconds to blend out.
    #[serde(default = "default_blend_seconds")]
    pub release_seconds: f32,
}

const fn default_blend_seconds() -> f32 {
    0.35
}

impl Snapshot {
    /// The one every game needs: music and ambience out of the way of a spoken line.
    ///
    /// Present as a constructor rather than only as data because a project without it ships with
    /// briefings nobody can hear, and discovering that requires content to exist first.
    #[must_use]
    pub fn ducking() -> Self {
        let mut offsets = BTreeMap::new();
        offsets.insert(BusId::Music, -12.0);
        offsets.insert(BusId::Ambience, -6.0);
        // Effects duck too, but far less: silencing the battle to deliver a line makes the line feel
        // like a cutscene rather than like a radio call over a fight that is still happening.
        offsets.insert(BusId::Effects, -3.0);
        Self {
            offsets,
            attack_seconds: 0.2,
            release_seconds: 0.6,
        }
    }
}

/// A snapshot's blend state.
#[derive(Debug, Clone, PartialEq)]
struct Active {
    snapshot: Snapshot,
    /// How far the blend has come, from `0.0` to `1.0`.
    weight: f32,
    /// Whether the blend is heading toward one or toward zero.
    engaged: bool,
}

/// The resolved mix: the player's settings, plus whatever snapshots are currently blended in.
#[derive(Debug, Clone, PartialEq)]
pub struct Mix {
    settings: [BusSettings; BusId::COUNT],
    active: BTreeMap<String, Active>,
}

impl Default for Mix {
    fn default() -> Self {
        Self::new()
    }
}

impl Mix {
    /// A mix with every bus at unity and nothing engaged.
    #[must_use]
    pub fn new() -> Self {
        Self {
            settings: std::array::from_fn(|_| BusSettings::default()),
            active: BTreeMap::new(),
        }
    }

    /// Returns the player's settings for a bus.
    #[must_use]
    pub fn settings(&self, bus: BusId) -> &BusSettings {
        &self.settings[bus.index()]
    }

    /// Replaces the player's settings for a bus.
    ///
    /// The gain is clamped to a range a settings slider can actually reach. Above zero is deliberately
    /// permitted up to a point, because a player on quiet speakers wanting speech louder than the mix
    /// was authored is a real request and the limiter is what protects the output.
    pub fn set_settings(&mut self, bus: BusId, settings: BusSettings) {
        let mut settings = settings;
        settings.gain_db = settings.gain_db.clamp(-80.0, 12.0);
        self.settings[bus.index()] = settings;
    }

    /// Sets a bus's gain, leaving its mute state and effects alone.
    pub fn set_gain_db(&mut self, bus: BusId, gain_db: f32) {
        self.settings[bus.index()].gain_db = gain_db.clamp(-80.0, 12.0);
    }

    /// Silences or unsilences a bus.
    pub fn set_muted(&mut self, bus: BusId, muted: bool) {
        self.settings[bus.index()].muted = muted;
    }

    /// Begins blending a snapshot in, or re-engages one already known by this name.
    pub fn engage(&mut self, name: impl Into<String>, snapshot: Snapshot) {
        let name = name.into();
        if let Some(existing) = self.active.get_mut(&name) {
            existing.snapshot = snapshot;
            existing.engaged = true;
        } else {
            self.active.insert(
                name,
                Active {
                    snapshot,
                    weight: 0.0,
                    engaged: true,
                },
            );
        }
    }

    /// Begins blending a snapshot back out. Unknown names are ignored.
    pub fn release(&mut self, name: &str) {
        if let Some(active) = self.active.get_mut(name) {
            active.engaged = false;
        }
    }

    /// Whether a snapshot is engaged or still blending out.
    #[must_use]
    pub fn is_active(&self, name: &str) -> bool {
        self.active.contains_key(name)
    }

    /// Advances every blend by `elapsed_seconds`.
    ///
    /// A snapshot that has finished blending out is dropped, which is what keeps the map from growing
    /// without bound in a session where a briefing plays every minute.
    pub fn advance(&mut self, elapsed_seconds: f32) {
        let elapsed = elapsed_seconds.max(0.0);
        for active in self.active.values_mut() {
            let duration = if active.engaged {
                active.snapshot.attack_seconds
            } else {
                active.snapshot.release_seconds
            }
            .max(1e-4);
            let step = elapsed / duration;
            if active.engaged {
                active.weight = (active.weight + step).min(1.0);
            } else {
                active.weight = (active.weight - step).max(0.0);
            }
        }
        self.active
            .retain(|_, active| active.engaged || active.weight > 0.0);
    }

    /// The gain a bus should actually run at, settings and snapshots combined.
    ///
    /// Offsets sum in decibels rather than multiplying weights, so two snapshots each asking for -6 dB
    /// produce -12 and not -6. That is the behaviour a designer expects from stacked ducking, and it
    /// is also the one that cannot produce a *rise* from two attenuating snapshots.
    #[must_use]
    pub fn resolved_gain_db(&self, bus: BusId) -> f32 {
        let settings = &self.settings[bus.index()];
        if settings.muted {
            return -120.0;
        }
        let mut gain = settings.gain_db;
        for active in self.active.values() {
            if let Some(offset) = active.snapshot.offsets.get(&bus) {
                gain += offset * active.weight;
            }
        }
        gain
    }

    /// Whether a bus is currently silent, so a mixer can skip it entirely.
    #[must_use]
    pub fn is_silent(&self, bus: BusId) -> bool {
        self.resolved_gain_db(bus) <= -80.0
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::{BusId, BusSettings, Mix, Snapshot};

    #[test]
    fn every_bus_has_a_distinct_index_inside_the_count() {
        let mut seen = [false; BusId::COUNT];
        for bus in BusId::ALL {
            let index = bus.index();
            assert!(index < BusId::COUNT);
            assert!(!seen[index], "{bus:?} collides with another bus");
            seen[index] = true;
        }
        assert!(seen.iter().all(|&hit| hit), "a bus is missing from ALL");
    }

    #[test]
    fn master_is_processed_last_so_the_mixer_needs_one_pass() {
        assert_eq!(BusId::ALL[BusId::ALL.len() - 1], BusId::Master);
    }

    #[test]
    fn a_bus_the_engine_does_not_define_fails_to_load() {
        assert!(serde_json::from_str::<BusId>("\"master\"").is_ok());
        assert!(serde_json::from_str::<BusId>("\"cheat_bus\"").is_err());
    }

    #[test]
    fn a_snapshot_blends_in_over_its_attack_and_out_over_its_release() {
        let mut mix = Mix::new();
        mix.engage("duck", Snapshot::ducking());
        assert_eq!(mix.resolved_gain_db(BusId::Music), 0.0, "not yet blended");

        mix.advance(0.1);
        let half = mix.resolved_gain_db(BusId::Music);
        assert!(half < 0.0 && half > -12.0, "mid-blend was {half}");

        mix.advance(1.0);
        assert_eq!(mix.resolved_gain_db(BusId::Music), -12.0);

        mix.release("duck");
        mix.advance(1.0);
        assert_eq!(mix.resolved_gain_db(BusId::Music), 0.0);
        assert!(
            !mix.is_active("duck"),
            "a finished snapshot must be dropped"
        );
    }

    #[test]
    fn two_snapshots_sum_rather_than_fighting_over_the_gain() {
        // The reason ducking is not implemented as "the speech system turns the music down": a pause
        // during a briefing must be the two offsets together, not a race to restore the gain.
        let mut mix = Mix::new();
        mix.engage("briefing", Snapshot::ducking());
        mix.engage("pause", Snapshot::ducking());
        mix.advance(10.0);
        assert_eq!(mix.resolved_gain_db(BusId::Music), -24.0);

        mix.release("pause");
        mix.advance(10.0);
        assert_eq!(
            mix.resolved_gain_db(BusId::Music),
            -12.0,
            "releasing one must leave the other in force"
        );
    }

    #[test]
    fn a_snapshot_offset_stacks_on_top_of_the_players_setting() {
        let mut mix = Mix::new();
        mix.set_gain_db(BusId::Music, -6.0);
        mix.engage("duck", Snapshot::ducking());
        mix.advance(10.0);
        assert_eq!(mix.resolved_gain_db(BusId::Music), -18.0);
    }

    #[test]
    fn muting_wins_over_everything() {
        let mut mix = Mix::new();
        mix.set_gain_db(BusId::Music, 12.0);
        mix.set_muted(BusId::Music, true);
        assert!(mix.is_silent(BusId::Music));
    }

    #[test]
    fn a_players_gain_is_clamped_to_what_a_slider_can_reach() {
        let mut mix = Mix::new();
        mix.set_gain_db(BusId::Effects, 1_000.0);
        assert_eq!(mix.resolved_gain_db(BusId::Effects), 12.0);
        mix.set_settings(
            BusId::Effects,
            BusSettings {
                gain_db: -1_000.0,
                ..BusSettings::default()
            },
        );
        assert_eq!(mix.resolved_gain_db(BusId::Effects), -80.0);
    }

    #[test]
    fn re_engaging_a_snapshot_that_is_still_releasing_resumes_it() {
        // A briefing interrupted by a second briefing must not have the duck bounce back to unity in
        // between, which is what a release-then-new-attack would do.
        let mut mix = Mix::new();
        mix.engage("duck", Snapshot::ducking());
        mix.advance(10.0);
        mix.release("duck");
        mix.advance(0.1);
        let partway = mix.resolved_gain_db(BusId::Music);
        assert!(partway < 0.0);

        mix.engage("duck", Snapshot::ducking());
        mix.advance(0.01);
        assert!(
            mix.resolved_gain_db(BusId::Music) <= partway,
            "re-engaging must continue from where the release got to"
        );
    }

    #[test]
    fn a_bus_settings_block_refuses_a_field_it_does_not_know() {
        assert!(serde_json::from_str::<BusSettings>(r#"{"gain_db":-6}"#).is_ok());
        assert!(serde_json::from_str::<BusSettings>(r#"{"gain_db":-6,"pan":0.5}"#).is_err());
    }
}
