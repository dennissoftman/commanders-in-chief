//! Scenery sway: how wind moves a plant, and the numbers that decide it.
//!
//! # Provenance
//!
//! Written from scratch. [`LICENSING.md`](../../../LICENSING.md) records that the predecessor's sway
//! defaults and its ten sway families were a derived constant table, that they were deliberately not
//! seeded into this tree, and that they must be re-authored rather than recovered — this file is that
//! re-authoring, and the original was not consulted. Every constant below is derived here, from a stated
//! physical argument, and the derivation is the point: a number nobody can justify is
//! indistinguishable from a number that was copied.
//!
//! # The physical picture
//!
//! A tree is a tapered cantilever fixed at the ground. Under steady wind it takes a static deflection;
//! under the turbulence real wind is made of, it oscillates about that deflection at close to its first
//! natural frequency. Leaves and twigs are far lighter and far slacker relative to their mass, so they
//! move much faster and much less far.
//!
//! That gives three parts, each answering a different question, and they are separable because they act
//! on different time scales:
//!
//! 1. **A steady bend along the wind.** How far the plant leans while the wind blows.
//! 2. **A slow oscillation about that bend**, at the plant's own natural frequency. What makes it alive
//!    rather than merely displaced.
//! 3. **A fast, small, cross-wind flutter.** What makes a canopy read as air moving *through* it. A
//!    purely along-wind motion reads as a pump, however well tuned.
//!
//! # Why the phase is a function of position
//!
//! Two things would otherwise give the whole trick away at a glance. A stand of trees moving in unison
//! is the more obvious one, and it is fixed by giving each instance its own phase offset — derived from
//! its world position rather than drawn at random, so it is identical every frame and identical in a
//! capture. See [`sway_phase`], which is integer arithmetic for exactly that reason.
//!
//! The subtler one is that wind arrives *somewhere first*. A gust that reaches every plant at the same
//! instant is not wind, it is a global parameter change. So the phase also carries a term along the wind
//! direction, and the visible result is a front crossing the map. That term costs one dot product and it
//! is the single largest contributor to the effect reading as weather.
//!
//! # What the mesh contributes and what this file contributes
//!
//! The mesh contributes a per-vertex weight — how much of the sway that vertex takes, which for a plant
//! is essentially its height up the stem. This file contributes the physics applied to it. Keeping the
//! split there means a model with an authored weight channel and a model with none behave the same way,
//! and the exponent that turns a height ramp into a bending mode shape lives with the physics rather
//! than being baked irreversibly into geometry.

/// Wind speed at which the sway response is half of its saturated value, in world units per second.
///
/// Drag rises as the square of speed, but a bending element sheds load as it leans — its projected area
/// falls — so the real relationship is markedly sub-quadratic and has no closed form worth carrying in
/// a vertex shader. What is needed instead is a response that is *bounded*: nothing stops a scenario
/// authoring an absurd wind, and a shader has no way to refuse one. `speed / (speed + reference)` is
/// linear at low speed, saturates below one, and costs a single divide.
///
/// Eight is chosen so the interesting range is the authored range. Cloud drift already uses the same
/// wind vector in world units per second, so at 2 a breeze gives 0.20 of full sway and at 30 a gale
/// gives 0.79 — visible early, and never running away.
pub const SWAY_REFERENCE_SPEED: f32 = 8.0;

/// The share of the along-wind bend that does not oscillate.
///
/// This and [`SWAY_OSCILLATION`] sum to one, so the bend reaches exactly the saturated amplitude at the
/// crest. The split between them is fixed by a constraint rather than by taste: the steady share must
/// **exceed** the oscillating one, because otherwise the trough is negative and the plant leans *into*
/// the wind for part of every cycle. Wind does not reverse; a sway that does reads as a metronome.
pub const SWAY_STEADY: f32 = 0.55;

