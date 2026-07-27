//! Reusable real-time-strategy camera model.
//!
//! Deliberately free of any window, input, or GPU dependency so the same camera drives the
//! inspection viewers, a future map editor, and the game itself. Callers translate their own input
//! into a [`CameraIntent`] of semantic actions and supply ground heights through [`GroundHeight`];
//! nothing here knows about key codes, `winit`, or `wgpu`.
//!
//! Every limit and rate in [`RtsCameraProfile::BASELINE`] is project-authored and expected to be
//! tuned by feel against real terrain. They are round numbers on purpose: nothing here is
//! reverse-engineered, so there is no external table to match and no reason to pretend otherwise.

/// Ground elevation lookup, so the camera can hold a height above the terrain beneath it without
/// depending on any particular terrain representation.
///
/// Returning `None` means "no ground known here", which the camera treats as a reason to hold its
/// last known elevation rather than to dive.
pub trait GroundHeight {
    /// Returns the ground elevation at a world XY position.
    fn height_at(&self, x: f32, y: f32) -> Option<f32>;
}

impl<F> GroundHeight for F
where
    F: Fn(f32, f32) -> Option<f32>,
{
    fn height_at(&self, x: f32, y: f32) -> Option<f32> {
        self(x, y)
    }
}

/// Ground elevation that is always flat. Useful for tests and for callers with no terrain yet.
#[derive(Debug, Clone, Copy, Default)]
pub struct FlatGround(pub f32);

impl GroundHeight for FlatGround {
    fn height_at(&self, _x: f32, _y: f32) -> Option<f32> {
        Some(self.0)
    }
}

/// [`RtsCameraProfile::adjust_speed`] is expressed as a fraction closed per logic tick, and the
/// simulation tick is 30 Hz. Converting through this keeps the same feel at any present rate
/// instead of making the smoothing faster on faster hardware.
const SIMULATION_LOGIC_HZ: f32 = 30.0;

/// Camera limits and rates.
///
/// Every field is public so a caller can load them from a data file, an editor can expose them, and
/// tests can construct degenerate cases directly.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RtsCameraProfile {
    /// Downward tilt from horizontal, in radians.
    pub pitch: f32,
    /// Initial rotation about the vertical axis, in radians.
    pub yaw: f32,
    /// Starting height above the ground beneath the focus.
    pub height: f32,
    /// Closest the camera may zoom toward the ground.
    pub minimum_height: f32,
    /// Furthest the camera may zoom out, when `enforce_maximum_height` is set.
    pub maximum_height: f32,
    /// Whether `maximum_height` binds while scrolling.
    pub enforce_maximum_height: bool,
    /// Fraction of the remaining height difference closed per logic tick, in `0..=1`.
    pub adjust_speed: f32,
    /// Pan distance per second above which ground-following height updates pause, so crossing a
    /// ridge at speed does not yank the camera.
    pub scroll_amount_cutoff: f32,
    /// Hard ceiling that binds even when `enforce_maximum_height` is false, so a profile that
    /// deliberately leaves the soft maximum unenforced still cannot zoom out of the world.
    pub absolute_maximum_height: f32,
    /// Fastest the tracked ground elevation may chase its samples, in world units per second.
    ///
    /// A rate rather than a per-frame step, so terrain following feels the same at any present rate,
    /// and low enough that cresting a ridge eases rather than snaps. Also the reason a malformed
    /// heightfield cannot fling the camera: a spike costs a bounded, brief drift.
    pub ground_units_per_second: f32,
    /// Base scroll rate in world units per second at the profile's default height, before the
    /// per-input factors below scale it.
    pub scroll_units_per_second: f32,
    /// Applied to keyboard-style panning on both axes.
    pub keyboard_scroll_factor: f32,
    /// Applied to pointer-drag panning across the screen.
    pub horizontal_scroll_factor: f32,
    /// Applied to pointer-drag panning up the screen. Kept separate from the horizontal factor
    /// because a 16:9 viewport gives vertical drags less pixel distance to work with, so matching
    /// the two factors makes vertical panning feel slower than horizontal at the same input.
    pub vertical_scroll_factor: f32,
    /// Height change applied per unit of zoom input.
    pub zoom_units_per_step: f32,
    /// Yaw change in radians per unit of rotate input.
    pub yaw_radians_per_unit: f32,
}

