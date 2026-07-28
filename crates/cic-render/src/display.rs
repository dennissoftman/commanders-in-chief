//! Display settings: the resolution the chain renders at, and which antialiasing it resolves with.
//!
//! # Why these two together
//!
//! They are the two halves of one player-facing choice. [ADR
//! 0005](../../../docs/adr/0005-antialiasing-strategy.md) declines MSAA outright — multisampling a
//! deferred G-buffer costs four times the memory on every target *and* per-sample lighting behind a
//! stencil pass, and having paid for that it still fixes only geometric edges — and settles on a
//! resolution scale as the primary control with a post pass as the cheap floor beneath it. A settings
//! screen presents them side by side, so they are one value here rather than two.
//!
//! # Why the arithmetic is a pure function
//!
//! [`DisplaySettings::render_size`] decides how large every intermediate target is. That is the kind of
//! calculation whose failures are silent — an off-by-one on a rounded dimension, a scale that quietly
//! allocates a target the device will not create — so it is testable without a GPU, in the same spirit
//! as the cascade fitting in [`crate::shadow`].

/// The smallest render scale offered.
///
/// Below a half the image is soft enough that no sharpen or antialiasing recovers it, and offering a
/// setting that makes the game look broken is worse than not offering it.
pub const MIN_RESOLUTION_SCALE: f32 = 0.5;

/// The largest render scale offered.
///
/// Two is four times the pixels and therefore roughly four times the cost of every screen-space pass,
/// which is already more than most machines have spare. Beyond it the returns are slight — the
/// sampling rate is past what the display can show — and the cost is not.
pub const MAX_RESOLUTION_SCALE: f32 = 2.0;

/// The largest dimension a render target may reach.
///
/// This is `wgpu`'s default `max_texture_dimension_2d`, so a scale that would exceed it is clamped
/// rather than left to fail at texture creation. It also matches the capture bound in [`crate::gpu`],
/// which is not a coincidence: a capture is a render target too.
pub const MAX_RENDER_DIMENSION: u32 = 8_192;

/// Which antialiasing the chain resolves with.
///
/// Deliberately not a boolean. TAA is the quality tier ADR 0005 plans for, and a boolean would have to
/// be replaced by an enum the moment it landed — along with every setting file that had stored it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Antialiasing {
    /// None. The composite writes straight into the caller's target and no extra pass runs.
    #[default]
    None,
    /// A luma-gated, edge-oriented blend over the tone-mapped image.
    ///
    /// The cheap floor: one fullscreen pass, no extra target beyond the image it reads, and no
    /// architectural cost anywhere else in the chain. See `shaders/antialias.wgsl` for what it
    /// actually does and what it cannot do.
    Fxaa,
}

/// What resolution to render at, and how to resolve it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisplaySettings {
    /// A multiplier on the output resolution.
    ///
    /// Above one this supersamples: every screen-space pass runs at the larger size and the composite
    /// downsamples with a filtered read. That is the only control here that addresses *every* class of
    /// aliasing — geometric, texture, specular, and the noise in the occlusion pass — because it is the
    /// only one that raises the actual sampling rate. Its cost is quadratic, which is an honest
    /// trade-off to put in front of a player rather than a hidden one.
    ///
    /// Sanitised rather than validated: see [`Self::scale`].
    pub resolution_scale: f32,
    /// Which resolve pass runs after the composite.
    pub antialiasing: Antialiasing,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self::NATIVE
    }
}

impl DisplaySettings {
    /// Render at the output resolution with no resolve pass.
    ///
    /// The frame this renderer produced before either setting existed, which is what lets every
    /// committed reference capture stay byte-identical across this change — and so what proves the
    /// plumbing did not quietly alter the image passing through it.
    pub const NATIVE: Self = Self {
        resolution_scale: 1.0,
        antialiasing: Antialiasing::None,
    };

    /// Returns the settings with the resolution scale replaced.
    #[must_use]
    pub const fn at_scale(mut self, resolution_scale: f32) -> Self {
        self.resolution_scale = resolution_scale;
        self
    }

    /// Returns the settings with the antialiasing replaced.
    #[must_use]
    pub const fn with_antialiasing(mut self, antialiasing: Antialiasing) -> Self {
        self.antialiasing = antialiasing;
        self
    }

    /// Returns the resolution scale, clamped into range and with any non-finite value replaced by one.
    ///
    /// Sanitised rather than refused. A settings file is a place a NaN arrives from, and a display
    /// setting is not worth failing a launch over: the honest recovery is to render at the size the
    /// window actually is.
    #[must_use]
    pub fn scale(&self) -> f32 {
        if self.resolution_scale.is_finite() {
            self.resolution_scale
                .clamp(MIN_RESOLUTION_SCALE, MAX_RESOLUTION_SCALE)
        } else {
            1.0
        }
    }

    /// Returns the size every screen-space target is allocated at, for a given output size.
    ///
    /// Each axis rounds independently, so the render aspect can differ from the output aspect by up to
    /// half a pixel on each. That is deliberate: matching the aspect exactly would mean deriving one
    /// axis from the other, which loses a whole row or column at some sizes to save a rounding error
    /// the projection cannot express anyway.
    #[must_use]
    pub fn render_size(&self, width: u32, height: u32) -> (u32, u32) {
        let scale = self.scale();
        (scaled(width, scale), scaled(height, scale))
    }

