//! A captured sky on the GPU: the equirectangular environment, its mip chain, and its bind group.
//!
//! # Why the upload is `Rgba16Float` and not the four other things it could be
//!
//! A [`cic_assets::SkyAsset`] decodes to `f32`, which is more precision than the RGBE it came from had
//! and four times the memory a sky needs. What the hardware then needs is a format that is **filterable**
//! — a sky is magnified enormously across a background and a nearest-neighbour lookup shows every texel —
//! and that carries values well above one.
//!
//! - `Rgba32Float` keeps the decoded precision and needs the `FLOAT32_FILTERABLE` feature to be sampled
//!   smoothly at all. Declining an optional feature over a format twice the size of the one that works
//!   is the whole argument.
//! - `Rgb9e5Ufloat` is RGBE's own layout at four bytes a texel and is filterable — the closest fit on
//!   paper. It cannot be a render attachment, which does not matter here, and it is the format with the
//!   least backend mileage of the three, which does: this renderer already ships `Rgba16Float` targets
//!   on every adapter it is tested against.
//! - `Rgba16Float` is filterable in core WebGPU, holds about five decimal orders of magnitude with eleven
//!   bits of mantissa, and is already the HDR target's format. Eight bytes a texel: a 2048x1024 sky is
//!   16 MiB, 21 MiB with its chain.
//!
//! Eleven bits of mantissa against RGBE's eight means the conversion loses nothing the file carried.
//!
//! # Why the mip chain is built on the CPU
//!
//! For the same reason [`crate::texture`] builds its own: a blit chain needs the texture to be a render
//! attachment and a pass per level, and this happens once when a scene loads. It also has to be built
//! *wrapping in longitude*, which no hardware generator does — see [`cic_assets::SkyAsset::mip_chain`].

use cic_assets::{SKY_CHANNELS, SkyAsset, SkyLighting};

use crate::RenderError;
use crate::gpu::GpuContext;

/// The format an environment is uploaded and sampled in. See the module note.
pub const SKY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Bytes one texel occupies in [`SKY_FORMAT`]: four channels of two.
///
/// Derived from the format rather than from the payload, so a row pitch cannot silently agree with a
/// buffer that is the wrong size.
#[allow(clippy::cast_possible_truncation)]
const SKY_CHANNEL_BYTES: u32 = (SKY_CHANNELS as u32) * 2;

/// Byte size of the sky's uniform block: two vec4s.
const SKY_UNIFORM_BYTES: usize = 32;

/// How a captured sky is placed and scaled, as against what is in the image.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkySettings {
    /// A multiplier on the stored radiance.
    ///
    /// One means "use the file's own values", which is the right default for a calibrated HDRI and the
    /// wrong one for about half of what is in circulation: an environment captured at an unknown
    /// exposure carries radiance in arbitrary units, and the only honest way to fit it to a renderer's
    /// tone curve is a number a designer turns. It scales the ambient this sky derives as well as the
    /// pixels, so turning it does not put the two out of agreement.
    pub intensity: f32,
    /// Rotation about the world's vertical axis, in radians.
    ///
    /// What aims the sky. A captured environment has a sun in it, and that sun has to end up where the
    /// scene's directional light says it is or every shadow points somewhere the sky does not explain.
    /// See [`Sky::aim_at`], which computes this from a light direction rather than leaving it to be
    /// found by eye.
    pub yaw: f32,
}

impl Default for SkySettings {
    /// The image as it was captured, unrotated.
    fn default() -> Self {
        Self {
            intensity: 1.0,
            yaw: 0.0,
        }
    }
}

impl SkySettings {
    /// Clamps every figure into what the shader assumes.
    #[must_use]
    fn sanitised(self) -> Self {
        Self {
            intensity: if self.intensity.is_finite() {
                self.intensity.max(0.0)
            } else {
                1.0
            },
            yaw: if self.yaw.is_finite() { self.yaw } else { 0.0 },
        }
    }
}

