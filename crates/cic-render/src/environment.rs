//! Time of day, weather, and the sun, sky, fog and clouds they imply.
//!
//! # Why one type derives all of it
//!
//! Before this, the sun was a hardcoded preset (`DirectionalLight::daylight_with_occlusion`) and the sky
//! was two constants in a shader. Adding an overcast sky that way means editing a preset, two shader
//! constants, a fog colour and a cloud density and hoping they agree — five places holding one idea.
//! They are not independent: an overcast sky is dimmer *and* bluer-grey *and* foggier *and* has a higher,
//! flatter ambient, because all of those are the same cloud deck seen from different angles.
//!
//! So the inputs are two numbers a designer actually thinks in — an hour of the day and a weather state —
//! and everything else is derived. That also makes the whole model pure arithmetic, testable with no GPU,
//! which is where the awkward cases live: midnight, a sun exactly on the horizon, a fully overcast noon.
//!
//! # What is deliberately not here
//!
//! Falling rain and snow. Precipitation is *particles*, and M3 defers a particle system to the gameplay
//! that spawns effects. What is here is everything precipitation implies about the light — a darker,
//! flatter, foggier scene under a thicker cloud deck — plus the surface state (`wetness`, `snow`) that
//! shaders read. A storm reads as a storm from those alone; the falling motes are additive.

use crate::terrain::DirectionalLight;

/// Weather, as a set of independently blendable states rather than an enum of presets.
///
/// Blendable because weather transitions: a designer wants to ramp `overcast` from 0.2 to 0.9 over a
/// minute, and an enum of `Clear | Rain | Storm` forces a discontinuity at each boundary. Nothing here is
/// mutually exclusive — snow lying on the ground under a clearing sky is a real and common state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weather {
    /// Cloud deck thickness in `0..=1`. Dims and flattens the sun, raises ambient, thickens fog.
    pub overcast: f32,
    /// Surface saturation in `0..=1`. Darkens albedo and drops roughness.
    pub wetness: f32,
    /// Lying snow in `0..=1`. Read by the surface shaders, which settle it by slope.
    pub snow: f32,
    /// Instantaneous lightning intensity in `0..=1`.
    ///
    /// A *sample*, not a schedule. The renderer has no business owning when lightning strikes — that is
    /// a timeline the caller drives, and presentation must not be the thing that decides it, because a
    /// frame rate would then set the strike rate.
    pub flash: f32,
    /// Wind in world units per second, which drifts the cloud shadows.
    pub wind: [f32; 2],
}

impl Default for Weather {
    /// A clear, dry, still day — the state that changes nothing about a frame.
    fn default() -> Self {
        Self {
            overcast: 0.0,
            wetness: 0.0,
            snow: 0.0,
            flash: 0.0,
            wind: [0.0, 0.0],
        }
    }
}

impl Weather {
    /// Overcast and wet, with a brisk wind.
    #[must_use]
    pub fn rain() -> Self {
        Self {
            overcast: 0.8,
            wetness: 1.0,
            wind: [14.0, 5.0],
            ..Self::default()
        }
    }

    /// Overcast, wet, and dark, with a strong wind.
    ///
    /// `flash` stays zero: a strike is a moment the caller supplies, not a property of the weather.
    #[must_use]
    pub fn thunderstorm() -> Self {
        Self {
            overcast: 0.95,
            wetness: 1.0,
            wind: [26.0, 9.0],
            ..Self::default()
        }
    }

    /// Overcast and lying snow. Deliberately not wet — snow reads as dry until it melts.
    #[must_use]
    pub fn snowfall() -> Self {
        Self {
            overcast: 0.7,
            snow: 1.0,
            wind: [8.0, 3.0],
            ..Self::default()
        }
    }

    /// Clamps every figure into the range the shaders assume.
    #[must_use]
    pub fn sanitised(self) -> Self {
        let unit = |value: f32| {
            if value.is_finite() {
                value.clamp(0.0, 1.0)
            } else {
                0.0
            }
        };
        let finite = |value: f32| if value.is_finite() { value } else { 0.0 };
        Self {
            overcast: unit(self.overcast),
            wetness: unit(self.wetness),
            snow: unit(self.snow),
            flash: unit(self.flash),
            wind: [finite(self.wind[0]), finite(self.wind[1])],
        }
    }
}

