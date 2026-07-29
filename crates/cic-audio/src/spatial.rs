//! Where a sound is, where the ear is, and what the distance between them does to it.
//!
//! # The listener is not the camera, and in this genre that is not a detail
//!
//! In a first-person game the two coincide and nobody has to think about it. An RTS camera sits sixty
//! metres above the battlefield looking down, and putting the ear there makes every sound on the map
//! distant, quiet, and centred — the pan collapses because everything is nearly straight down, and the
//! attenuation curve reports a firefight forty metres away as sixty-two metres away because the height
//! dominates the horizontal distance.
//!
//! [`Listener::for_overhead_camera`] is the answer: the ear is placed along the line from the camera to
//! the point it is looking at, at a caller-chosen fraction of the way down, while the *orientation*
//! stays the camera's. At a fraction of one the ear is on the ground and the map is as wide as the
//! screen; at zero it is at the camera and the mix is a distant observer's. Neither extreme is right for
//! every zoom level, which is why it is a parameter rather than a constant.
//!
//! # Everything here is a pure function
//!
//! [`Spatialization::of`] takes a listener and an emitter and returns gains, a pitch ratio and a filter
//! cutoff. It holds no state, reads no clock, and allocates nothing — so the whole spatial model is
//! testable without a device, which is what the tests at the bottom of this file are.

use serde::{Deserialize, Serialize};

/// The speed of sound in dry air at 20 degrees Celsius, in metres per second.
///
/// It appears in exactly one place — the Doppler ratio — and it is named rather than written inline so
/// a caller working in units where a metre is not a metre can see what would have to change.
pub const SPEED_OF_SOUND: f32 = 343.0;

/// The largest pitch ratio Doppler is allowed to produce.
///
/// Uncapped, the ratio goes to infinity as a source approaches the speed of sound, and long before that
/// it is producing a sound nobody authored. Two octaves up and down is past anything that reads as
/// motion and short of anything that reads as a bug.
const MAX_DOPPLER_RATIO: f32 = 4.0;

/// Where the ear is, which way it faces, and how fast it is moving.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Listener {
    /// World position of the ear.
    pub position: [f32; 3],
    /// Unit vector the ear faces along.
    pub forward: [f32; 3],
    /// Unit vector out of the top of the head, which is what resolves left from right.
    pub up: [f32; 3],
    /// World velocity, for Doppler. Zero disables the listener's half of the effect.
    pub velocity: [f32; 3],
}

impl Default for Listener {
    fn default() -> Self {
        Self {
            position: [0.0, 0.0, 0.0],
            forward: [0.0, 0.0, -1.0],
            up: [0.0, 1.0, 0.0],
            velocity: [0.0, 0.0, 0.0],
        }
    }
}

impl Listener {
    /// Places the ear between an overhead camera and the ground point it is looking at.
    ///
    /// `descent` is the fraction of the way from the camera to its focus: `0.0` leaves the ear at the
    /// camera, `1.0` puts it on the ground under the cursor. Values are clamped to that range.
    ///
    /// The orientation stays the camera's, so left is still screen-left. Moving the ear without moving
    /// the facing is exactly the trick — the pan spreads out because the ear got closer to the plane the
    /// action is on, and the screen still agrees with the mix because the axes did not turn.
    #[must_use]
    pub fn for_overhead_camera(
        camera_position: [f32; 3],
        focus_position: [f32; 3],
        up: [f32; 3],
        descent: f32,
    ) -> Self {
        let descent = descent.clamp(0.0, 1.0);
        let to_focus = sub(focus_position, camera_position);
        let position = add(camera_position, scale(to_focus, descent));
        Self {
            position,
            forward: normalize_or(to_focus, [0.0, 0.0, -1.0]),
            up: normalize_or(up, [0.0, 1.0, 0.0]),
            velocity: [0.0, 0.0, 0.0],
        }
    }
}