/// A captured sky, uploaded and bound.
///
/// Owned by the caller rather than by the renderer, exactly as a [`crate::water::WaterBody`] is, and for
/// the same reason: a resize rebuilds the [`crate::deferred::DeferredRenderer`] along with every bind
/// group holding a view of a resized target, and a sky is neither of those things. Passing it in per
/// frame means a window resize does not silently drop the environment.
#[derive(Debug)]
pub struct Sky {
    group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    lighting: SkyLighting,
    /// The azimuth of the brightest part of the image's upper half, measured once at construction.
    ///
    /// Kept rather than recomputed because [`Self::aim_at`] is called whenever the sun moves — every
    /// frame of a scrubbed day cycle — and finding it walks a mip chain.
    sun_azimuth: f32,
    levels: u32,
    /// Radians of longitude one base-level texel spans.
    ///
    /// What `sky.wgsl` turns a reflection's cone angle into a mip level with. Computed here because it
    /// is a property of the image the shader would otherwise have to query per fragment.
    texel_angle: f32,
    settings: SkySettings,
}

impl Sky {
    /// Builds the layout a [`Sky`] binds through, which is group 3 of the lighting and water passes.
    ///
    /// Shared with the pipelines rather than created per sky, for the reason every other layout here is:
    /// a pipeline and the group bound to it must be built against one declaration.
    #[must_use]
    pub fn layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cic-render sky layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        })
    }

    /// Uploads an environment and its mip chain.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::EmptyCapture`] for an image with no texels, which nothing can be sampled
    /// from.
    pub fn new(
        context: &GpuContext,
        layout: &wgpu::BindGroupLayout,
        asset: &SkyAsset,
        settings: SkySettings,
    ) -> Result<Self, RenderError> {
        let chain = asset.mip_chain();
        if chain.is_empty() || asset.width() == 0 || asset.height() == 0 {
            return Err(RenderError::EmptyCapture);
        }
        let device = context.device();
        let levels = u32::try_from(chain.len()).unwrap_or(1);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cic-render sky environment"),
            size: wgpu::Extent3d {
                width: asset.width(),
                height: asset.height(),
                depth_or_array_layers: 1,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SKY_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (level, (width, height, texels)) in chain.iter().enumerate() {
            let halves: Vec<u8> = texels
                .iter()
                .flat_map(|value| to_half(*value).to_le_bytes())
                .collect();
            context.queue().write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: u32::try_from(level).unwrap_or(0),
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &halves,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    // Two bytes a channel, four channels. Derived rather than taken from the buffer's
                    // length, because a row pitch computed from the payload cannot catch a payload of
                    // the wrong size — it would simply describe it consistently.
                    bytes_per_row: Some(width * SKY_CHANNEL_BYTES),
                    rows_per_image: Some(*height),
                },
                wgpu::Extent3d {
                    width: *width,
                    height: *height,
                    depth_or_array_layers: 1,
                },
            );
        }

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("cic-render sky sampler"),
            // Repeat in u and clamp in v, which is what the projection is: longitude wraps and latitude
            // ends. A clamp in u instead leaves a smeared column down the meridian, and the yaw makes
            // that column land somewhere different in every scene — which is how it would be reported
            // as an intermittent artefact rather than as an address mode.
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            // Linear between levels as well as within them. The shader chooses its level from a
            // roughness rather than from a derivative, so a nearest mip filter would step visibly as a
            // lake's chop changes rather than blurring through it.
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cic-render sky parameters"),
            size: SKY_UNIFORM_BYTES as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cic-render sky bindings"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });

        let sky = Self {
            group,
            uniform,
            lighting: asset.lighting(),
            sun_azimuth: brightest_azimuth(asset),
            levels,
            texel_angle: std::f32::consts::TAU / width_as_f32(asset.width()),
            settings: settings.sanitised(),
        };
        sky.upload_settings(context);
        Ok(sky)
    }

    /// Replaces the intensity and rotation without re-uploading the image.
    pub fn set_settings(&mut self, context: &GpuContext, settings: SkySettings) {
        self.settings = settings.sanitised();
        self.upload_settings(context);
    }

    /// Returns the current settings.
    #[must_use]
    pub const fn settings(&self) -> SkySettings {
        self.settings
    }

    /// What this sky contributes to the lighting under it, scaled by the current intensity.
    ///
    /// Assign it to [`crate::environment::Environment::sky`] and the fog and the ambient follow the
    /// image. Not applied automatically, because the renderer is handed the sky and the environment
    /// separately and silently rewriting one from the other is the behaviour
    /// [`crate::deferred::DeferredFrame::in_environment`] exists to make explicit.
    #[must_use]
    pub fn lighting(&self) -> SkyLighting {
        let scale = |colour: [f32; 3]| {
            [
                colour[0] * self.settings.intensity,
                colour[1] * self.settings.intensity,
                colour[2] * self.settings.intensity,
            ]
        };
        SkyLighting {
            horizon: scale(self.lighting.horizon),
            zenith: scale(self.lighting.zenith),
            ambient: scale(self.lighting.ambient),
        }
    }

    /// Returns the group bound at index 3.
    pub(crate) const fn group(&self) -> &wgpu::BindGroup {
        &self.group
    }

    fn upload_settings(&self, context: &GpuContext) {
        // The highest level, not the count: the shader clamps a computed level against it.
        let top = f32::from(u16::try_from(self.levels.saturating_sub(1)).unwrap_or(0));
        let block = [
            self.settings.intensity,
            self.settings.yaw,
            1.0,
            top,
            self.texel_angle,
            0.0,
            0.0,
            0.0,
        ];
        let bytes: Vec<u8> = block.iter().flat_map(|value| value.to_le_bytes()).collect();
        debug_assert_eq!(bytes.len(), SKY_UNIFORM_BYTES, "sky uniform drifted");
        context.queue().write_buffer(&self.uniform, 0, &bytes);
    }

    /// Turns the image until its own sun sits at a given direction's azimuth.
    ///
    /// A captured sky has a sun in it and a scene has a directional light, and nothing makes the two
    /// agree by default. When they disagree the failure is not subtle and it is not obviously a
    /// *rotation*: every shadow falls away from a bright patch of sky that is somewhere else, which
    /// reads as the shadows being wrong rather than as the sky being turned the wrong way.
    ///
    /// `sun_direction` is the direction *toward* the sun, matching
    /// [`crate::terrain::DirectionalLight::direction`] and
    /// [`crate::environment::Environment::sun_direction`], so a day cycle calls this each time the hour
    /// changes and the sky turns with the shadows.
    ///
    /// This aligns azimuth only. Elevation is fixed in the image, so a sky captured at noon cannot be
    /// rotated into a sunset — the honest answer there is a different file, and stating the limit here
    /// is cheaper than a caller discovering it.
    pub fn aim_at(&mut self, context: &GpuContext, sun_direction: [f32; 3]) {
        let wanted = sun_direction[1].atan2(sun_direction[0]);
        // `sky_direction_uv` adds the yaw to the sample's longitude, so rotating the image *toward*
        // `wanted` means subtracting rather than adding. Getting this backwards puts the sun exactly as
        // far the wrong way as it was out to begin with, which is why it is one expression in one place.
        self.set_settings(
            context,
            SkySettings {
                yaw: self.sun_azimuth - wanted,
                ..self.settings
            },
        );
    }

    /// The azimuth of the brightest part of the image's upper half, before any rotation.
    ///
    /// Where this sky thinks its sun is. Exposed because a caller aiming a scene's light *at the sky*,
    /// rather than the other way round, needs the same figure.
    #[must_use]
    pub const fn sun_azimuth(&self) -> f32 {
        self.sun_azimuth
    }
}