/// Distance and height fog.
///
/// Marched along the view ray in six taps, with an exponential height falloff evaluated per tap.
///
/// Still not volumetric in the sense that buys light shafts, since nothing here samples the shadow map -
/// but no longer a closed form either, and that is a correction rather than an escalation. A closed form is
/// exact only while the density varies with height alone, and `patchiness` makes it vary laterally too. See
/// [ADR 0006](../../../docs/adr/0006-atmosphere.md).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fog {
    /// Fog accumulated per world unit of view distance at the reference height.
    pub density: f32,
    /// World units over which density falls to `1/e` as height rises above `base`.
    ///
    /// This is what makes fog read as *air* rather than as a distance haze: a hilltop stands clear while
    /// the valley beside it fills, which a purely distance-based term cannot express at any density.
    pub height_falloff: f32,
    /// The elevation the density is quoted at, in world units.
    pub base: f32,
    /// How much the density varies from place to place, in `0..=1`.
    ///
    /// Zero is a uniform haze, which is what fog looks like when nobody has thought about it. Real fog
    /// stands in banks, and the difference between the two is most of whether it reads as air.
    pub patchiness: f32,
    /// World units across one bank of fog.
    ///
    /// **Large** - comparable to how far the camera can see, not to a cloud.
    ///
    /// The density is *integrated* along each ray, so at a scale much smaller than the ray is long every ray
    /// crosses several banks and averages them to the same figure. Neighbouring pixels then agree, and the
    /// result is exactly the uniform haze the patchiness exists to avoid. A scale near the ray length keeps
    /// each ray largely inside one bank, so neighbouring rays genuinely differ.
    ///
    /// This is the opposite of what an earlier single-tap version wanted, and the two are easy to confuse:
    /// with one tap at the ray's midpoint a large scale gives too little variation *across the frame*, and
    /// with a march it gives too much averaging *along the ray*.
    pub patch_scale: f32,
}

impl Default for Fog {
    /// No fog. Chosen so a default frame is the frame this renderer produced before fog existed.
    fn default() -> Self {
        Self {
            density: 0.0,
            height_falloff: 120.0,
            base: 0.0,
            patchiness: 0.6,
            patch_scale: 1_100.0,
        }
    }
}

/// Drifting cloud shadows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Clouds {
    /// Fraction of the ground in shadow at any moment, in `0..=1`.
    pub coverage: f32,
    /// World units across one cell of the cloud pattern.
    ///
    /// Large — clouds are hundreds of metres across, and a pattern scaled like a texture detail reads as
    /// dappled foliage shadow rather than as weather.
    pub scale: f32,
    /// How much of the sun a cloud removes, in `0..=1`.
    ///
    /// Below one on purpose. A cloud shadow that reaches zero reads as a solid object's shadow; real
    /// cloud shade keeps a good deal of light because the deck scatters rather than blocks.
    pub strength: f32,
    /// Edge softness in `0..=1`, blurring the boundary between lit and shaded ground.
    pub softness: f32,
}

impl Default for Clouds {
    /// No clouds, for the same reason [`Fog::default`] is clear.
    fn default() -> Self {
        Self {
            coverage: 0.0,
            scale: 900.0,
            strength: 0.65,
            softness: 0.45,
        }
    }
}

/// Everything about the air and the light, from an hour and a weather state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Environment {
    /// Hours since midnight, wrapped into `0..24`.
    pub time_of_day: f32,
    /// The weather.
    pub weather: Weather,
    /// Fog, which weather thickens.
    pub fog: Fog,
    /// Cloud shadows, whose coverage weather raises.
    pub clouds: Clouds,
}

impl Default for Environment {
    /// Clear mid-morning: no fog, no clouds, a high sun.
    ///
    /// Every default here is chosen so a frame rendered through this environment is *identical* to the
    /// frame the renderer produced before an environment existed. That is what let the committed
    /// reference captures stay byte-identical across the change, which in turn is what proved the
    /// plumbing had not quietly altered the lighting it passes through.
    fn default() -> Self {
        Self {
            time_of_day: 10.0,
            weather: Weather::default(),
            fog: Fog::default(),
            clouds: Clouds::default(),
        }
    }
}