/// How loudness falls off with distance.
///
/// Three curves rather than one because they are not interchangeable. `Inverse` is the physically
/// correct one and never reaches silence, so a sound using it is audible across the whole map at some
/// level. `Linear` reaches exactly zero at its far distance, which is what a designer wants for a sound
/// that must be *inaudible* outside a radius. `Exponential` is between them and is the usual choice for
/// ambience.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Attenuation {
    /// No distance falloff at all. For music, narration, and interface sounds.
    None,
    /// Falls to silence linearly between `near` and `far`.
    Linear {
        /// Distance below which the sound is at full gain.
        near: f32,
        /// Distance at which it reaches exactly zero.
        far: f32,
    },
    /// The inverse-square law, softened by `rolloff` and clamped at `far`.
    Inverse {
        /// Distance at which the sound is at full gain.
        reference: f32,
        /// Distance past which no further attenuation is applied.
        far: f32,
        /// Multiplier on the falloff rate. One is physical.
        rolloff: f32,
    },
    /// A power curve between `near` and `far`, reaching silence at `far`.
    Exponential {
        /// Distance below which the sound is at full gain.
        near: f32,
        /// Distance at which it reaches zero.
        far: f32,
        /// Curve exponent. One is linear; larger values hold the sound up longer and then drop it.
        rolloff: f32,
    },
}

impl Attenuation {
    /// Returns the gain multiplier this curve gives at `distance`.
    #[must_use]
    pub fn gain_at(self, distance: f32) -> f32 {
        let distance = distance.max(0.0);
        match self {
            Self::None => 1.0,
            Self::Linear { near, far } => {
                let (near, far) = ordered(near, far);
                if distance <= near {
                    1.0
                } else if distance >= far {
                    0.0
                } else {
                    1.0 - (distance - near) / (far - near)
                }
            }
            Self::Inverse {
                reference,
                far,
                rolloff,
            } => {
                let reference = reference.max(f32::EPSILON);
                let clamped = distance.clamp(reference, far.max(reference));
                // The standard inverse-distance model. At `rolloff` of one and `distance` of twice
                // the reference this is 0.5, which is the -6 dB per doubling everyone expects.
                reference / rolloff.max(0.0).mul_add(clamped - reference, reference)
            }
            Self::Exponential { near, far, rolloff } => {
                let (near, far) = ordered(near, far);
                if distance <= near {
                    1.0
                } else if distance >= far {
                    0.0
                } else {
                    let travelled = (distance - near) / (far - near);
                    (1.0 - travelled).powf(rolloff.max(0.01))
                }
            }
        }
    }

    /// The distance past which this curve is silent, if it has one.
    ///
    /// A voice beyond it can be culled outright rather than mixed at zero, which is what makes a
    /// thousand-unit battle affordable. [`Self::Inverse`] returns `None` because it never reaches zero.
    #[must_use]
    pub fn silence_distance(self) -> Option<f32> {
        match self {
            Self::None | Self::Inverse { .. } => None,
            Self::Linear { near, far } | Self::Exponential { near, far, .. } => {
                Some(ordered(near, far).1)
            }
        }
    }
}

/// A directional emitter's beam: full gain inside the inner angle, `outer_gain` outside the outer one.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cone {
    /// Full width in degrees of the region at full gain.
    pub inner_degrees: f32,
    /// Full width in degrees past which `outer_gain` applies.
    pub outer_degrees: f32,
    /// Gain applied outside the outer angle. Rarely zero — a tank's engine behind you is quieter,
    /// not absent.
    pub outer_gain: f32,
}

/// A sound's position in the world and how it should be heard from elsewhere.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Emitter {
    /// World position.
    pub position: [f32; 3],
    /// World velocity, for Doppler.
    pub velocity: [f32; 3],
    /// Which way the emitter points, used only when `cone` is set.
    pub direction: [f32; 3],
    /// The distance curve.
    pub attenuation: Attenuation,
    /// Directionality, if any.
    pub cone: Option<Cone>,
    /// How wide the source is, from `0.0` (a point, fully panned) to `1.0` (everywhere, no pan).
    ///
    /// A rocket is a point. A river is not, and panning one hard left because its origin marker
    /// happens to be there is the artefact this exists to prevent.
    pub spread: f32,
    /// Doppler strength, from `0.0` (off) to `1.0` (physical). Exaggeration above one is allowed.
    pub doppler: f32,
    /// How much of the sound is blocked by geometry, from `0.0` (clear) to `1.0` (fully occluded).
    ///
    /// Supplied by the caller rather than computed here: whether a building is in the way is a
    /// question about the world, and this crate deliberately knows nothing about one.
    pub occlusion: f32,
}