    /// Whether the composite writes to an intermediate that a further pass reads.
    ///
    /// The one question the chain asks of these settings at record time, so it is answered here rather
    /// than by matching on the enum at the call site.
    #[must_use]
    pub const fn needs_resolve_target(&self) -> bool {
        matches!(self.antialiasing, Antialiasing::Fxaa)
    }
}

/// Scales one dimension, bounded at both ends.
///
/// Both casts are exact for the values that reach them: the input is clamped to `MAX_RENDER_DIMENSION`
/// before it is converted, and 8192 and every integer below it are representable in `f32`; the result
/// is clamped back into the same range before it is converted to an integer, so the truncation cannot
/// wrap and the value cannot be negative.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn scaled(dimension: u32, scale: f32) -> u32 {
    let dimension = dimension.clamp(1, MAX_RENDER_DIMENSION);
    let limit = MAX_RENDER_DIMENSION as f32;
    ((dimension as f32 * scale).round().clamp(1.0, limit)) as u32
}

#[cfg(test)]
mod tests {
    // The comparisons here are against values the clamp produces exactly -- its own two bounds and the
    // literal 1.0 the sanitiser substitutes -- so an exact test is the assertion, not an approximation
    // of one.
    #![allow(clippy::float_cmp)]

    use super::{
        Antialiasing, DisplaySettings, MAX_RENDER_DIMENSION, MAX_RESOLUTION_SCALE,
        MIN_RESOLUTION_SCALE,
    };

    #[test]
    fn the_native_settings_change_nothing() {
        // The property the committed references depend on. If this ever stops holding, every reference
        // in the tree was rendered by a chain that is no longer the default one.
        let native = DisplaySettings::NATIVE;
        assert_eq!(native.render_size(1920, 1080), (1920, 1080));
        assert_eq!(native.antialiasing, Antialiasing::None);
        assert!(!native.needs_resolve_target());
        assert_eq!(DisplaySettings::default(), native);
    }

    #[test]
    fn a_scale_multiplies_both_axes() {
        let doubled = DisplaySettings::NATIVE.at_scale(2.0);
        assert_eq!(doubled.render_size(1280, 720), (2560, 1440));
        let halved = DisplaySettings::NATIVE.at_scale(0.5);
        assert_eq!(halved.render_size(1280, 720), (640, 360));
    }

    #[test]
    fn an_odd_dimension_rounds_rather_than_truncating() {
        // Truncating loses a row at every odd size, and the row it loses is the one at the edge of the
        // frame -- so the symptom is a sliver of the previous frame's contents along one border after
        // the composite stretches the rest over it.
        let halved = DisplaySettings::NATIVE.at_scale(0.5);
        assert_eq!(halved.render_size(721, 481), (361, 241));
    }

    #[test]
    fn a_scale_outside_the_offered_range_is_clamped() {
        assert_eq!(
            DisplaySettings::NATIVE.at_scale(8.0).scale(),
            MAX_RESOLUTION_SCALE
        );
        assert_eq!(
            DisplaySettings::NATIVE.at_scale(0.01).scale(),
            MIN_RESOLUTION_SCALE
        );
        assert_eq!(
            DisplaySettings::NATIVE.at_scale(-3.0).scale(),
            MIN_RESOLUTION_SCALE
        );
    }

    #[test]
    fn a_non_finite_scale_renders_at_the_output_size() {
        // Where a NaN comes from is a settings file, and refusing to launch over a display setting is a
        // worse answer than rendering at the size the window is.
        for broken in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let settings = DisplaySettings::NATIVE.at_scale(broken);
            assert_eq!(settings.scale(), 1.0, "{broken} should sanitise to 1.0");
            assert_eq!(settings.render_size(800, 600), (800, 600));
        }
    }

    #[test]
    fn no_scale_can_ask_for_a_target_the_device_will_not_create() {
        // The clamp that stops a setting turning into a texture-creation failure. `max_texture_dimension_2d`
        // is 8192 by default, and a 5120-wide window at a scale of two would ask for 10240.
        let doubled = DisplaySettings::NATIVE.at_scale(MAX_RESOLUTION_SCALE);
        let (width, height) = doubled.render_size(5_120, 2_880);
        assert_eq!(width, MAX_RENDER_DIMENSION);
        assert_eq!(height, 5_760.min(MAX_RENDER_DIMENSION));
        assert!(height <= MAX_RENDER_DIMENSION);
    }

    #[test]
    fn a_zero_dimension_still_yields_a_drawable_one() {
        // A minimised window reports zero, and target allocation refuses it as an error. This function
        // must not be the thing that turns it into a panic on the way there.
        assert_eq!(DisplaySettings::NATIVE.render_size(0, 0), (1, 1));
    }

    #[test]
    fn only_a_post_pass_needs_an_intermediate() {
        assert!(
            DisplaySettings::NATIVE
                .with_antialiasing(Antialiasing::Fxaa)
                .needs_resolve_target()
        );
        // A scale on its own does not: the composite reads the larger target and writes the caller's,
        // which is the downsample.
        assert!(!DisplaySettings::NATIVE.at_scale(2.0).needs_resolve_target());
    }
}