/// The share of the along-wind bend that oscillates at the plant's natural frequency.
///
/// See [`SWAY_STEADY`] for why this is the smaller of the two. At 0.45 the trough is a tenth of the
/// crest, which is a deep enough breath to be legible without ever unbending.
pub const SWAY_OSCILLATION: f32 = 0.45;

/// Exponent applied to the per-vertex weight before it scales the displacement.
///
/// A cantilever's first mode shape is strongly super-linear in height — near the base the deflection
/// grows roughly as the square. Two matters visibly rather than academically: under a linear ramp the
/// vertices just above the base move a noticeable fraction of the tip's distance, and a trunk whose foot
/// slides reads as a tree that is not planted. The square leaves the base genuinely still.
pub const SWAY_SHAPE_EXPONENT: f32 = 2.0;

/// World units between successive crests of the travelling gust front.
///
/// A gust has to be several plant spacings across. Much shorter and neighbours a few units apart are in
/// opposition, which looks like turbulence at the wrong scale; much longer and the front stops being
/// visible at all because everything in view is in step. At 90 units, plants a few units apart sit a
/// small fraction of a cycle behind each other and a front crosses a tactical view in a few seconds.
pub const SWAY_GUST_WAVELENGTH: f32 = 90.0;

/// Flutter frequency as a multiple of the plant's own sway frequency.
///
/// Leaves at roughly two cycles a second against a trunk near a third of one is a ratio around five,
/// which is where this starts. It is deliberately **not** the integer: this renderer has already been
/// caught by near-harmonic ratios once, when five summed water waves at related wavelengths interfered
/// into a visible diamond lattice (recorded in [the M3
/// milestone](../../../docs/milestones/m3-renderer.md)). Two motions at 5:1 repeat every trunk cycle and
/// the repeat is what the eye finds; at 5.37:1 they never close, so the canopy keeps producing figures
/// it has not produced before.
pub const SWAY_FLUTTER_RATIO: f32 = 5.37;

/// How a particular kind of scenery moves.
///
/// Four profiles, not a longer table. Each is a distinct physical regime — stiff trunk, slack stem,
/// bladed, and fixed — and a fifth would be an interpolation between two of these rather than a new
/// behaviour. Anything between them is reachable by [`Self::new`], which is the honest way to offer the
/// range without pretending the intermediate points are named things.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SwayProfile {
    /// Peak displacement at full wind, as a fraction of the vertex's own height above its anchor.
    ///
    /// A fraction rather than a distance, so the same profile suits a sapling and a mature tree: both
    /// lean by the same *proportion*, which is what the physics says and also what looks right.
    pub tip_fraction: f32,
    /// The plant's own sway frequency, in cycles per second.
    ///
    /// A cantilever's first natural frequency falls as the square of its length, which is why the
    /// profiles differ by so much more than their heights do.
    pub frequency: f32,
    /// Cross-wind flutter amplitude, as a fraction of the along-wind bend.
    pub flutter: f32,
}

impl SwayProfile {
    /// Scenery that does not move.
    ///
    /// Named rather than left to an `Option`, so a rock or a crate is a *stated* decision at the call
    /// site instead of the absence of one. Every figure is zero, so the displacement is exactly zero
    /// rather than a small one — which is what keeps a scene of rigid props byte-identical to the same
    /// scene rendered before sway existed.
    pub const RIGID: Self = Self {
        tip_fraction: 0.0,
        frequency: 0.0,
        flutter: 0.0,
    };

    /// A mature tree: a stiff trunk carrying a heavy canopy.
    ///
    /// A canopy ten metres up moving about six hundredths of that — a little over half a metre — at full
    /// wind, at a shade over one cycle every three seconds. Both figures are what a tree of that size
    /// does: the amplitude is small enough that the silhouette stays a tree, and the period is slow
    /// enough to read as mass rather than as animation.
    pub const TREE: Self = Self {
        tip_fraction: 0.06,
        frequency: 0.35,
        flutter: 0.25,
    };