impl RtsCameraProfile {
    /// Project-authored starting point for a 1 world unit ~= 1 metre scale.
    ///
    /// This profile enforces `maximum_height`, so the soft and hard ceilings agree by default and
    /// `absolute_maximum_height` is pure belt-and-braces against a profile that deliberately unsets
    /// enforcement. Expect to tune every number here by feel.
    pub const BASELINE: Self = Self {
        pitch: 40.0 * core::f32::consts::PI / 180.0,
        yaw: 0.0,
        height: 240.0,
        minimum_height: 100.0,
        maximum_height: 400.0,
        enforce_maximum_height: true,
        adjust_speed: 0.25,
        scroll_amount_cutoff: 60.0,
        absolute_maximum_height: 600.0,
        ground_units_per_second: 45.0,
        scroll_units_per_second: 320.0,
        keyboard_scroll_factor: 1.0,
        horizontal_scroll_factor: 1.0,
        vertical_scroll_factor: 1.25,
        zoom_units_per_step: 24.0,
        yaw_radians_per_unit: 0.0075,
    };
}

impl Default for RtsCameraProfile {
    fn default() -> Self {
        Self::BASELINE
    }
}

/// One frame of semantic camera input.
///
/// Deliberately not key codes or mouse buttons: a caller maps its own bindings onto these, so the
/// game, the editor, and the debug viewers can bind differently and share the camera.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CameraIntent {
    /// Keyboard-style pan request in the camera's own ground plane, `x` right and `y` forward, each
    /// in `-1..=1`. Scaled by the keyboard factor.
    pub pan: [f32; 2],
    /// Pointer-drag pan request in the same space. Kept separate from `pan` because the profile
    /// scales dragging and key-holding by different factors, and splits dragging per axis.
    pub drag: [f32; 2],
    /// Zoom request for this frame. Positive moves the camera closer to the ground.
    pub zoom: f32,
    /// Rotation request about the vertical axis, in the same units the profile scales.
    pub rotate: f32,
    /// Whether to return to the starting height and yaw.
    pub reset: bool,
    /// Whether to return rotation alone to its starting yaw, leaving height and focus untouched.
    /// Conventional in the genre: a rotate click that did not drag snaps back to north.
    pub reset_rotation: bool,
}

impl CameraIntent {
    /// Returns the intent with each pan request clamped to unit length, so an unnormalized caller
    /// cannot pan faster diagonally than along an axis.
    #[must_use]
    pub fn normalized(mut self) -> Self {
        self.pan = clamp_to_unit(self.pan);
        self.drag = clamp_to_unit(self.drag);
        self
    }
}

/// A resolved camera pose, in a form any renderer can build a view matrix from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraPose {
    /// Camera position in world space, Z up.
    pub eye: [f32; 3],
    /// The ground point the camera is centred on.
    pub focus: [f32; 3],
    /// Unit vector from `eye` toward `focus`.
    pub forward: [f32; 3],
}

/// A real-time-strategy camera: a fixed-tilt view centred on a ground point, holding a height above
/// the terrain beneath that point.
#[derive(Debug, Clone, Copy)]
pub struct RtsCamera {
    profile: RtsCameraProfile,
    focus_xy: [f32; 2],
    yaw: f32,
    /// Height the camera is being asked to hold, before smoothing.
    target_height: f32,
    /// Height actually held this frame.
    height: f32,
    /// Ground elevation the camera is easing toward, as last sampled.
    sampled_ground: f32,
    /// Ground elevation actually in use this frame.
    ground: f32,
}

impl RtsCamera {
    /// Creates a camera centred on a ground position.
    #[must_use]
    pub fn new(profile: RtsCameraProfile, focus_xy: [f32; 2], ground: &impl GroundHeight) -> Self {
        let sampled = ground
            .height_at(focus_xy[0], focus_xy[1])
            .filter(|value| value.is_finite())
            .unwrap_or(0.0);
        let height = clamp_height(&profile, profile.height);
        Self {
            profile,
            focus_xy,
            yaw: profile.yaw,
            target_height: height,
            height,
            sampled_ground: sampled,
            ground: sampled,
        }
    }