impl Default for Emitter {
    fn default() -> Self {
        Self {
            position: [0.0, 0.0, 0.0],
            velocity: [0.0, 0.0, 0.0],
            direction: [0.0, 0.0, -1.0],
            attenuation: Attenuation::Inverse {
                reference: 8.0,
                far: 400.0,
                rolloff: 1.0,
            },
            cone: None,
            spread: 0.0,
            doppler: 0.0,
            occlusion: 0.0,
        }
    }
}

/// What a mixer needs in order to place one voice: a gain per output channel, a playback rate
/// multiplier, and a cutoff for the filter that stands in for air and geometry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spatialization {
    /// Left and right gain, distance attenuation and cone already folded in.
    pub gains: [f32; 2],
    /// Playback rate multiplier from Doppler. One is unshifted.
    pub pitch: f32,
    /// Low-pass cutoff in hertz. Above the audible range means no filtering.
    pub low_pass_hz: f32,
    /// Straight-line distance from ear to source, which a caller uses for culling and for reverb sends.
    pub distance: f32,
}

impl Spatialization {
    /// The cutoff that means "do not filter", chosen above the audible range so a filter set to it is
    /// audibly transparent even before a mixer skips it.
    pub const UNFILTERED: f32 = 20_000.0;

    /// Computes how `emitter` sounds from `listener`.
    #[must_use]
    pub fn of(listener: &Listener, emitter: &Emitter) -> Self {
        let to_source = sub(emitter.position, listener.position);
        let distance = length(to_source);

        let mut gain = emitter.attenuation.gain_at(distance);
        if let Some(cone) = emitter.cone {
            gain *= cone_gain(cone, emitter.direction, to_source, distance);
        }

        // Occlusion is deliberately two effects rather than one. Blocking geometry removes high
        // frequencies far faster than it removes energy, so an occluded sound that is merely quieter
        // reads as distant while one that is quieter *and* duller reads as being behind something.
        let occlusion = emitter.occlusion.clamp(0.0, 1.0);
        gain *= 1.0 - 0.8 * occlusion;

        let pan = pan_of(listener, to_source, distance, emitter.spread);
        let gains = constant_power_pan(pan);

        Self {
            gains: [gains[0] * gain, gains[1] * gain],
            pitch: doppler_ratio(listener, emitter, to_source, distance),
            low_pass_hz: air_and_occlusion_cutoff(distance, occlusion),
            distance,
        }
    }
}

/// Returns the pan position in `[-1, 1]`, where `-1` is fully left.
fn pan_of(listener: &Listener, to_source: [f32; 3], distance: f32, spread: f32) -> f32 {
    // A source at the listener's exact position has no direction, and normalizing it would divide by
    // zero. Centring it is right rather than merely safe: a sound you are standing inside is not to
    // one side of you.
    if distance <= f32::EPSILON {
        return 0.0;
    }
    let forward = normalize_or(listener.forward, [0.0, 0.0, -1.0]);
    let up = normalize_or(listener.up, [0.0, 1.0, 0.0]);
    let right = normalize_or(cross(forward, up), [1.0, 0.0, 0.0]);
    let direction = scale(to_source, 1.0 / distance);
    let sideways = dot(direction, right).clamp(-1.0, 1.0);
    sideways * (1.0 - spread.clamp(0.0, 1.0))
}