    /// A bush or a sapling: much shorter, much slacker, and correspondingly faster.
    ///
    /// The ideal cantilever relation would make something a fifth as tall twenty-five times as fast,
    /// which is far too fast to be believable — stiffness falls with size as well as length, and the two
    /// partly cancel. A few times faster is what a shrub actually does, so this sits near three times
    /// the tree's rate with nearly twice its relative amplitude.
    pub const SHRUB: Self = Self {
        tip_fraction: 0.10,
        frequency: 0.9,
        flutter: 0.45,
    };

    /// Grass and reeds: a blade with almost no bending stiffness.
    ///
    /// Nearly all flutter, and the largest relative amplitude of the three — a blade lays over in wind
    /// that a shrub barely notices. The flutter share is high enough that the motion is mostly across
    /// the wind, which is what a field of grass looks like from above.
    pub const GRASS: Self = Self {
        tip_fraction: 0.22,
        frequency: 1.6,
        flutter: 0.8,
    };

    /// Builds a profile between the named ones.
    ///
    /// Sanitised rather than validated, on the same reasoning as the display settings: these figures
    /// reach a vertex shader, where a NaN propagates into a position and takes the whole primitive with
    /// it. A non-finite figure becomes zero, which is the one value guaranteed to render something.
    #[must_use]
    pub fn new(tip_fraction: f32, frequency: f32, flutter: f32) -> Self {
        Self {
            tip_fraction: sanitise(tip_fraction, 0.0, 1.0),
            frequency: sanitise(frequency, 0.0, 16.0),
            flutter: sanitise(flutter, 0.0, 2.0),
        }
    }

    /// Whether this profile displaces anything at all.
    ///
    /// A batch whose profile does not move can skip the wind and time terms entirely, which is what
    /// makes rigid scenery cost nothing rather than costing a multiply by zero.
    #[must_use]
    pub fn moves(&self) -> bool {
        self.tip_fraction > 0.0 && self.frequency > 0.0
    }

    /// Packs the profile and an instance's phase into the four floats an instance carries.
    ///
    /// Per instance rather than per batch, and that is not an arbitrary choice: the sway has to be
    /// applied identically in the G-buffer pass, in all four shadow cascades, and in the motion target,
    /// and the instance buffer is the only per-draw data every one of those passes already binds. A
    /// separate bind group would have to be added to the shadow pipelines, which currently bind the
    /// terrain group and the cascade and nothing else. See `model_gbuffer.wgsl`.
    #[must_use]
    pub fn packed(&self, phase: f32) -> [f32; 4] {
        [self.tip_fraction, phase, self.frequency, self.flutter]
    }
}

impl Default for SwayProfile {
    fn default() -> Self {
        Self::RIGID
    }
}