    /// Returns the active profile.
    #[must_use]
    pub const fn profile(&self) -> RtsCameraProfile {
        self.profile
    }

    /// Replaces the profile, re-clamping the current and requested heights into its limits.
    pub fn set_profile(&mut self, profile: RtsCameraProfile) {
        self.profile = profile;
        self.target_height = clamp_height(&profile, self.target_height);
        self.height = clamp_height(&profile, self.height);
    }

    /// Returns the ground position the camera is centred on.
    #[must_use]
    pub const fn focus_xy(&self) -> [f32; 2] {
        self.focus_xy
    }

    /// Returns the rotation about the vertical axis, in radians.
    #[must_use]
    pub const fn yaw(&self) -> f32 {
        self.yaw
    }

    /// Returns the height currently held above the ground beneath the focus.
    #[must_use]
    pub const fn height(&self) -> f32 {
        self.height
    }

    /// Advances the camera by one frame.
    ///
    /// `delta_seconds` is real elapsed time; the per-tick adjust rate is converted so
    /// smoothing feels identical regardless of present rate.
    pub fn update(&mut self, intent: CameraIntent, delta_seconds: f32, ground: &impl GroundHeight) {
        let delta = if delta_seconds.is_finite() {
            delta_seconds.clamp(0.0, 0.25)
        } else {
            0.0
        };
        let intent = intent.normalized();

        if intent.reset {
            self.yaw = self.profile.yaw;
            self.target_height = clamp_height(&self.profile, self.profile.height);
        } else if intent.reset_rotation {
            self.yaw = self.profile.yaw;
        }

        if intent.rotate.is_finite() {
            self.yaw = wrap_angle(self.yaw + intent.rotate * self.profile.yaw_radians_per_unit);
        }

        if intent.zoom.is_finite() && intent.zoom != 0.0 {
            // Zooming in reduces height, so a positive request subtracts.
            let requested = self.target_height - intent.zoom * self.profile.zoom_units_per_step;
            self.target_height = clamp_height(&self.profile, requested);
        }

        // Pan speed scales with height so the ground appears to move at a consistent rate as the
        // view zooms, which is what makes an RTS camera feel the same at every zoom level.
        let height_scale = if self.profile.height > f32::EPSILON {
            self.height / self.profile.height
        } else {
            1.0
        };
        let rate = self.profile.scroll_units_per_second * height_scale.clamp(0.25, 4.0);
        // Keyboard and pointer-drag panning carry different factors, and dragging is split
        // per screen axis, so the two requests are scaled separately before being combined.
        let requested = [
            intent.pan[0] * self.profile.keyboard_scroll_factor
                + intent.drag[0] * self.profile.horizontal_scroll_factor,
            intent.pan[1] * self.profile.keyboard_scroll_factor
                + intent.drag[1] * self.profile.vertical_scroll_factor,
        ];
        // Pan axes follow where the camera is facing: `y` along its ground-plane forward direction
        // and `x` along its right. At zero yaw the camera looks down +X, so forward panning must
        // move along +X and rightward panning along -Y, matching the pose below.
        let (sine, cosine) = self.yaw.sin_cos();
        let travel = [
            (requested[1] * cosine + requested[0] * sine) * rate * delta,
            (requested[1] * sine - requested[0] * cosine) * rate * delta,
        ];
        self.focus_xy[0] += travel[0];
        self.focus_xy[1] += travel[1];

        // `scroll_amount_cutoff` is a per-tick scroll amount, compared squared
        // against the squared per-frame scroll vector, so it converts to a rate through the
        // logic rate rather than being a speed already. Comparing a per-second rate against the raw
        // value would trip the cutoff during essentially all panning.
        //
        // Pausing is not unconditional: scrolling too fast still adjusts while the
        // camera sits outside its height constraints, so a fast pan cannot strand it out of bounds.
        let travel_rate = if delta > f32::EPSILON {
            (travel[0] * travel[0] + travel[1] * travel[1]).sqrt() / delta
        } else {
            0.0
        };
        let cutoff_rate = self.profile.scroll_amount_cutoff.max(0.0) * SIMULATION_LOGIC_HZ;
        let scrolling_too_fast = travel_rate > cutoff_rate;
        if (!scrolling_too_fast || !self.height_within_limits())
            && let Some(sampled) = ground
                .height_at(self.focus_xy[0], self.focus_xy[1])
                .filter(|value| value.is_finite())
        {
            // Only the target moves here; the tracked value eases toward it below.
            self.sampled_ground = sampled;
        }

        // `adjust_speed` is a fraction closed per logic tick; hold that feel at any
        // present rate rather than closing the same fraction per rendered frame.
        let rate = self.profile.adjust_speed.clamp(0.0, 1.0);
        let blend = if rate >= 1.0 {
            1.0
        } else {
            1.0 - (1.0 - rate).powf(delta * SIMULATION_LOGIC_HZ)
        };
        self.height += (self.target_height - self.height) * blend;
        self.height = clamp_height(&self.profile, self.height);

        // Ease toward the sampled elevation and cap how fast it may move. Easing alone would still
        // lurch across a cliff edge, and capping alone would track at a constant harsh rate; both
        // together give terrain following that is smooth and still bounded against bad samples.
        let eased = (self.sampled_ground - self.ground) * blend;
        let limit = self.profile.ground_units_per_second.max(0.0) * delta;
        self.ground += eased.clamp(-limit, limit);
    }