/// Converts a pan position into a pair of gains that sum to constant *power* rather than constant
/// amplitude.
///
/// Linear panning is the obvious implementation and it is wrong: a source panned to the centre arrives
/// as 0.5 in each ear, which is 3 dB quieter than the same source panned hard to one side. A sound
/// crossing the screen audibly dips in the middle. The sine-cosine law holds the summed power constant,
/// so the crossing is level.
fn constant_power_pan(pan: f32) -> [f32; 2] {
    let angle = (pan.clamp(-1.0, 1.0) + 1.0) * (std::f32::consts::FRAC_PI_4);
    [angle.cos(), angle.sin()]
}

/// Returns the cone's gain for a source pointing `direction` heard from along `to_source`.
fn cone_gain(cone: Cone, direction: [f32; 3], to_source: [f32; 3], distance: f32) -> f32 {
    if distance <= f32::EPSILON {
        return 1.0;
    }
    let facing = normalize_or(direction, [0.0, 0.0, -1.0]);
    // `to_source` runs listener to source, so the angle wanted is between the emitter's facing and the
    // vector back toward the listener.
    let to_listener = scale(to_source, -1.0 / distance);
    let angle = dot(facing, to_listener).clamp(-1.0, 1.0).acos();

    let inner = (cone.inner_degrees.max(0.0).to_radians() * 0.5).min(std::f32::consts::PI);
    let outer = (cone.outer_degrees.max(0.0).to_radians() * 0.5)
        .min(std::f32::consts::PI)
        .max(inner);
    let outer_gain = cone.outer_gain.clamp(0.0, 1.0);

    if angle <= inner {
        1.0
    } else if angle >= outer {
        outer_gain
    } else {
        let travelled = (angle - inner) / (outer - inner);
        1.0 - travelled * (1.0 - outer_gain)
    }
}

/// Returns the playback rate multiplier from relative motion.
fn doppler_ratio(
    listener: &Listener,
    emitter: &Emitter,
    to_source: [f32; 3],
    distance: f32,
) -> f32 {
    let strength = emitter.doppler;
    if strength <= 0.0 || distance <= f32::EPSILON {
        return 1.0;
    }
    let direction = scale(to_source, 1.0 / distance);
    let listener_toward = dot(listener.velocity, direction);
    let source_away = dot(emitter.velocity, direction);

    // The denominator going to zero is a source at the speed of sound, and going negative is one past
    // it. Both are outside what the model describes, so it is clamped to a tenth of the speed of sound
    // rather than allowed to produce an infinity that would propagate into the resampler.
    let denominator = (SPEED_OF_SOUND + source_away).max(SPEED_OF_SOUND * 0.1);
    let physical = (SPEED_OF_SOUND + listener_toward) / denominator;

    // Strength interpolates from no shift toward the physical one, so a designer can dial the effect
    // without the pitch ceasing to track the motion.
    strength
        .mul_add(physical - 1.0, 1.0)
        .clamp(1.0 / MAX_DOPPLER_RATIO, MAX_DOPPLER_RATIO)
}

/// Returns the low-pass cutoff standing in for air absorption and for blocking geometry.
///
/// Air absorbs high frequencies faster than low ones, which is why distant thunder is a rumble and near
/// thunder is a crack. Modelling it as one first-order low-pass whose cutoff falls with distance is
/// crude and is most of the perceptual effect.
fn air_and_occlusion_cutoff(distance: f32, occlusion: f32) -> f32 {
    /// Distance at which air absorption starts being applied at all, in metres.
    const ONSET: f32 = 25.0;
    /// Distance at which air alone has brought the cutoff down to `FLOOR`.
    const FULL: f32 = 600.0;
    /// Lowest cutoff air absorption alone will produce, in hertz.
    const FLOOR: f32 = 1_800.0;
    /// Lowest cutoff full occlusion will produce, in hertz.
    const OCCLUDED_FLOOR: f32 = 550.0;

    let travelled = ((distance - ONSET) / (FULL - ONSET)).clamp(0.0, 1.0);
    let air = travelled.mul_add(
        FLOOR - Spatialization::UNFILTERED,
        Spatialization::UNFILTERED,
    );
    occlusion.mul_add(OCCLUDED_FLOOR - air, air)
}