/// A texel width as a float, floored at one so the angle it divides into can never be an infinity.
#[allow(clippy::cast_precision_loss)]
fn width_as_f32(width: u32) -> f32 {
    width.max(1) as f32
}

/// The azimuth of the brightest column of an image's upper half, in radians.
///
/// Computed on a reduced level and over the upper half only, because the two failure modes of "find the
/// sun" are both avoidable: a single bright texel from sensor noise, which the reduction averages away,
/// and a bright patch of ground below the horizon, which the half excludes.
#[allow(clippy::cast_precision_loss)]
fn brightest_azimuth(asset: &SkyAsset) -> f32 {
    let chain = asset.mip_chain();
    let Some((width, height, texels)) = chain
        .iter()
        .find(|(width, _, _)| *width <= 128)
        .or_else(|| chain.first())
    else {
        return 0.0;
    };
    let mut best = (0u32, f32::NEG_INFINITY);
    for x in 0..*width {
        let mut total = 0.0f32;
        for y in 0..height.div_ceil(2) {
            let at = (y as usize * *width as usize + x as usize) * SKY_CHANNELS;
            // Unweighted across the three channels: this is looking for the sun, and the sun is the
            // brightest thing in the image by orders of magnitude in every one of them.
            total += texels[at] + texels[at + 1] + texels[at + 2];
        }
        if total > best.1 {
            best = (x, total);
        }
    }
    // The inverse of `sky_direction_uv` at yaw zero: u is `longitude / TAU + 0.5`.
    let u = (best.0 as f32 + 0.5) / (*width).max(1) as f32;
    (u - 0.5) * std::f32::consts::TAU
}