fn mix3(from: [f32; 3], to: [f32; 3], amount: f32) -> [f32; 3] {
    let amount = amount.clamp(0.0, 1.0);
    [
        from[0] + (to[0] - from[0]) * amount,
        from[1] + (to[1] - from[1]) * amount,
        from[2] + (to[2] - from[2]) * amount,
    ]
}

fn scale3(value: [f32; 3], by: f32) -> [f32; 3] {
    [value[0] * by, value[1] * by, value[2] * by]
}

fn add3(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] + right[0], left[1] + right[1], left[2] + right[2]]
}

/// Rotates the whole sun path, in radians.
///
/// Chosen so the default hour of 10 reproduces the heading of the preset this replaces: that light's
/// horizontal direction is `[-0.83, -0.55]`, or 33.7 degrees, and the unoffset sweep gives 60 degrees there.
/// A sun rising a little north of east is perfectly ordinary, so nothing is distorted by it.
const SUN_AZIMUTH_OFFSET: f32 = -0.459;

/// The lowest the sun's own direction is allowed to sit, as a cosine against vertical.
///
/// Above zero so the vector never lies in the ground plane, where the horizontal component would be the
/// whole of it and every surface would be lit edge-on.
const MINIMUM_SUN_HEIGHT: f32 = 0.05;

// The colour and intensity constants below are *calibrated against* `DirectionalLight::daylight_with_occlusion`,
// the hand-tuned preset this replaces as the default. That preset was arrived at by looking at captures on
// this renderer, through this tone curve, so it is the only trustworthy reference point available -- a set of
// physically-reasoned values that renders two stops darker is not more correct, it is just wrong here.
//
// At the default hour of 10 these reproduce that preset closely: diffuse near `[1.05, 0.98, 0.86]` and
// ambient near `[0.30, 0.34, 0.42]`, against its `[-0.45, -0.30, 0.84]` direction at a 57-degree elevation.
// A first attempt derived them from scratch and gave an ambient around `[0.09, 0.11, 0.15]`, roughly three
// times too dark, which would have visibly dimmed every scene the moment this became the default.

/// The beam's colour with the sun high.
///
/// Not neutral white: the ratios are the preset's own, `0.98/1.05` and `0.86/1.05`. Even a high sun is
/// slightly warm, because the shortest wavelengths have scattered out of it into the sky — which is the same
/// air that makes [`SKY_NEUTRAL`] blue, so the two are the two halves of one effect.
const SUN_NEUTRAL: [f32; 3] = [1.0, 0.93, 0.82];

/// The beam's colour at the deepest sunrise or sunset.
const SUN_LOW: [f32; 3] = [1.0, 0.72, 0.45];

/// Scales [`Environment::daylight`], which is a fraction, into the diffuse range this renderer expects.
///
/// Above one deliberately: fully lit ground sits near the top of the tone curve rather than halfway up it,
/// which is the same reason the composite exposes before applying Reinhard.
const SUN_DIFFUSE_GAIN: f32 = 1.24;

/// Skylight remaining when the sun is on the horizon, as a fraction of its value overhead.
const SKYLIGHT_FLOOR: f32 = 0.15;

/// Skylight with the sun high: a cool, fairly strong ambient, which is what the occlusion pass affords.
const SKY_NEUTRAL: [f32; 3] = [0.32, 0.36, 0.45];

/// Skylight at a low sun, warmer and less blue as the beam takes the blue out of the air.
const SKY_LOW: [f32; 3] = [0.34, 0.30, 0.30];

/// Skylight under a full cloud deck: brighter than clear sky and nearly colourless.
const SKY_OVERCAST: [f32; 3] = [0.42, 0.44, 0.48];

/// What a lightning flash adds to the ambient. Cold, because a discharge is.
const FLASH_AMBIENT: [f32; 3] = [0.55, 0.62, 0.85];

/// Sun elevation in radians at the horizon, below which it contributes no direct light.
const HORIZON: f32 = 0.0;