/// Returns `(low, high)` whichever way round the caller supplied them.
///
/// Author error rather than hostile input — a bank is data this project validates on load — so the
/// curve tolerates it instead of producing a division by a negative range and a gain above one.
fn ordered(first: f32, second: f32) -> (f32, f32) {
    if first <= second {
        (first, second.max(first + f32::EPSILON))
    } else {
        (second, first)
    }
}

fn add(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] + right[0], left[1] + right[1], left[2] + right[2]]
}

fn sub(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn scale(vector: [f32; 3], factor: f32) -> [f32; 3] {
    [vector[0] * factor, vector[1] * factor, vector[2] * factor]
}

fn dot(left: [f32; 3], right: [f32; 3]) -> f32 {
    left[2].mul_add(right[2], left[0].mul_add(right[0], left[1] * right[1]))
}

fn cross(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [
        left[1].mul_add(right[2], -(left[2] * right[1])),
        left[2].mul_add(right[0], -(left[0] * right[2])),
        left[0].mul_add(right[1], -(left[1] * right[0])),
    ]
}

fn length(vector: [f32; 3]) -> f32 {
    dot(vector, vector).sqrt()
}

/// Normalizes, falling back to `fallback` for a vector too short to have a direction.
fn normalize_or(vector: [f32; 3], fallback: [f32; 3]) -> [f32; 3] {
    let magnitude = length(vector);
    if magnitude <= f32::EPSILON {
        fallback
    } else {
        scale(vector, 1.0 / magnitude)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::{Attenuation, Cone, Emitter, Listener, Spatialization};

    /// A listener at the origin facing down negative Z with Y up, so right is positive X.
    fn ear() -> Listener {
        Listener::default()
    }

    #[test]
    fn panning_holds_power_constant_across_the_screen() {
        // The property linear panning fails. A source crossing from left to right must not dip in
        // the middle, so the summed square of the two gains is the same everywhere.
        let listener = ear();
        for x in [-100.0, -30.0, 0.0, 30.0, 100.0f32] {
            let emitter = Emitter {
                position: [x, 0.0, -50.0],
                attenuation: Attenuation::None,
                ..Emitter::default()
            };
            let placed = Spatialization::of(&listener, &emitter);
            let power = placed.gains[0].mul_add(placed.gains[0], placed.gains[1] * placed.gains[1]);
            assert!(
                (power - 1.0).abs() < 1e-5,
                "power was {power} at x = {x}, so a crossing sound would dip"
            );
        }
    }

    #[test]
    fn a_source_to_the_right_is_louder_in_the_right_ear() {
        let placed = Spatialization::of(
            &ear(),
            &Emitter {
                position: [10.0, 0.0, 0.0],
                attenuation: Attenuation::None,
                ..Emitter::default()
            },
        );
        assert!(placed.gains[1] > placed.gains[0]);

        let placed = Spatialization::of(
            &ear(),
            &Emitter {
                position: [-10.0, 0.0, 0.0],
                attenuation: Attenuation::None,
                ..Emitter::default()
            },
        );
        assert!(placed.gains[0] > placed.gains[1]);
    }

    #[test]
    fn a_source_at_the_listeners_position_is_centred_rather_than_dividing_by_zero() {
        let placed = Spatialization::of(
            &ear(),
            &Emitter {
                position: [0.0, 0.0, 0.0],
                attenuation: Attenuation::None,
                doppler: 1.0,
                ..Emitter::default()
            },
        );
        assert!((placed.gains[0] - placed.gains[1]).abs() < 1e-6);
        assert!(placed.gains[0].is_finite());
        assert_eq!(placed.pitch, 1.0);
    }

    #[test]
    fn full_spread_removes_the_pan_and_nothing_else() {
        // A river's origin marker being on the left must not put the whole river on the left.
        let emitter = Emitter {
            position: [100.0, 0.0, 0.0],
            attenuation: Attenuation::None,
            spread: 1.0,
            ..Emitter::default()
        };
        let placed = Spatialization::of(&ear(), &emitter);
        assert!((placed.gains[0] - placed.gains[1]).abs() < 1e-6);
    }

    #[test]
    fn the_inverse_curve_loses_six_decibels_per_doubling() {
        let curve = Attenuation::Inverse {
            reference: 10.0,
            far: 10_000.0,
            rolloff: 1.0,
        };
        assert!((curve.gain_at(10.0) - 1.0).abs() < 1e-6);
        assert!((curve.gain_at(20.0) - 0.5).abs() < 1e-6);
        assert!((curve.gain_at(40.0) - 0.25).abs() < 1e-6);
        // Inside the reference distance it does not exceed full gain, which is what stops a sound
        // exploding as a unit walks over its emitter.
        assert!((curve.gain_at(0.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn only_the_curves_that_reach_zero_report_a_culling_distance() {
        // The inverse law never reaches silence, so a caller that culled on it would be cutting a
        // sound off mid-tail at whatever distance it invented.
        assert_eq!(
            Attenuation::Inverse {
                reference: 1.0,
                far: 100.0,
                rolloff: 1.0
            }
            .silence_distance(),
            None
        );
        assert_eq!(Attenuation::None.silence_distance(), None);
        assert_eq!(
            Attenuation::Linear {
                near: 5.0,
                far: 90.0
            }
            .silence_distance(),
            Some(90.0)
        );
    }

    #[test]
    fn a_reversed_near_and_far_is_tolerated_rather_than_producing_gain_above_one() {
        let curve = Attenuation::Linear {
            near: 90.0,
            far: 5.0,
        };
        for distance in [0.0, 2.5, 5.0, 50.0, 200.0f32] {
            let gain = curve.gain_at(distance);
            assert!(
                (0.0..=1.0).contains(&gain),
                "gain {gain} at {distance} is outside the unit range"
            );
        }
    }

    #[test]
    fn a_cone_is_quietest_behind_the_emitter() {
        let cone = Cone {
            inner_degrees: 60.0,
            outer_degrees: 180.0,
            outer_gain: 0.25,
        };
        // Emitter at the origin pointing along +X; listener is at the origin too, so place the
        // emitter away and point it at or away from the ear.
        let in_beam = Spatialization::of(
            &ear(),
            &Emitter {
                position: [50.0, 0.0, 0.0],
                direction: [-1.0, 0.0, 0.0],
                attenuation: Attenuation::None,
                cone: Some(cone),
                ..Emitter::default()
            },
        );
        let facing_away = Spatialization::of(
            &ear(),
            &Emitter {
                position: [50.0, 0.0, 0.0],
                direction: [1.0, 0.0, 0.0],
                attenuation: Attenuation::None,
                cone: Some(cone),
                ..Emitter::default()
            },
        );
        let beam_power = in_beam.gains[0] + in_beam.gains[1];
        let away_power = facing_away.gains[0] + facing_away.gains[1];
        assert!(
            away_power < beam_power * 0.4,
            "{away_power} vs {beam_power}"
        );
        assert!(
            away_power > 0.0,
            "an engine behind you is quieter, not absent"
        );
    }

    #[test]
    fn approaching_raises_the_pitch_and_receding_lowers_it() {
        let listener = ear();
        let approaching = Emitter {
            position: [0.0, 0.0, -100.0],
            // Moving toward the listener is moving along +Z, since the source is at -Z.
            velocity: [0.0, 0.0, 30.0],
            doppler: 1.0,
            attenuation: Attenuation::None,
            ..Emitter::default()
        };
        let receding = Emitter {
            velocity: [0.0, 0.0, -30.0],
            ..approaching
        };
        assert!(Spatialization::of(&listener, &approaching).pitch > 1.0);
        assert!(Spatialization::of(&listener, &receding).pitch < 1.0);
    }

    #[test]
    fn doppler_cannot_produce_an_infinity_at_the_speed_of_sound() {
        // Uncapped, the ratio's denominator reaches zero here. An infinite playback rate would
        // propagate straight into the resampler's position accumulator.
        let placed = Spatialization::of(
            &ear(),
            &Emitter {
                position: [0.0, 0.0, -100.0],
                velocity: [0.0, 0.0, -super::SPEED_OF_SOUND * 2.0],
                doppler: 1.0,
                attenuation: Attenuation::None,
                ..Emitter::default()
            },
        );
        assert!(placed.pitch.is_finite());
        assert!(placed.pitch >= 1.0 / super::MAX_DOPPLER_RATIO);
    }

    #[test]
    fn doppler_strength_of_zero_leaves_the_pitch_alone() {
        let placed = Spatialization::of(
            &ear(),
            &Emitter {
                position: [0.0, 0.0, -100.0],
                velocity: [0.0, 0.0, 200.0],
                doppler: 0.0,
                ..Emitter::default()
            },
        );
        assert_eq!(placed.pitch, 1.0);
    }

    #[test]
    fn distance_and_occlusion_both_darken_the_sound() {
        let near = Spatialization::of(
            &ear(),
            &Emitter {
                position: [0.0, 0.0, -5.0],
                ..Emitter::default()
            },
        );
        let far = Spatialization::of(
            &ear(),
            &Emitter {
                position: [0.0, 0.0, -500.0],
                ..Emitter::default()
            },
        );
        let blocked = Spatialization::of(
            &ear(),
            &Emitter {
                position: [0.0, 0.0, -5.0],
                occlusion: 1.0,
                ..Emitter::default()
            },
        );
        assert_eq!(near.low_pass_hz, Spatialization::UNFILTERED);
        assert!(far.low_pass_hz < near.low_pass_hz);
        assert!(blocked.low_pass_hz < near.low_pass_hz);
        // Occlusion attenuates as well as filters, but not to silence: a sound behind a wall is
        // still there.
        let blocked_gain = blocked.gains[0] + blocked.gains[1];
        let clear_gain = near.gains[0] + near.gains[1];
        assert!(blocked_gain < clear_gain && blocked_gain > 0.0);
    }

    #[test]
    fn an_overhead_listener_hears_the_ground_from_closer_than_the_camera_does() {
        // The genre problem this exists for. A camera sixty metres up hears a firefight forty metres
        // away as seventy-two metres away, and everything on the map pans to the centre.
        let camera = [0.0, 60.0, 40.0];
        let focus = [0.0, 0.0, 0.0];
        let source = Emitter {
            position: [40.0, 0.0, 0.0],
            attenuation: Attenuation::None,
            ..Emitter::default()
        };

        let at_camera = Listener::for_overhead_camera(camera, focus, [0.0, 1.0, 0.0], 0.0);
        let lowered = Listener::for_overhead_camera(camera, focus, [0.0, 1.0, 0.0], 0.9);

        let high = Spatialization::of(&at_camera, &source);
        let low = Spatialization::of(&lowered, &source);
        assert!(
            low.distance < high.distance,
            "lowering the ear must shorten the distance to the ground"
        );

        // And the pan must widen, which is the audible half of the same fix.
        let high_spread = (high.gains[1] - high.gains[0]).abs();
        let low_spread = (low.gains[1] - low.gains[0]).abs();
        assert!(
            low_spread > high_spread,
            "lowering the ear must widen the stereo image, {low_spread} vs {high_spread}"
        );
    }

    #[test]
    fn the_descent_fraction_is_clamped() {
        let camera = [0.0, 60.0, 0.0];
        let focus = [0.0, 0.0, 0.0];
        let past_the_ground = Listener::for_overhead_camera(camera, focus, [0.0, 1.0, 0.0], 4.0);
        assert!((past_the_ground.position[1] - 0.0).abs() < 1e-6);
        let above_the_camera = Listener::for_overhead_camera(camera, focus, [0.0, 1.0, 0.0], -2.0);
        assert!((above_the_camera.position[1] - 60.0).abs() < 1e-6);
    }
}