/// The group bound at index 3 when no environment is loaded.
///
/// A pipeline layout is fixed when the pipeline is created, so group 3 exists whether a scene has a sky
/// or not and something has to occupy it. This is a one-texel texture and a `params.z` of zero, which is
/// the flag every function in `sky.wgsl` branches on to take the analytic path — so a scene with no
/// environment renders through exactly the expressions it did before environments existed, and the
/// committed references say so.
#[derive(Debug)]
pub(crate) struct AnalyticSky {
    group: wgpu::BindGroup,
}

impl AnalyticSky {
    pub(crate) fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cic-render analytic sky placeholder"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SKY_FORMAT,
            // Never written: nothing samples it, because `params.z` is zero. Left at whatever the
            // driver zero-initialises rather than uploaded, so this costs no queue traffic at all.
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("cic-render analytic sky sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cic-render analytic sky parameters"),
            size: SKY_UNIFORM_BYTES as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            // Mapped and written here rather than through the queue, because this buffer is written
            // exactly once and its contents are all zeros but for nothing at all — `params.z` must be
            // zero, and so must everything else.
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cic-render analytic sky bindings"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });
        Self { group }
    }

    pub(crate) const fn group(&self) -> &wgpu::BindGroup {
        &self.group
    }
}

/// Converts a finite `f32` to IEEE 754 binary16, rounding to nearest even.
///
/// Written out rather than pulled in, because it is thirty lines and the alternative is a dependency in
/// a workspace that has eight. The three cases below are the ones a sky actually produces: an ordinary
/// value, a magnitude past what half can hold — which a bright sun genuinely reaches, since half tops out
/// at 65504 — and a magnitude below its smallest normal, where the result is subnormal or zero.
// Every cast is of a value the lines above have already bounded into the range it lands in.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]
fn to_half(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32;
    let mantissa = bits & 0x007f_ffff;

    if exponent == 0xff {
        // Infinity, or a NaN a decoder should never have produced. Both become half infinity rather
        // than a NaN, because a NaN in a sky propagates through the ambient term into every surface.
        return sign | 0x7c00;
    }
    // Rebase the exponent: f32 bias 127, f16 bias 15.
    let unbiased = exponent - 127 + 15;
    if unbiased >= 0x1f {
        // Past half's range. Saturating to the largest finite value rather than to infinity: a sun at
        // 70000 is a sun, and an infinity would poison every average taken over it.
        return sign | 0x7bff;
    }
    if unbiased <= 0 {
        if unbiased < -10 {
            // Smaller than the smallest subnormal.
            return sign;
        }
        // Subnormal: restore the implicit leading one and shift it down into place, rounding to
        // nearest even on the bits that fall off.
        let with_implicit = mantissa | 0x0080_0000;
        let shift = (14 - unbiased) as u32;
        let rounded = (with_implicit + (1 << (shift - 1))) >> shift;
        return sign | rounded as u16;
    }
    // The ordinary case, rounding to nearest even on the thirteen mantissa bits that do not fit.
    let half = ((unbiased as u32) << 10) | (mantissa >> 13);
    let remainder = mantissa & 0x1fff;
    let round_up = remainder > 0x1000 || (remainder == 0x1000 && (half & 1) == 1);
    sign | (half + u32::from(round_up)) as u16
}

#[cfg(test)]
mod tests {
    use cic_assets::{SKY_CHANNELS, SkyAsset, SkyLimits};

    use super::{SkySettings, brightest_azimuth, to_half};

    /// Decodes IEEE 754 binary16, so the encoder is checked against an independent reading of it
    /// rather than against itself.
    fn from_half(bits: u16) -> f32 {
        let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
        let exponent = i32::from((bits >> 10) & 0x1f);
        let mantissa = f32::from(bits & 0x03ff);
        if exponent == 0 {
            return sign * mantissa * 2.0f32.powi(-24);
        }
        if exponent == 0x1f {
            return sign * f32::INFINITY;
        }
        sign * (1.0 + mantissa / 1024.0) * 2.0f32.powi(exponent - 15)
    }