    /// Whether the held height is strictly inside its limits, matching the profile's height
    /// constraint check.
    fn height_within_limits(&self) -> bool {
        let minimum = clamp_height(&self.profile, f32::MIN);
        let maximum = clamp_height(&self.profile, f32::MAX);
        self.height > minimum + f32::EPSILON && self.height < maximum - f32::EPSILON
    }

    /// Resolves the current pose.
    #[must_use]
    pub fn pose(&self) -> CameraPose {
        let focus = [self.focus_xy[0], self.focus_xy[1], self.ground];
        // A fixed tilt means the horizontal standoff follows from the held height: the camera sits
        // `height / tan(pitch)` behind the focus and `height` above it.
        let pitch = self
            .profile
            .pitch
            .clamp(0.05, core::f32::consts::FRAC_PI_2 - 0.01);
        let standoff = self.height / pitch.tan();
        let (sine, cosine) = self.yaw.sin_cos();
        let eye = [
            focus[0] - cosine * standoff,
            focus[1] - sine * standoff,
            focus[2] + self.height,
        ];
        let offset = [focus[0] - eye[0], focus[1] - eye[1], focus[2] - eye[2]];
        let length = (offset[0] * offset[0] + offset[1] * offset[1] + offset[2] * offset[2]).sqrt();
        let forward = if length > f32::EPSILON {
            [offset[0] / length, offset[1] / length, offset[2] / length]
        } else {
            [cosine, sine, -1.0]
        };
        CameraPose {
            eye,
            focus,
            forward,
        }
    }
}

/// Clamps a height into the profile's limits.
///
/// The lower bound always binds. The upper bound is the soft maximum only when the profile says to
/// enforce it, but the absolute ceiling always binds, because with enforcement off nothing in the
/// the hard ceiling stops a zoom-out from leaving the world.
fn clamp_height(profile: &RtsCameraProfile, height: f32) -> f32 {
    let minimum = profile.minimum_height.max(0.0);
    let mut maximum = profile.absolute_maximum_height;
    if profile.enforce_maximum_height {
        maximum = maximum.min(profile.maximum_height);
    }
    let maximum = maximum.max(minimum);
    if height.is_finite() {
        height.clamp(minimum, maximum)
    } else {
        minimum
    }
}

/// Clamps a two-axis request to unit length, so diagonal input is not faster than axis input.
fn clamp_to_unit(value: [f32; 2]) -> [f32; 2] {
    let x = if value[0].is_finite() { value[0] } else { 0.0 };
    let y = if value[1].is_finite() { value[1] } else { 0.0 };
    let magnitude = (x * x + y * y).sqrt();
    if magnitude > 1.0 {
        [x / magnitude, y / magnitude]
    } else {
        [x, y]
    }
}

/// Wraps an angle into `-PI..=PI` so yaw cannot accumulate without bound.
fn wrap_angle(angle: f32) -> f32 {
    if !angle.is_finite() {
        return 0.0;
    }
    let turn = core::f32::consts::TAU;
    let wrapped = angle - (angle / turn).round() * turn;
    wrapped.clamp(-turn, turn)
}