/// A per-instance phase offset in radians, derived from a world position.
///
/// # Why it is derived rather than drawn
///
/// A stand of plants moving in unison is the single most obvious tell there is, so each instance needs
/// its own phase. Drawing one at random would work and would also make a capture irreproducible, which
/// this renderer does not allow — the same rule that keeps the wall clock out of [`crate::DeferredFrame`].
/// Deriving it from position instead gives a phase that is stable across frames, stable across runs, and
/// stable across a save and reload, because it was never stored.
///
/// # Why integer arithmetic
///
/// [Determinism](../../../docs/invariants/determinism.md) is a project invariant for anything that will
/// reach simulation state, and scenery placement will. Float hashing is where that goes wrong: the same
/// expression can round differently under a different instruction selection. Quantising to a sixteenth
/// of a world unit and mixing integers has one rounding step, at the start, on a value the caller
/// supplied — and a sixteenth of a unit is far finer than any spacing at which two plants would be told
/// apart anyway.
#[must_use]
pub fn sway_phase(position: [f32; 3]) -> f32 {
    // Sixteenths of a world unit, wrapped into the integer range. `as` saturates on overflow in Rust,
    // and a NaN converts to zero, so a hostile position produces a phase rather than a panic.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let quantised = |value: f32| -> u32 { (value * 16.0) as i32 as u32 };
    // Three odd multipliers with well-separated bit patterns, so two positions differing on one axis do
    // not land on the same mix. The constants are the odd 32-bit primes nearest 2^32 divided by the
    // golden ratio and its square and cube, which is a standard way to get multipliers whose products
    // spread across the whole word rather than clustering in the high bits.
    let mut state = quantised(position[0])
        .wrapping_mul(0x9E37_79B1)
        .wrapping_add(quantised(position[1]).wrapping_mul(0x85EB_CA6B))
        .wrapping_add(quantised(position[2]).wrapping_mul(0xC2B2_AE35));
    // Two rounds of xor-shift and multiply. One round leaves visible structure along an axis, which is
    // exactly the failure the cloud hash in `atmosphere.wgsl` was caught with: a hash correlated along
    // one axis produced stripes rather than noise, and a stand of trees would produce rows in step.
    state ^= state >> 15;
    state = state.wrapping_mul(0x2545_F491);
    state ^= state >> 13;
    // Mapped to a full turn from the top 24 bits, which are the best mixed. 24 rather than 32 because
    // every integer below 2^24 is exactly representable in `f32`, so each distinct hash gives a distinct
    // phase instead of several collapsing onto one through the conversion.
    #[allow(clippy::cast_precision_loss)]
    {
        (state >> 8) as f32 / 16_777_216.0 * core::f32::consts::TAU
    }
}

/// Clamps a figure into range, replacing a non-finite one with the lower bound.
fn sanitise(value: f32, minimum: f32, maximum: f32) -> f32 {
    if value.is_finite() {
        value.clamp(minimum, maximum)
    } else {
        minimum
    }
}

