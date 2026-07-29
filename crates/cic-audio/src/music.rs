//! Score playback: layers that come and go with what is happening, and transitions between tracks.
//!
//! # Why music is layers rather than tracks
//!
//! A strategy game has no idea how long its own scenes are. A track written for "a battle" is either
//! cut off when the battle ends early or loops audibly when it does not, and a game that only picks
//! between finished pieces has to choose which of those to be wrong about every time.
//!
//! Layers avoid the question. One piece is recorded as several stems — a bed, a rhythm section, a brass
//! line — all the same length and all playing at once, with the *mix* between them following what the
//! game is doing. Nothing starts, nothing stops, and the music intensifies by a stem coming up rather
//! than by a new piece beginning. The transition is inaudible because there is no transition.
//!
//! # The one rule that makes it work
//!
//! **Layers are started together and never individually restarted.** They stay in phase only because
//! they began on the same frame and advance at the same rate; the moment one is stopped and started
//! again to "bring it back in", it is somewhere else in the bar and the result is not quiet music but
//! wrong music. So a layer that is inaudible is playing at zero gain, not stopped — which costs a voice
//! per layer for as long as the track is up, and is what the cost buys.
//!
//! # This holds no voices and reads no clock
//!
//! [`MusicDirector`] is a state machine over gains. It is advanced with an elapsed time and asked what
//! each layer should be at; starting voices and applying the result is [`crate::engine`]'s job. That is
//! what lets the whole of it be tested without a device, and what lets a caller drive it from a fixed
//! step rather than from a real clock.

/// How a layer's gain follows the intensity.
#[derive(Debug, Clone, PartialEq)]
pub struct MusicLayer {
    /// Virtual path of the stem.
    pub clip: String,
    /// Intensity at which this layer begins to be audible.
    pub enters_at: f32,
    /// Intensity at which it reaches full gain.
    pub full_at: f32,
}

impl MusicLayer {
    /// A layer that is always at full gain — the bed every track needs at least one of.
    #[must_use]
    pub fn bed(clip: impl Into<String>) -> Self {
        Self {
            clip: clip.into(),
            enters_at: 0.0,
            full_at: 0.0,
        }
    }

    /// A layer that fades in across a range of intensity.
    #[must_use]
    pub fn entering(clip: impl Into<String>, enters_at: f32, full_at: f32) -> Self {
        Self {
            clip: clip.into(),
            enters_at,
            full_at,
        }
    }

    /// The gain this layer wants at `intensity`, from `0.0` to `1.0`.
    #[must_use]
    pub fn gain_at(&self, intensity: f32) -> f32 {
        let enters = self.enters_at;
        let full = self.full_at;
        if intensity <= enters {
            return if enters <= 0.0 { 1.0 } else { 0.0 };
        }
        if full <= enters || intensity >= full {
            return 1.0;
        }
        (intensity - enters) / (full - enters)
    }
}

/// A piece of music, as a set of stems.
#[derive(Debug, Clone, PartialEq)]
pub struct MusicTrack {
    /// The stems, all the same length.
    pub layers: Vec<MusicLayer>,
}

impl MusicTrack {
    /// Builds a track from its layers.
    #[must_use]
    pub fn new(layers: Vec<MusicLayer>) -> Self {
        Self { layers }
    }
}

/// What the director is currently doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MusicPhase {
    /// Nothing is playing.
    Silent,
    /// One track is up.
    Playing,
    /// One track is coming up while another goes down.
    Crossfading,
}

/// The score's state machine.
#[derive(Debug, Clone, PartialEq)]
pub struct MusicDirector {
    incoming: Option<MusicTrack>,
    outgoing: Option<MusicTrack>,
    /// Crossfade progress, from `0.0` at the start to `1.0` when the incoming track is alone.
    progress: f32,
    crossfade_seconds: f32,
    intensity: f32,
    target_intensity: f32,
    /// How much the intensity may change per second.
    intensity_rate: f32,
}

impl Default for MusicDirector {
    fn default() -> Self {
        Self::new()
    }
}

impl MusicDirector {
    /// Seconds a crossfade takes when the caller does not say.
    pub const DEFAULT_CROSSFADE_SECONDS: f32 = 3.0;
    /// How fast intensity moves by default, in units per second.
    ///
    /// Slow on purpose. A battle starting is not a reason for the brass to arrive on the same frame —
    /// music that tracks the game instantly reads as a meter rather than as a score. Two seconds from
    /// calm to full is about as fast as it can move without sounding reactive.
    pub const DEFAULT_INTENSITY_RATE: f32 = 0.5;

    /// A director with nothing playing.
    #[must_use]
    pub fn new() -> Self {
        Self {
            incoming: None,
            outgoing: None,
            progress: 1.0,
            crossfade_seconds: Self::DEFAULT_CROSSFADE_SECONDS,
            intensity: 0.0,
            target_intensity: 0.0,
            intensity_rate: Self::DEFAULT_INTENSITY_RATE,
        }
    }