#[cfg(test)]
mod tests {
    use super::{CameraIntent, FlatGround, GroundHeight, RtsCamera, RtsCameraProfile, wrap_angle};

    fn flat() -> FlatGround {
        FlatGround(0.0)
    }

    #[test]
    fn the_baseline_profile_is_internally_consistent() {
        // There is no external table to conform to, so what is worth asserting is that the
        // profile cannot describe an impossible camera: the height bounds must be ordered, the
        // starting height must sit inside them, the smoothing fraction must stay a fraction, and
        // every rate must be positive. A typo in the constant table trips one of these.
        let profile = RtsCameraProfile::BASELINE;
        assert!(
            profile.minimum_height > 0.0,
            "minimum height must be above ground"
        );
        assert!(
            profile.minimum_height < profile.maximum_height,
            "height bounds must be ordered"
        );
        assert!(
            profile.maximum_height <= profile.absolute_maximum_height,
            "the soft maximum must not exceed the hard ceiling"
        );
        assert!(
            (profile.minimum_height..=profile.maximum_height).contains(&profile.height),
            "the starting height must sit inside its own bounds"
        );
        assert!(
            (0.0..=1.0).contains(&profile.adjust_speed),
            "adjust_speed is a fraction closed per tick"
        );
        assert!(profile.pitch > 0.0 && profile.pitch < core::f32::consts::FRAC_PI_2);
        for (rate, name) in [
            (profile.ground_units_per_second, "ground_units_per_second"),
            (profile.scroll_units_per_second, "scroll_units_per_second"),
            (profile.scroll_amount_cutoff, "scroll_amount_cutoff"),
            (profile.zoom_units_per_step, "zoom_units_per_step"),
            (profile.yaw_radians_per_unit, "yaw_radians_per_unit"),
        ] {
            assert!(rate > 0.0, "{name} must be positive, was {rate}");
        }
    }

    #[test]
    fn starting_pose_sits_behind_and_above_the_focus_at_the_profile_tilt() {
        let profile = RtsCameraProfile::default();
        let camera = RtsCamera::new(profile, [100.0, 200.0], &flat());
        let pose = camera.pose();
        assert!(
            (pose.eye[2] - profile.height).abs() < 0.001,
            "height above ground"
        );
        for (value, expected) in pose.focus.into_iter().zip([100.0, 200.0, 0.0]) {
            assert!((value - expected).abs() < 1.0e-4, "focus was {pose:?}");
        }
        // Zero yaw looks along +X, so the camera stands back along -X.
        assert!(pose.eye[0] < pose.focus[0]);
        assert!((pose.eye[1] - pose.focus[1]).abs() < 0.001);
        // The tilt is what the profile asked for.
        let horizontal =
            (pose.forward[0] * pose.forward[0] + pose.forward[1] * pose.forward[1]).sqrt();
        let tilt = (-pose.forward[2]).atan2(horizontal).to_degrees();
        assert!(
            (tilt - profile.pitch.to_degrees()).abs() < 0.01,
            "tilt was {tilt}"
        );
    }

    #[test]
    fn zoom_is_bounded_below_by_the_minimum_and_above_by_the_enforced_maximum() {
        // The baseline enforces its maximum, so that is the bound a zoom-out must hit.
        let profile = RtsCameraProfile::default();
        assert!(
            profile.enforce_maximum_height,
            "this test covers the baseline's enforcing behaviour"
        );
        let mut camera = RtsCamera::new(profile, [0.0, 0.0], &flat());
        for _ in 0..400 {
            camera.update(
                CameraIntent {
                    zoom: -1.0,
                    ..CameraIntent::default()
                },
                1.0 / 30.0,
                &flat(),
            );
        }
        assert!(
            camera.height() <= profile.maximum_height + 0.001,
            "enforced maximum must bind: {}",
            camera.height()
        );

        for _ in 0..400 {
            camera.update(
                CameraIntent {
                    zoom: 1.0,
                    ..CameraIntent::default()
                },
                1.0 / 30.0,
                &flat(),
            );
        }
        assert!(
            (camera.height() - profile.minimum_height).abs() < 0.001,
            "minimum must bind: {}",
            camera.height()
        );
    }