/// Hours from sunrise to sunset. A fixed civil day rather than a latitude model.
const DAY_LENGTH: f32 = 12.0;

/// Hour the sun rises.
const SUNRISE: f32 = 6.0;

impl Environment {
    /// The environment with its weather replaced.
    #[must_use]
    pub fn with_weather(mut self, weather: Weather) -> Self {
        self.weather = weather;
        // Weather and atmosphere are not independent: a thicker deck is a foggier, cloudier sky. Deriving
        // them here rather than leaving it to the caller is the whole point of the type — five fields
        // holding one idea is how they drift apart.
        let overcast = weather.sanitised().overcast;
        self.fog.density = self.fog.density.max(overcast * 0.0016);
        self.clouds.coverage = self.clouds.coverage.max(overcast * 0.85);
        self
    }

    /// Hours since midnight, wrapped into `0..24`.
    #[must_use]
    pub fn hour(&self) -> f32 {
        if !self.time_of_day.is_finite() {
            return 0.0;
        }
        self.time_of_day.rem_euclid(24.0)
    }

    /// The sun's elevation above the horizon in radians, negative at night.
    ///
    /// A half-sine over the daylight hours. Not an ephemeris: a strategy map wants a sun that reads
    /// correctly and is authored in one number, and a real solar position model would need a latitude, a
    /// date and a north reference that no map format carries.
    #[must_use]
    pub fn sun_elevation(&self) -> f32 {
        let day = (self.hour() - SUNRISE) / DAY_LENGTH;
        // Below the horizon outside daylight, so night is expressed by the same formula rather than by a
        // branch that would have to agree with it.
        std::f32::consts::PI * day.clamp(-0.5, 1.5)
    }

    /// The direction *toward* the sun, matching [`crate::terrain::DirectionalLight::direction`].
    #[must_use]
    pub fn sun_direction(&self) -> [f32; 3] {
        // Clamped above the horizon before the horizontal component is derived from it, so the vector stays
        // a unit vector at every hour. An earlier version squashed the north-south component by 0.6 to keep
        // the sun from swinging too far, which quietly made the vector shorter than one -- and since the
        // shader normalises it, the *effective* elevation came out higher than the figure that produced it.
        // A sun that reads as 60 degrees while claiming 57 is the kind of discrepancy nothing reports.
        let height = self.sun_elevation().sin().max(MINIMUM_SUN_HEIGHT);
        let horizontal = (1.0 - height * height).max(0.0).sqrt();
        // Sweeping the azimuth as well as the elevation is what stops shadows merely shortening and
        // lengthening in place over a day: they have to rotate, or dawn and dusk light the same faces.
        //
        // The offset orients that sweep. A single-parameter sun path cannot hit an arbitrary
        // (elevation, azimuth) pair — both come from the same hour — so without it the default hour landed 27
        // degrees away from the heading the preset used, which rotated every shadow and flattened a ridge
        // fixture shaped to run across the old light. The offset costs nothing and makes the derived sun a
        // genuine drop-in at the default hour, which is what replacing a preset ought to mean.
        let azimuth =
            SUN_AZIMUTH_OFFSET + std::f32::consts::PI * (self.hour() - SUNRISE) / DAY_LENGTH;
        [
            -azimuth.cos() * horizontal,
            -azimuth.sin() * horizontal,
            height,
        ]
    }

    /// How much daylight reaches the ground, in `0..=1`.
    ///
    /// Zero at and below the horizon, and reduced by the cloud deck. Squared against elevation rather
    /// than linear, because the extra air a low sun travels through removes light faster than the
    /// geometry alone suggests.
    #[must_use]
    pub fn daylight(&self) -> f32 {
        let elevation = self.sun_elevation().sin();
        if elevation <= HORIZON {
            return 0.0;
        }
        let height = elevation.clamp(0.0, 1.0);
        let clear = height.mul_add(0.65, 0.35) * height.sqrt();
        clear * (1.0 - 0.75 * self.weather.sanitised().overcast)
    }