    /// Sets how long a crossfade takes.
    pub fn set_crossfade_seconds(&mut self, seconds: f32) {
        self.crossfade_seconds = seconds.max(0.0);
    }

    /// Sets how fast the intensity may move, in units per second.
    pub fn set_intensity_rate(&mut self, rate: f32) {
        self.intensity_rate = rate.max(0.0);
    }

    /// Asks for a new intensity. The director moves toward it at its own rate.
    pub fn set_intensity(&mut self, intensity: f32) {
        self.target_intensity = intensity.clamp(0.0, 1.0);
    }

    /// The intensity currently in force, which lags what was asked for.
    #[must_use]
    pub const fn intensity(&self) -> f32 {
        self.intensity
    }

    /// What the director is doing.
    #[must_use]
    pub const fn phase(&self) -> MusicPhase {
        match (&self.incoming, &self.outgoing) {
            (None, None) => MusicPhase::Silent,
            (_, Some(_)) => MusicPhase::Crossfading,
            (Some(_), None) => MusicPhase::Playing,
        }
    }

    /// Starts `track`, crossfading from whatever was playing.
    ///
    /// A track already crossfading out is dropped rather than queued. Three tracks overlapping is not
    /// a mix anybody authored, and the alternative — refusing the new one — means the music stays
    /// wrong for the length of a fade the player cannot see.
    pub fn play(&mut self, track: MusicTrack) {
        if self.incoming.is_some() {
            self.outgoing = self.incoming.take();
            self.progress = 0.0;
        } else {
            self.progress = 1.0;
        }
        self.incoming = Some(track);
    }

    /// Fades the current track out and leaves nothing behind it.
    pub fn stop(&mut self) {
        if self.incoming.is_some() {
            self.outgoing = self.incoming.take();
            self.progress = 0.0;
        }
    }

    /// Advances the intensity ramp and any crossfade.
    pub fn advance(&mut self, elapsed_seconds: f32) {
        let elapsed = elapsed_seconds.max(0.0);

        let step = self.intensity_rate * elapsed;
        if self.intensity < self.target_intensity {
            self.intensity = (self.intensity + step).min(self.target_intensity);
        } else {
            self.intensity = (self.intensity - step).max(self.target_intensity);
        }

        if self.outgoing.is_some() {
            let step = if self.crossfade_seconds <= 0.0 {
                1.0
            } else {
                elapsed / self.crossfade_seconds
            };
            self.progress = (self.progress + step).min(1.0);
            if self.progress >= 1.0 {
                self.outgoing = None;
            }
        }
    }

    /// The track coming in, if any.
    #[must_use]
    pub const fn incoming(&self) -> Option<&MusicTrack> {
        self.incoming.as_ref()
    }

    /// The track going out, if any.
    #[must_use]
    pub const fn outgoing(&self) -> Option<&MusicTrack> {
        self.outgoing.as_ref()
    }

    /// The gain each layer of the incoming track should be at.
    #[must_use]
    pub fn incoming_gains(&self) -> Vec<f32> {
        let fade = crossfade_gains(self.progress)[1];
        self.incoming.as_ref().map_or_else(Vec::new, |track| {
            track
                .layers
                .iter()
                .map(|layer| layer.gain_at(self.intensity) * fade)
                .collect()
        })
    }

    /// The gain each layer of the outgoing track should be at.
    ///
    /// The outgoing track's layers keep the intensity mix they had, because changing it *while* it
    /// fades would be re-orchestrating a piece nobody will hear the end of.
    #[must_use]
    pub fn outgoing_gains(&self) -> Vec<f32> {
        let fade = crossfade_gains(self.progress)[0];
        self.outgoing.as_ref().map_or_else(Vec::new, |track| {
            track
                .layers
                .iter()
                .map(|layer| layer.gain_at(self.intensity) * fade)
                .collect()
        })
    }
}