    #[test]
    fn the_absolute_ceiling_still_binds_when_a_profile_unsets_enforcement() {
        // A profile is free to leave the soft maximum unenforced. Nothing then stops a zoom-out
        // except the hard ceiling, which is the reason that field exists.
        let profile = RtsCameraProfile {
            enforce_maximum_height: false,
            ..RtsCameraProfile::default()
        };
        let mut camera = RtsCamera::new(profile, [0.0, 0.0], &flat());
        for _ in 0..400 {
            camera.update(
                CameraIntent {
                    zoom: -1.0,
                    ..CameraIntent::default()
                },
                1.0 / 30.0,
                &flat(),
            );
        }
        assert!(
            camera.height() > profile.maximum_height,
            "an unenforced maximum should not bind: {}",
            camera.height()
        );
        assert!(
            camera.height() <= profile.absolute_maximum_height + 0.001,
            "absolute ceiling must bind: {}",
            camera.height()
        );
    }

    #[test]
    fn height_smoothing_is_frame_rate_independent() {
        // The same elapsed time at different present rates must reach the same height, or the
        // per-tick rate would make smoothing faster on faster hardware.
        let settle = |steps: u32, delta: f32| {
            let mut camera = RtsCamera::new(RtsCameraProfile::default(), [0.0, 0.0], &flat());
            camera.update(
                CameraIntent {
                    zoom: -2.0,
                    ..CameraIntent::default()
                },
                0.0,
                &flat(),
            );
            for _ in 0..steps {
                camera.update(CameraIntent::default(), delta, &flat());
            }
            camera.height()
        };
        let slow = settle(30, 1.0 / 30.0);
        let fast = settle(240, 1.0 / 240.0);
        assert!(
            (slow - fast).abs() < 0.5,
            "one second of smoothing diverged: {slow} vs {fast}"
        );
    }

    #[test]
    fn ground_tracking_is_rate_limited_against_a_malformed_heightfield() {
        // A single absurd sample must cost a bounded drift, not a teleport, and the bound is a rate
        // so it holds regardless of present rate.
        let profile = RtsCameraProfile::default();
        let mut camera = RtsCamera::new(profile, [0.0, 0.0], &flat());
        let broken = |_x: f32, _y: f32| Some(-100_000.0_f32);
        let step = 1.0 / 30.0;
        camera.update(CameraIntent::default(), step, &broken);
        let travelled = -camera.pose().focus[2];
        let allowed = profile.ground_units_per_second * step;
        assert!(
            travelled <= allowed + 0.001,
            "one frame moved the ground {travelled}, above the {allowed} allowed"
        );
        assert!(travelled > 0.0, "it should still be tracking toward it");
    }

    #[test]
    fn rotation_reset_leaves_height_and_focus_alone() {
        // A rotate click that did not drag resets rotation, which must not also
        // undo the player's zoom or scroll position.
        let profile = RtsCameraProfile::default();
        let mut camera = RtsCamera::new(profile, [0.0, 0.0], &flat());
        camera.update(
            CameraIntent {
                pan: [1.0, 1.0],
                zoom: -2.0,
                rotate: 150.0,
                ..CameraIntent::default()
            },
            1.0 / 30.0,
            &flat(),
        );
        let moved_focus = camera.focus_xy();
        let raised = camera.height();
        assert!((camera.yaw() - profile.yaw).abs() > 0.001, "yaw turned");

        camera.update(
            CameraIntent {
                reset_rotation: true,
                ..CameraIntent::default()
            },
            1.0 / 30.0,
            &flat(),
        );
        assert!((camera.yaw() - profile.yaw).abs() < 0.001, "yaw returned");
        for (value, expected) in camera.focus_xy().into_iter().zip(moved_focus) {
            assert!((value - expected).abs() < 0.001, "focus was preserved");
        }
        assert!(
            camera.height() > raised - 5.0,
            "height should not have been reset: {} vs {raised}",
            camera.height()
        );
    }