    /// The directional light this environment implies.
    ///
    /// **Opt-in, not automatic.** [`crate::deferred::DeferredFrame::light`] stays authoritative, and a
    /// caller wanting a time-of-day sun assigns this to it. Deriving it behind the caller's back would
    /// have silently rewritten the light in every existing scene — including the ones the committed
    /// reference captures were taken from, which would have destroyed the only evidence that the rest of
    /// this plumbing changed nothing.
    #[must_use]
    pub fn sun_light(&self) -> DirectionalLight {
        let weather = self.weather.sanitised();
        let warmth = self.sun_warmth();

        // A low sun is warm because its light has crossed more air, which scatters the blue out of it. The
        // same scattering is why the ambient goes the other way and turns bluer as the sun drops: the light
        // removed from the beam is exactly the light filling the sky.
        let direct = mix3(SUN_NEUTRAL, SUN_LOW, warmth);
        let diffuse = scale3(direct, self.daylight() * SUN_DIFFUSE_GAIN);

        // Skylight rises with the sun but never reaches zero while it is up, because the sky is still lit
        // at dusk by air the ground cannot see the sun through.
        let lit = self.sun_elevation().sin().clamp(0.0, 1.0);
        let sky = SKYLIGHT_FLOOR + (1.0 - SKYLIGHT_FLOOR) * lit.sqrt();
        let clear_ambient = scale3(mix3(SKY_NEUTRAL, SKY_LOW, warmth), sky);

        // Overcast raises ambient while lowering the direct term: a cloud deck is a diffuser, so it moves
        // light out of the beam and into the sky rather than removing it. Getting this backwards -- dimming
        // both -- is what makes an overcast scene read as dusk instead of as daylight.
        let mut ambient = mix3(clear_ambient, SKY_OVERCAST, weather.overcast);

        // A lightning flash lifts ambient rather than the beam, and is cold. It is a discharge across the
        // whole sky, so it arrives from every direction at once -- adding it to the directional term would
        // put a hard shadow behind every object, from a source that has no position.
        if weather.flash > 0.0 {
            ambient = add3(ambient, scale3(FLASH_AMBIENT, weather.flash));
        }

        DirectionalLight {
            direction: self.sun_direction(),
            ambient,
            diffuse,
        }
    }