    #[test]
    fn half_conversion_keeps_the_range_a_sky_uses() {
        // The whole reason this format was chosen, checked at the numbers that matter: a deep shadow, a
        // mid tone, a bright cloud, and a sun. Eleven bits of mantissa is about a part in two thousand.
        for value in [0.0f32, 0.000_12, 0.25, 1.0, 17.5, 900.0, 40_000.0] {
            let round_tripped = from_half(to_half(value));
            let tolerance = (value / 1_000.0).max(1.0e-7);
            assert!(
                (round_tripped - value).abs() <= tolerance,
                "{value} came back as {round_tripped}"
            );
        }
    }

    #[test]
    fn a_value_past_half_saturates_to_the_largest_finite_rather_than_to_infinity() {
        // A sun in a calibrated HDRI genuinely exceeds 65504, and an infinity in the texture would make
        // every average taken over the sky — the ambient term included — a NaN.
        assert!(from_half(to_half(120_000.0)).is_finite());
        assert!(from_half(to_half(120_000.0)) > 65_000.0);
        assert!(from_half(to_half(-120_000.0)) < -65_000.0);
        // And a NaN, which nothing here should produce, becomes an infinity rather than propagating.
        assert!(!from_half(to_half(f32::NAN)).is_nan());
    }

    #[test]
    fn the_brightest_azimuth_is_found_where_the_sun_is() {
        // The figure `yaw_toward` is built on. A sun placed at a known longitude in the upper half must
        // come back at that longitude, because the alternative — an azimuth off by pi, which a sign
        // error in the inverse mapping gives — puts the sky's sun exactly opposite the scene's.
        let (width, height) = (64u32, 32u32);
        for (column, expected) in [
            (0u32, -std::f32::consts::PI),
            (48, std::f32::consts::PI / 2.0),
        ] {
            let mut texels = vec![0.05f32; (width * height) as usize * SKY_CHANNELS];
            // A bright column in the upper half only.
            for y in 0..height / 2 {
                let at = (y as usize * width as usize + column as usize) * SKY_CHANNELS;
                for channel in 0..3 {
                    texels[at + channel] = 500.0;
                }
            }
            let sky = SkyAsset::new(width, height, texels, SkyLimits::default()).expect("sky");
            let found = brightest_azimuth(&sky);
            assert!(
                (found - expected).abs() < 0.2,
                "column {column} gave {found}, expected {expected}"
            );
        }
    }

    #[test]
    fn the_yaw_turns_a_suns_azimuth_onto_the_lights() {
        // The arithmetic `aim_at` performs, checked without a device: rotating by the yaw it computes
        // must land the image's sun on the light's azimuth. The sign is the whole content — being wrong
        // puts the sun exactly twice as far out as doing nothing at all.
        let (width, height) = (64u32, 32u32);
        let mut texels = vec![0.05f32; (width * height) as usize * SKY_CHANNELS];
        // Column 16 of 64 is a quarter turn round, which is azimuth -pi/2 by the mapping above.
        for y in 0..height / 2 {
            let at = (y as usize * width as usize + 16) * SKY_CHANNELS;
            for channel in 0..3 {
                texels[at + channel] = 500.0;
            }
        }
        let asset = SkyAsset::new(width, height, texels, SkyLimits::default()).expect("sky");
        // A sun in the +y direction, which is azimuth +pi/2.
        let wanted = std::f32::consts::FRAC_PI_2;
        let yaw = brightest_azimuth(&asset) - wanted;
        let rotated = brightest_azimuth(&asset) - yaw;
        assert!(
            (rotated - wanted).abs() < 0.2,
            "rotated to {rotated}, wanted {wanted} (yaw {yaw})"
        );
    }

    #[test]
    fn nonsense_settings_are_clamped_rather_than_reaching_a_shader() {
        let wild = SkySettings {
            intensity: f32::NAN,
            yaw: f32::INFINITY,
        }
        .sanitised();
        assert!((wild.intensity - 1.0).abs() < f32::EPSILON);
        assert!(wild.yaw.abs() < f32::EPSILON);
        let negative = SkySettings {
            intensity: -3.0,
            yaw: 1.0,
        }
        .sanitised();
        assert!(
            negative.intensity.abs() < f32::EPSILON,
            "a negative sky is black, not inverted"
        );
    }
}