    #[test]
    fn non_finite_ground_and_missing_ground_hold_the_last_elevation() {
        let mut camera = RtsCamera::new(RtsCameraProfile::default(), [0.0, 0.0], &FlatGround(25.0));
        let absent = |_x: f32, _y: f32| None;
        camera.update(CameraIntent::default(), 1.0 / 30.0, &absent);
        assert!((camera.pose().focus[2] - 25.0).abs() < 0.001);
        let broken = |_x: f32, _y: f32| Some(f32::NAN);
        camera.update(CameraIntent::default(), 1.0 / 30.0, &broken);
        assert!((camera.pose().focus[2] - 25.0).abs() < 0.001);
    }

    #[test]
    fn ordinary_panning_keeps_tracking_ground_and_only_extreme_scrolling_pauses_it() {
        // `scroll_amount_cutoff` is a per-tick amount, so as a rate it is the
        // value times the logic rate. Treating the raw value as a speed would trip the cutoff
        // during all normal panning and stop the camera following terrain at all.
        let profile = RtsCameraProfile::default();
        let raised = FlatGround(30.0);
        let mut ordinary = RtsCamera::new(profile, [0.0, 0.0], &flat());
        ordinary.update(
            CameraIntent {
                pan: [1.0, 0.0],
                ..CameraIntent::default()
            },
            1.0 / 30.0,
            &raised,
        );
        assert!(
            ordinary.pose().focus[2] > 0.0,
            "normal panning must still track terrain, ground is {}",
            ordinary.pose().focus[2]
        );

        // Far beyond the cutoff, tracking pauses -- but only while the height is within limits.
        let mut racing = RtsCamera::new(
            RtsCameraProfile {
                scroll_units_per_second: 50_000.0,
                ..profile
            },
            [0.0, 0.0],
            &flat(),
        );
        racing.update(
            CameraIntent {
                pan: [1.0, 0.0],
                ..CameraIntent::default()
            },
            1.0 / 30.0,
            &raised,
        );
        assert!(
            racing.pose().focus[2].abs() < 0.001,
            "scrolling far past the cutoff should pause tracking, ground is {}",
            racing.pose().focus[2]
        );
    }

    #[test]
    fn keyboard_and_drag_panning_each_apply_their_own_factor() {
        // Asserted against an explicit profile rather than the baseline, so retuning the baseline's
        // feel cannot break this. What is under test is that each factor reaches the axis it names
        // and scales distance proportionally -- not what any particular factor is set to.
        let profile = RtsCameraProfile {
            keyboard_scroll_factor: 1.0,
            horizontal_scroll_factor: 2.0,
            vertical_scroll_factor: 3.0,
            ..RtsCameraProfile::default()
        };
        let travel = |intent: CameraIntent| {
            let mut camera = RtsCamera::new(profile, [0.0, 0.0], &flat());
            camera.update(intent, 0.1, &flat());
            let position = camera.focus_xy();
            (position[0] * position[0] + position[1] * position[1]).sqrt()
        };
        let keyboard = travel(CameraIntent {
            pan: [1.0, 0.0],
            ..CameraIntent::default()
        });
        let drag_horizontal = travel(CameraIntent {
            drag: [1.0, 0.0],
            ..CameraIntent::default()
        });
        let drag_vertical = travel(CameraIntent {
            drag: [0.0, 1.0],
            ..CameraIntent::default()
        });
        let ratio = |a: f32, b: f32| a / b;
        assert!(
            (ratio(drag_horizontal, keyboard) - 2.0).abs() < 0.01,
            "horizontal drag should be twice keyboard: {drag_horizontal} vs {keyboard}"
        );
        assert!(
            (ratio(drag_vertical, keyboard) - 3.0).abs() < 0.01,
            "vertical drag should be three times keyboard: {drag_vertical} vs {keyboard}"
        );
    }

    #[test]
    fn the_baseline_pans_vertically_faster_than_horizontally() {
        // A 16:9 viewport gives a vertical drag less pixel distance, so the baseline deliberately
        // splits the two factors. This is a statement about the baseline's feel, kept separate
        // from the mechanical test above.
        let profile = RtsCameraProfile::default();
        assert!(
            profile.vertical_scroll_factor > profile.horizontal_scroll_factor,
            "vertical drag should outrun horizontal at equal input"
        );
    }