#[cfg(test)]
mod tests {
    // Every comparison here is against a constant this module declares directly, and every cast is of a
    // small loop counter or of a phase already bounded to a full turn.
    #![allow(
        clippy::float_cmp,
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::assertions_on_constants
    )]

    use super::{SWAY_FLUTTER_RATIO, SWAY_OSCILLATION, SWAY_STEADY, SwayProfile, sway_phase};

    #[test]
    fn the_bend_never_reverses() {
        // The constraint that fixes the split between the two along-wind terms. If the oscillating share
        // ever exceeded the steady one, the trough would be negative and the plant would lean into the
        // wind for part of every cycle -- a metronome rather than a tree.
        assert!(
            SWAY_OSCILLATION < SWAY_STEADY,
            "the oscillation must not overcome the steady bend"
        );
        assert!(
            (SWAY_STEADY + SWAY_OSCILLATION - 1.0).abs() < 1.0e-6,
            "the crest must reach exactly the saturated amplitude"
        );
    }

    #[test]
    fn the_flutter_ratio_is_not_near_harmonic() {
        // The water waves already taught this one: five summed at related wavelengths interfered into a
        // visible diamond lattice. Two motions at an integer ratio repeat every slow cycle, and the
        // repeat is what the eye finds.
        for harmonic in 1..=8 {
            let distance = (SWAY_FLUTTER_RATIO - harmonic as f32).abs();
            assert!(
                distance > 0.2,
                "{SWAY_FLUTTER_RATIO} is within {distance} of the {harmonic}:1 harmonic"
            );
        }
    }

    #[test]
    fn a_rigid_profile_does_not_move() {
        assert!(!SwayProfile::RIGID.moves());
        assert_eq!(SwayProfile::RIGID.packed(1.0), [0.0, 1.0, 0.0, 0.0]);
        assert_eq!(SwayProfile::default(), SwayProfile::RIGID);
    }

    #[test]
    fn the_named_profiles_get_faster_as_they_get_slacker() {
        // Not a tuning assertion but the physics: a cantilever's first frequency falls as the square of
        // its length, so the shorter and slacker a plant is, the faster *and* the further it moves. A
        // profile set that violated this ordering would have a number transposed.
        let profiles = [SwayProfile::TREE, SwayProfile::SHRUB, SwayProfile::GRASS];
        for pair in profiles.windows(2) {
            assert!(
                pair[1].frequency > pair[0].frequency,
                "slacker scenery must sway faster"
            );
            assert!(
                pair[1].tip_fraction > pair[0].tip_fraction,
                "slacker scenery must sway further"
            );
            assert!(
                pair[1].flutter > pair[0].flutter,
                "slacker scenery must flutter more"
            );
            assert!(pair[0].moves());
        }
    }

    #[test]
    fn a_profile_sanitises_rather_than_refusing() {
        // These figures reach a vertex shader, where a NaN propagates into a position and takes the
        // whole primitive with it. Zero is the one value guaranteed to render something.
        for broken in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let profile = SwayProfile::new(broken, broken, broken);
            assert!(!profile.moves(), "{broken} must not produce motion");
        }
        let clamped = SwayProfile::new(-1.0, 1_000.0, -5.0);
        assert!(clamped.tip_fraction >= 0.0 && clamped.frequency <= 16.0);
    }

    #[test]
    fn a_phase_is_in_range_reproducible_and_position_dependent() {
        let a = sway_phase([12.0, 34.0, 5.0]);
        let b = sway_phase([12.0, 34.0, 5.0]);
        assert!(
            (a - b).abs() < f32::EPSILON,
            "the same position must give the same phase, every frame and every run"
        );
        for position in [
            [0.0; 3],
            [1.0, 0.0, 0.0],
            [-4_000.0, 4_000.0, 250.0],
            [0.0625, 0.0, 0.0],
        ] {
            let phase = sway_phase(position);
            assert!(
                (0.0..=core::f32::consts::TAU).contains(&phase),
                "{position:?} gave {phase}"
            );
        }
    }

    #[test]
    fn a_row_of_instances_is_not_correlated_along_its_axis() {
        // The failure this actually guards against, which is the one the cloud hash in
        // `atmosphere.wgsl` was caught with: a hash correlated along one axis produces *rows in step*,
        // and that reads worse than no variation at all. Two properties say it is not correlated —
        // consecutive phases must not march in one direction, and the set must reach every part of the
        // turn rather than clustering.
        //
        // Deliberately not a pairwise-separation test. Sixteen values spread uniformly over a full turn
        // collide within a twentieth of a radian about twice by the birthday bound, so a test demanding
        // otherwise would be demanding something a good hash does not provide.
        let phases: Vec<f32> = (0u8..32)
            .map(|step| sway_phase([f32::from(step), 0.0, 0.0]))
            .collect();
        let rising = phases.windows(2).filter(|pair| pair[1] > pair[0]).count();
        assert!(
            (8..=23).contains(&rising),
            "{rising} of 31 steps rose: a monotone run means the hash is a ramp"
        );
        let mut quadrants = [0usize; 4];
        for phase in &phases {
            let index = ((phase / core::f32::consts::FRAC_PI_2) as usize).min(3);
            quadrants[index] += 1;
        }
        assert!(
            quadrants.iter().all(|count| *count >= 3),
            "phases clustered rather than spreading: {quadrants:?}"
        );
        // And a single step along each axis changes the phase, so the mix is not one-dimensional. Two
        // positions differing on only one axis landing on the same phase would put whole rows or columns
        // in step.
        let origin = sway_phase([0.0; 3]);
        for axis in 0..3 {
            let mut position = [0.0; 3];
            position[axis] = 1.0;
            assert!(
                (sway_phase(position) - origin).abs() > 0.05,
                "a unit step along axis {axis} left the phase unchanged"
            );
        }
    }

    #[test]
    fn a_non_finite_position_still_yields_a_phase() {
        // A placement arrives from a scenario file, and a phase is not worth failing a launch over.
        for broken in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let phase = sway_phase([broken, 0.0, 0.0]);
            assert!(phase.is_finite(), "{broken} produced {phase}");
        }
    }
}