/// Returns `[outgoing, incoming]` gains for a crossfade at `progress`.
///
/// Equal power rather than linear, for the reason panning is: two uncorrelated signals summed at 0.5
/// each produce 0.707 of the power either had alone, so a linear crossfade dips 3 dB in the middle. On
/// a pan that is a sound crossing the screen; on a crossfade it is the music briefly getting quieter
/// every time it changes, which is more noticeable because it happens at a moment the listener is
/// already attending to.
fn crossfade_gains(progress: f32) -> [f32; 2] {
    let progress = progress.clamp(0.0, 1.0);
    let angle = progress * std::f32::consts::FRAC_PI_2;
    [angle.cos(), angle.sin()]
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::{MusicDirector, MusicLayer, MusicPhase, MusicTrack, crossfade_gains};

    fn track() -> MusicTrack {
        MusicTrack::new(vec![
            MusicLayer::bed("music/bed.wav"),
            MusicLayer::entering("music/rhythm.wav", 0.3, 0.6),
            MusicLayer::entering("music/brass.wav", 0.7, 1.0),
        ])
    }

    #[test]
    fn a_bed_is_always_up_and_a_layer_enters_across_its_range() {
        let layers = track().layers;
        assert_eq!(layers[0].gain_at(0.0), 1.0);
        assert_eq!(layers[0].gain_at(1.0), 1.0);

        assert_eq!(layers[1].gain_at(0.2), 0.0);
        assert!((layers[1].gain_at(0.45) - 0.5).abs() < 1e-5);
        assert_eq!(layers[1].gain_at(0.6), 1.0);
        assert_eq!(layers[1].gain_at(1.0), 1.0);
    }

    #[test]
    fn a_layer_with_no_range_is_a_step_rather_than_a_division_by_zero() {
        let degenerate = MusicLayer::entering("x", 0.5, 0.5);
        assert_eq!(degenerate.gain_at(0.4), 0.0);
        assert_eq!(degenerate.gain_at(0.6), 1.0);
        assert!(degenerate.gain_at(0.5).is_finite());
    }

    #[test]
    fn intensity_ramps_rather_than_jumping() {
        // Music that tracks the game instantly reads as a meter rather than as a score.
        let mut director = MusicDirector::new();
        director.play(track());
        director.set_intensity(1.0);
        assert_eq!(
            director.intensity(),
            0.0,
            "the change must not be immediate"
        );

        director.advance(0.5);
        let partway = director.intensity();
        assert!(partway > 0.0 && partway < 1.0, "intensity was {partway}");

        director.advance(10.0);
        assert_eq!(director.intensity(), 1.0);
    }

    #[test]
    fn intensity_ramps_back_down_too() {
        let mut director = MusicDirector::new();
        director.play(track());
        director.set_intensity(1.0);
        director.advance(10.0);
        director.set_intensity(0.0);
        director.advance(0.5);
        assert!(director.intensity() < 1.0 && director.intensity() > 0.0);
        director.advance(10.0);
        assert_eq!(director.intensity(), 0.0);
    }

    #[test]
    fn a_crossfade_holds_power_constant_rather_than_dipping_in_the_middle() {
        // The same failure a linear pan has, and more noticeable here because it happens at a moment
        // the listener is already attending to.
        for step in 0..=10 {
            #[expect(clippy::cast_precision_loss, reason = "a loop counter below eleven")]
            let progress = step as f32 / 10.0;
            let gains = crossfade_gains(progress);
            let power = gains[0].mul_add(gains[0], gains[1] * gains[1]);
            assert!((power - 1.0).abs() < 1e-5, "power {power} at {progress}");
        }
    }

    #[test]
    fn starting_a_second_track_crossfades_and_then_drops_the_first() {
        let mut director = MusicDirector::new();
        assert_eq!(director.phase(), MusicPhase::Silent);

        director.play(track());
        assert_eq!(director.phase(), MusicPhase::Playing);
        assert_eq!(
            director.incoming_gains()[0],
            1.0,
            "a first track does not fade in"
        );

        director.play(track());
        assert_eq!(director.phase(), MusicPhase::Crossfading);
        director.advance(1.5);
        let mid_in = director.incoming_gains()[0];
        let mid_out = director.outgoing_gains()[0];
        assert!(
            mid_in > 0.0 && mid_out > 0.0,
            "both should be audible mid-fade"
        );

        director.advance(5.0);
        assert_eq!(director.phase(), MusicPhase::Playing);
        assert!(director.outgoing().is_none());
        assert_eq!(director.incoming_gains()[0], 1.0);
    }

    #[test]
    fn a_third_track_replaces_the_one_still_fading_out() {
        // Three tracks overlapping is not a mix anybody authored.
        let mut director = MusicDirector::new();
        director.play(track());
        director.play(track());
        director.advance(0.5);
        director.play(track());
        assert_eq!(director.phase(), MusicPhase::Crossfading);
        assert_eq!(director.outgoing().map(|t| t.layers.len()), Some(3));
    }

    #[test]
    fn stopping_fades_out_and_leaves_silence() {
        let mut director = MusicDirector::new();
        director.play(track());
        director.stop();
        assert_eq!(director.phase(), MusicPhase::Crossfading);
        assert!(director.incoming_gains().is_empty());
        director.advance(10.0);
        assert_eq!(director.phase(), MusicPhase::Silent);
        assert!(director.outgoing_gains().is_empty());
    }

    #[test]
    fn a_zero_length_crossfade_completes_in_one_step() {
        let mut director = MusicDirector::new();
        director.set_crossfade_seconds(0.0);
        director.play(track());
        director.play(track());
        director.advance(1.0 / 60.0);
        assert_eq!(director.phase(), MusicPhase::Playing);
    }

    #[test]
    fn the_layer_mix_follows_intensity_while_the_track_is_up() {
        let mut director = MusicDirector::new();
        director.play(track());
        director.set_intensity(1.0);
        director.advance(10.0);
        let gains = director.incoming_gains();
        assert_eq!(gains, vec![1.0, 1.0, 1.0]);

        director.set_intensity(0.0);
        director.advance(10.0);
        let gains = director.incoming_gains();
        assert_eq!(gains[0], 1.0, "the bed stays");
        assert_eq!(gains[1], 0.0);
        assert_eq!(gains[2], 0.0);
    }
}