    #[test]
    fn panning_follows_yaw_and_diagonal_input_is_not_faster() {
        let profile = RtsCameraProfile::default();
        let mut forward = RtsCamera::new(profile, [0.0, 0.0], &flat());
        forward.update(
            CameraIntent {
                pan: [0.0, 1.0],
                ..CameraIntent::default()
            },
            0.1,
            &flat(),
        );
        let straight = forward.focus_xy();

        let mut diagonal = RtsCamera::new(profile, [0.0, 0.0], &flat());
        diagonal.update(
            CameraIntent {
                pan: [1.0, 1.0],
                ..CameraIntent::default()
            },
            0.1,
            &flat(),
        );
        let travelled =
            |position: [f32; 2]| (position[0] * position[0] + position[1] * position[1]).sqrt();
        assert!(
            (travelled(straight) - travelled(diagonal.focus_xy())).abs() < 0.001,
            "diagonal panning must not outrun axis panning"
        );

        // Rotating the camera must rotate which way "forward" pans.
        let mut turned = RtsCamera::new(profile, [0.0, 0.0], &flat());
        turned.update(
            CameraIntent {
                rotate: core::f32::consts::FRAC_PI_2 / profile.yaw_radians_per_unit,
                ..CameraIntent::default()
            },
            0.0,
            &flat(),
        );
        turned.update(
            CameraIntent {
                pan: [0.0, 1.0],
                ..CameraIntent::default()
            },
            0.1,
            &flat(),
        );
        let rotated = turned.focus_xy();
        assert!(
            rotated[0].abs() < travelled(straight) * 0.05,
            "a quarter turn should redirect forward panning: {rotated:?}"
        );
    }

    #[test]
    fn reset_restores_the_starting_height_and_yaw() {
        let profile = RtsCameraProfile::default();
        let mut camera = RtsCamera::new(profile, [0.0, 0.0], &flat());
        camera.update(
            CameraIntent {
                zoom: -3.0,
                rotate: 200.0,
                ..CameraIntent::default()
            },
            1.0 / 30.0,
            &flat(),
        );
        assert!((camera.yaw() - profile.yaw).abs() > 0.001);
        camera.update(
            CameraIntent {
                reset: true,
                ..CameraIntent::default()
            },
            1.0 / 30.0,
            &flat(),
        );
        assert!(
            (camera.yaw() - profile.yaw).abs() < 0.001,
            "yaw resets at once"
        );
        // Height is smoothed rather than snapped, so let it settle.
        for _ in 0..60 {
            camera.update(CameraIntent::default(), 1.0 / 30.0, &flat());
        }
        assert!(
            (camera.height() - profile.height).abs() < 0.5,
            "height settled to {}",
            camera.height()
        );
    }

    #[test]
    fn non_finite_input_and_elapsed_time_leave_the_camera_usable() {
        let mut camera = RtsCamera::new(RtsCameraProfile::default(), [0.0, 0.0], &flat());
        camera.update(
            CameraIntent {
                pan: [f32::NAN, f32::INFINITY],
                drag: [f32::INFINITY, f32::NAN],
                zoom: f32::NAN,
                rotate: f32::INFINITY,
                reset: false,
                reset_rotation: false,
            },
            f32::NAN,
            &flat(),
        );
        let pose = camera.pose();
        assert!(pose.eye.iter().all(|value| value.is_finite()), "{pose:?}");
        assert!(
            pose.forward.iter().all(|value| value.is_finite()),
            "{pose:?}"
        );
        assert!(camera.height().is_finite());
        assert!(camera.yaw().is_finite());
    }

    #[test]
    fn yaw_wrapping_stays_bounded_and_finite() {
        assert!((wrap_angle(core::f32::consts::TAU * 3.0)).abs() < 0.001);
        assert!(wrap_angle(f32::NAN).abs() < 1.0e-6);
        assert!(wrap_angle(1.0e30).is_finite());
    }

    #[test]
    fn a_closure_and_a_flat_ground_are_both_accepted_sources() {
        let closure = |x: f32, _y: f32| Some(x * 0.5);
        assert_eq!(closure.height_at(10.0, 0.0), Some(5.0));
        assert_eq!(FlatGround(3.0).height_at(0.0, 0.0), Some(3.0));
    }
}