    /// How warm the sun is, in `0..=1`, where one is the deepest sunrise or sunset colour.
    ///
    /// Driven by elevation and not by the clock, so it stays correct as the day length changes and needs
    /// no second table of times to keep in step.
    #[must_use]
    pub fn sun_warmth(&self) -> f32 {
        let elevation = self.sun_elevation().sin().clamp(0.0, 1.0);
        // Warmth collapses quickly as the sun climbs: the golden band is a narrow one.
        (1.0 - elevation * 3.2).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    // Exact comparisons here are against values the model produces *structurally* rather than by
    // arithmetic: a zero returned by an early exit below the horizon, a clamp landing on its own bound, a
    // default that is literally the constant beside it. An epsilon comparison on those would weaken the
    // assertion — "the sun is off at night" means exactly zero, not nearly zero. The genuinely computed
    // comparisons in this module are all inequalities.
    #![allow(clippy::float_cmp)]

    use super::{Environment, Fog, Weather};

    fn at(hour: f32) -> Environment {
        Environment {
            time_of_day: hour,
            ..Environment::default()
        }
    }

    #[test]
    fn the_sun_is_up_by_day_and_down_by_night() {
        assert!(at(12.0).daylight() > 0.5, "noon should be bright");
        assert!(at(6.5).daylight() > 0.0, "just after sunrise is lit");
        // The horizon and everything past it, expressed by one formula rather than a branch.
        for hour in [0.0, 3.0, 6.0, 18.0, 21.0, 23.5] {
            assert_eq!(at(hour).daylight(), 0.0, "hour {hour} should be dark");
        }
    }

    #[test]
    fn noon_is_brighter_than_morning_and_morning_than_dawn() {
        let dawn = at(6.75).daylight();
        let morning = at(9.0).daylight();
        let noon = at(12.0).daylight();
        assert!(dawn < morning, "{dawn} !< {morning}");
        assert!(morning < noon, "{morning} !< {noon}");
    }

    #[test]
    fn a_low_sun_is_warm_and_a_high_sun_is_not() {
        // The property, not the constants: warmth has to fall as the sun climbs.
        assert!(at(6.2).sun_warmth() > 0.8, "dawn should be warm");
        assert_eq!(at(12.0).sun_warmth(), 0.0, "noon should be neutral");
        assert!(at(7.0).sun_warmth() > at(9.0).sun_warmth());
    }

    #[test]
    fn the_sun_rotates_as_well_as_climbing() {
        // Otherwise shadows only shorten and lengthen in place, and dawn lights the same faces as dusk.
        let morning = at(8.0).sun_direction();
        let afternoon = at(16.0).sun_direction();
        assert!(
            (morning[0] - afternoon[0]).abs() > 0.5,
            "azimuth barely moved: {morning:?} against {afternoon:?}"
        );
    }

    #[test]
    fn the_sun_never_points_below_the_ground() {
        // The shaders divide by this direction's length and light by its negation; a downward sun would
        // light the underside of the terrain.
        for hour in [0.0, 6.0, 12.0, 18.0, 23.0] {
            assert!(at(hour).sun_direction()[2] > 0.0, "hour {hour}");
        }
    }

    #[test]
    fn overcast_dims_the_day_without_extinguishing_it() {
        let clear = at(12.0).daylight();
        let dull = at(12.0).with_weather(Weather::rain()).daylight();
        assert!(dull < clear, "{dull} !< {clear}");
        assert!(dull > 0.0, "an overcast noon is dim, not dark");
    }

    #[test]
    fn weather_thickens_fog_and_raises_cloud_coverage() {
        // The reason the type exists: these are one idea, and a caller setting `overcast` alone should not
        // end up with a thick deck casting no shadow over perfectly clear air.
        let storm = Environment::default().with_weather(Weather::thunderstorm());
        assert!(storm.fog.density > 0.0);
        assert!(storm.clouds.coverage > 0.5);
    }

    #[test]
    fn an_explicit_fog_setting_is_not_lowered_by_fair_weather() {
        // `with_weather` raises the floor rather than assigning, so an authored fog bank survives a clear
        // sky. A designer asking for a misty dawn should not have it deleted by `overcast: 0.0`.
        let misty = Environment {
            fog: Fog {
                density: 0.02,
                ..Fog::default()
            },
            ..Environment::default()
        }
        .with_weather(Weather::default());
        assert!((misty.fog.density - 0.02).abs() < f32::EPSILON);
    }

    #[test]
    fn the_default_environment_reproduces_the_preset_it_replaces() {
        // The calibration, pinned.
        //
        // `daylight_with_occlusion` was arrived at by looking at captures on this renderer, through this tone
        // curve, so it is the only trustworthy statement of what "correct" means here. A sun derived from
        // physical reasoning that renders two stops darker is not more correct — it is simply wrong for this
        // pipeline. The first attempt at these constants gave an ambient around `[0.09, 0.11, 0.15]` against
        // the preset's `[0.30, 0.34, 0.42]`, roughly three times too dark, and would have dimmed every scene
        // the moment this became the default light.
        let light = Environment::default().sun_light();
        let preset = super::DirectionalLight::daylight_with_occlusion();
        for channel in 0..3 {
            let diffuse = (light.diffuse[channel] - preset.diffuse[channel]).abs();
            assert!(
                diffuse < 0.03,
                "diffuse channel {channel} differs by {diffuse:.3}: {:?} against {:?}",
                light.diffuse,
                preset.diffuse
            );
            let ambient = (light.ambient[channel] - preset.ambient[channel]).abs();
            assert!(
                ambient < 0.03,
                "ambient channel {channel} differs by {ambient:.3}: {:?} against {:?}",
                light.ambient,
                preset.ambient
            );
        }
        // And a comparable *direction*, not merely a comparable elevation. Colour alone is not enough: an
        // earlier version matched both diffuse and ambient exactly while sitting 27 degrees away in azimuth,
        // which rotated every shadow in the scene and visibly flattened a ridge fixture shaped to run across
        // the old light. The capture showed it; no assertion on brightness could have.
        for channel in 0..3 {
            let difference = (light.direction[channel] - preset.direction[channel]).abs();
            assert!(
                difference < 0.07,
                "direction channel {channel} differs by {difference:.3}: {:?} against {:?}",
                light.direction,
                preset.direction
            );
        }
    }

    #[test]
    fn the_sun_direction_is_a_unit_vector_at_every_hour() {
        // The shader normalises it, so a short vector does not fail — it silently reports a *higher*
        // elevation than the figure that produced it. An earlier version squashed the north-south component
        // and read as 60 degrees while claiming 57.
        for hour in [0.0, 6.0, 8.0, 10.0, 12.0, 16.0, 18.0, 23.0] {
            let [x, y, z] = at(hour).sun_direction();
            let length = (x * x + y * y + z * z).sqrt();
            assert!(
                (length - 1.0).abs() < 1.0e-4,
                "hour {hour} gives length {length:.4}"
            );
        }
    }

    #[test]
    fn the_default_environment_changes_nothing() {
        // Load-bearing. Every committed reference capture was rendered before an environment existed, and
        // they stay byte-identical only because the default is clear, fogless and cloudless.
        let environment = Environment::default();
        assert_eq!(environment.fog.density, 0.0);
        assert_eq!(environment.clouds.coverage, 0.0);
        assert_eq!(environment.weather, Weather::default());
    }

    #[test]
    fn overcast_moves_light_from_the_beam_into_the_sky() {
        // The relationship that is easy to get backwards. A cloud deck is a diffuser, not a dimmer: it
        // takes light out of the direct beam and puts it into the ambient. Lowering both is what makes an
        // overcast noon read as dusk.
        let clear = at(12.0).sun_light();
        let dull = at(12.0).with_weather(Weather::rain()).sun_light();
        assert!(
            dull.diffuse[1] < clear.diffuse[1],
            "the beam should weaken: {:?} against {:?}",
            dull.diffuse,
            clear.diffuse
        );
        assert!(
            dull.ambient[1] > clear.ambient[1],
            "the sky should brighten: {:?} against {:?}",
            dull.ambient,
            clear.ambient
        );
    }

    #[test]
    fn a_flash_lifts_the_ambient_and_leaves_the_beam_alone() {
        // Lightning is a discharge across the whole sky, so it has no position. Adding it to the
        // directional term would cast a hard shadow from a source that does not exist.
        let calm = Environment::default().with_weather(Weather::thunderstorm());
        let struck = Environment::default().with_weather(Weather {
            flash: 1.0,
            ..Weather::thunderstorm()
        });
        assert!(struck.sun_light().ambient[2] > calm.sun_light().ambient[2]);
        assert_eq!(struck.sun_light().diffuse, calm.sun_light().diffuse);
    }

    #[test]
    fn a_low_sun_is_a_warmer_beam_than_a_high_one() {
        let dawn = at(6.4).sun_light().diffuse;
        let noon = at(12.0).sun_light().diffuse;
        // Compared as ratios, since the dawn beam is also far dimmer in absolute terms.
        assert!(
            dawn[0] / dawn[2].max(1.0e-6) > noon[0] / noon[2].max(1.0e-6),
            "dawn {dawn:?} is not warmer than noon {noon:?}"
        );
    }

    #[test]
    fn the_hour_wraps_and_refuses_nonsense() {
        assert!((at(25.0).hour() - 1.0).abs() < 1.0e-5);
        assert!((at(-1.0).hour() - 23.0).abs() < 1.0e-5);
        assert_eq!(at(f32::NAN).hour(), 0.0);
        assert_eq!(at(f32::INFINITY).hour(), 0.0);
    }

    #[test]
    fn nonsense_weather_is_clamped_rather_than_reaching_a_shader() {
        let wild = Weather {
            overcast: 4.0,
            wetness: -2.0,
            snow: f32::NAN,
            flash: f32::INFINITY,
            wind: [f32::NAN, 3.0],
        }
        .sanitised();
        assert_eq!(wild.overcast, 1.0);
        assert_eq!(wild.wetness, 0.0);
        assert_eq!(wild.snow, 0.0);
        assert_eq!(wild.flash, 0.0);
        assert_eq!(wild.wind, [0.0, 3.0]);
    }
}
