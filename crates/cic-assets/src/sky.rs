//! Equirectangular high-dynamic-range sky images, read from Radiance `.hdr`.
//!
//! # Why a separate format from every other texture
//!
//! [`crate::texture`] reads block-compressed DDS, and every slot it serves is eight bits per channel.
//! That is the right trade for a surface: an albedo is a reflectance, bounded in `0..=1` by physics, so
//! the range an integer format offers is the range the data has.
//!
//! A sky is not a reflectance. It is *radiance*, and the ratio between the brightest and the dimmest
//! part of a real one is four or five orders of magnitude — the sun's disc against the shadowed side of
//! a cloud. Quantising that to 256 steps does not merely lose the sun; it loses the whole reason to
//! photograph a sky rather than paint one, because everything a captured environment contributes to a
//! scene — the colour of the bounce, the brightness of a reflection, the shape of a highlight — lives in
//! the part of the range an 8-bit encoding throws away. A tone-mapped sky reflected in water reads as a
//! grey card.
//!
//! So this is a second, small, deliberately separate reader rather than a sixth `BlockFormat`. BC6H is
//! the block format that *would* have fit, and it is declined for a reason recorded in
//! [ADR 4001](../../../docs/adr/4001-hdri-sky.md): writing an encoder for it is a project, and a sky is
//! one image per scene rather than the hundreds of surface textures block compression exists to pay for.
//!
//! # Why Radiance `.hdr` rather than `OpenEXR`
//!
//! Every HDRI a content author will find is distributed as `.hdr`. The format is a text header, a
//! resolution line, and run-length-encoded RGBE scanlines — about two hundred lines to read, with no
//! dependency. `OpenEXR` is the better format and needs a library, a tile reader, and three compression
//! codecs before it decodes its first pixel.
//!
//! RGBE is also a *good* fit rather than merely a convenient one: it is four bytes per texel for a
//! shared exponent across the three channels, which is roughly the precision the eye can use from a sky
//! and a quarter of what three half floats would cost to read.
//!
//! # Provenance
//!
//! Written from the published format description. The tests build their fixtures arithmetically rather
//! than committing a captured environment, so nothing here carries a licence from elsewhere — see
//! [LICENSING.md](../../../LICENSING.md).
//!
//! Like every other decoder in this crate, this one is bounded and total: explicit limits, a structured
//! error naming what it found and what it expected, and no panic on hostile input.

use std::error::Error;
use std::f32::consts::PI;
use std::fmt::{self, Display, Formatter};

use crate::image::halve;

/// Channels per texel in a decoded sky: RGB and a constant alpha.
///
/// Four rather than three so the reductions in [`crate::image`] operate on the same stride they do
/// everywhere else, and so an upload never has to repack rows. The alpha is always one and nothing
/// reads it.
pub const SKY_CHANNELS: usize = 4;

/// The band, in radians either side of the horizon, that [`SkyAsset::lighting`] averages for its
/// horizon colour.
///
/// Five degrees. Wide enough that a single row of a small image is never the whole answer, narrow
/// enough that a bright sun twenty degrees up does not colour it.
const HORIZON_BAND: f32 = 0.087;

/// The cap, in radians from straight up, that the same function averages for its zenith colour.
const ZENITH_CAP: f32 = 0.262;

/// How many times the sky's own mean a texel may exceed before it is treated as the sun.
///
/// # Why this is here at all
///
/// A calibrated HDRI's sun disc is four or five orders of magnitude brighter than the sky around it and
/// covers about a five-thousandth of the hemisphere — which works out to *most of the irradiance*. That
/// is physically correct and it is the wrong number to hand this renderer, which already has a
/// directional light standing for the sun. Adding the measured irradiance on top of it counts the sun
/// twice, and the visible result is a scene with no shadow contrast at all: the ambient is as strong as
/// the beam, so shade and sunlight are the same brightness.
///
/// So the figures below are integrated over a sky with the sun *clamped out* of it, which is a
/// statement about the division of labour rather than about the image. The ceiling is relative to the
/// image's own mean rather than absolute, so it means the same thing for a file in physical units and
/// for one in arbitrary ones.
///
/// Eight is chosen to sit clear of both ends. A bright cumulus edge runs two to four times the mean and
/// must survive; a sun runs thousands and must not. Nothing in an overcast sky reaches eight, which is
/// the case where clamping would be felt if it were wrong — and there, this changes nothing at all.
const SUN_CLAMP: f32 = 8.0;

/// Explicit bounds applied while reading an untrusted sky image.
///
/// # Why one of these reduces rather than refuses
///
/// Every other decoder in this crate refuses what crosses a bound, and that is the right rule for a
/// mesh or a texture, where nothing sensible can be done with half of one. A sky is the case where it
/// is the wrong rule, for a reason particular to the content: HDRIs are distributed at 8K by
/// convention, and 8K is *more resolution than a sky can use*. One texel then covers half a pixel at
/// the horizon, and it costs 358 MiB of video memory for a picture that is out of focus anyway.
///
/// So the size a sky is *read at* is a target rather than a limit, and an image above it is box
/// filtered down while it is decoded — scanline by scanline, so the oversized buffer is never
/// allocated at all. An 8192x4096 file peaks at about 34 MiB rather than 536 MiB and yields the same
/// picture. [`Self::maximum_dimension`] is still a refusal, set far above anything real, because a
/// hostile file declaring 200000 texels wide is a different thing from a large one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkyLimits {
    /// Texels along the longest axis above which the image is reduced as it is read.
    ///
    /// Reduced by a whole power of two, so the filter is a clean box over 2x2, 4x4 or 16x16 blocks and
    /// no resampling weights are involved. That means the result may sit below this rather than on it —
    /// an 8192-wide image at a target of 2048 reduces by four to exactly 2048, and a 6000-wide one
    /// reduces by four to 1500 rather than being stretched to fit.
    pub target_dimension: u32,
    /// Texels along either axis above which the image is refused outright.
    pub maximum_dimension: u32,
    /// Maximum encoded bytes read from the container.
    pub maximum_bytes: usize,
}

impl Default for SkyLimits {
    fn default() -> Self {
        Self {
            // 2048x1024, which is what a sky is worth. At a 4K background one texel covers about two
            // pixels at the horizon and fewer higher up, and the horizon is where an equirectangular
            // image's texels are densest and where a ground-level camera looks.
            target_dimension: 2_048,
            // Twice the largest axis any content ships at, so this refuses a declaration rather than a
            // picture.
            maximum_dimension: 16_384,
            // An 8192x4096 Radiance file with no run-length compression is 128 MiB exactly, and several
            // tools write one — GIMP's exporter among them. A bound that refused the ordinary output of
            // an ordinary tool would be a bound nobody could keep.
            maximum_bytes: 256 * 1_024 * 1_024,
        }
    }
}

/// What a sky contributes to the lighting of everything under it.
///
/// Three colours, derived from the image on the CPU at load, and the reason they exist is
/// [ADR 0006](../../../docs/adr/0006-atmosphere.md)'s central argument: the sky, the fog it fades into,
/// and the ambient it bounces down are one thing seen three ways, and a renderer that lets them be
/// authored separately is a renderer where an orange sunset sits above blue-grey shade and nothing
/// reports it. Binding an HDRI would create exactly that disagreement, because the two shader constants
/// the fog colour and the ambient were derived from no longer describe the sky on screen.
///
/// So they are re-derived from the image instead. This is not image-based lighting — there is no
/// specular prefilter and no irradiance in any direction but up — and it is not meant to be. It is the
/// three numbers the existing model already consumed, taken from the new source of truth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkyLighting {
    /// Mean radiance in a narrow band around the horizon, which is what fog fades toward.
    pub horizon: [f32; 3],
    /// Mean radiance in a cap around straight up.
    pub zenith: [f32; 3],
    /// Cosine-weighted mean radiance over the upper hemisphere.
    ///
    /// Equal to the irradiance on an upward-facing surface divided by pi, which is exactly the quantity
    /// [`crate::sky`]'s consumers already call an ambient term: multiply it by an albedo and you have
    /// the light a flat, unoccluded, perfectly diffuse patch of ground returns. For a uniform sky it is
    /// the sky's own radiance, so the two definitions agree where they overlap.
    pub ambient: [f32; 3],
}

/// One equirectangular sky image, decoded to linear radiance.
///
/// **Equirectangular, not a cube map.** Latitude and longitude map to the two axes directly, so the
/// image is what every HDRI is distributed as and the shader lookup is two inverse trigonometric calls
/// rather than a face selection. The cost is the pole distortion an equirectangular projection has —
/// which matters least for a sky, where the zenith is a slowly varying gradient and the horizon, where
/// the texels are densest, is what a ground-level camera actually looks at.
#[derive(Debug, Clone, PartialEq)]
pub struct SkyAsset {
    width: u32,
    height: u32,
    /// Row-major from the *top* — the direction straight up — four channels per texel.
    texels: Vec<f32>,
}

impl SkyAsset {
    /// Wraps decoded radiance, checking it against the dimensions it claims.
    ///
    /// # Errors
    ///
    /// Returns [`SkyError::EmptyDimension`] for a zero axis, [`SkyError::LimitExceeded`] against
    /// `limits`, or [`SkyError::TexelCountMismatch`] when the buffer is not
    /// `width * height * SKY_CHANNELS` long.
    pub fn new(
        width: u32,
        height: u32,
        texels: Vec<f32>,
        limits: SkyLimits,
    ) -> Result<Self, SkyError> {
        check_dimensions(width, height, limits)?;
        let expected = texel_count(width, height)
            .and_then(|count| count.checked_mul(SKY_CHANNELS))
            .ok_or(SkyError::LimitExceeded {
                what: "sky texels",
                actual: usize::MAX,
                maximum: usize::MAX,
            })?;
        if texels.len() != expected {
            return Err(SkyError::TexelCountMismatch {
                expected,
                actual: texels.len(),
            });
        }
        Ok(Self {
            width,
            height,
            texels,
        })
    }

    /// A sky of one radiance everywhere.
    ///
    /// The fixture an equirectangular test wants first, and the one case where every derived figure has
    /// an answer known in advance: a uniform sky's horizon, zenith and ambient are all its own radiance,
    /// whatever the projection does to the texel areas. That makes it the check on the weighting in
    /// [`Self::lighting`] rather than merely a convenient shape.
    ///
    /// # Errors
    ///
    /// As [`Self::new`].
    pub fn uniform(
        width: u32,
        height: u32,
        radiance: [f32; 3],
        limits: SkyLimits,
    ) -> Result<Self, SkyError> {
        check_dimensions(width, height, limits)?;
        let count = texel_count(width, height).ok_or(SkyError::LimitExceeded {
            what: "sky texels",
            actual: usize::MAX,
            maximum: usize::MAX,
        })?;
        let texel = [radiance[0], radiance[1], radiance[2], 1.0];
        Self::new(width, height, texel.repeat(count), limits)
    }

    /// Returns the width in texels, which spans a full turn of longitude.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the height in texels, which spans zenith to nadir.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Returns the base level's radiance, row-major from the top, four channels per texel.
    #[must_use]
    pub fn texels(&self) -> &[f32] {
        &self.texels
    }

    /// Returns one texel's linear radiance, or `None` outside the image.
    #[must_use]
    pub fn texel(&self, x: u32, y: u32) -> Option<[f32; 3]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let at = (y as usize * self.width as usize + x as usize) * SKY_CHANNELS;
        Some([self.texels[at], self.texels[at + 1], self.texels[at + 2]])
    }

    /// Builds the full mip chain, as `(width, height, radiance)` per level from the base down to 1x1.
    ///
    /// **Wrapped in longitude, clamped in latitude**, which a general-purpose reduction cannot be. The
    /// left and right columns of an equirectangular image are adjacent directions, so averaging the last
    /// column against itself — what a clamping reduction does — puts a seam of wrong texels down the
    /// meridian, and every level makes it wider. The poles genuinely have an edge, so those clamp.
    ///
    /// Averaged in linear light, which needs saying only because [`crate::image::mip_chain`] has to
    /// argue for it: radiance already *is* linear light, so there is no transfer function to undo.
    #[must_use]
    pub fn mip_chain(&self) -> Vec<(u32, u32, Vec<f32>)> {
        let levels = crate::image::mip_level_count(self.width, self.height);
        let mut chain = Vec::with_capacity(levels as usize);
        let mut level = (self.width, self.height, self.texels.clone());
        for _ in 1..levels {
            let (width, height, texels) = &level;
            let reduced = reduce_equirect(texels, *width, *height);
            let next = (halve(*width), halve(*height), reduced);
            chain.push(std::mem::replace(&mut level, next));
        }
        chain.push(level);
        chain
    }

    /// Derives what this sky contributes to the lighting under it.
    ///
    /// Computed on a reduced level rather than the base, because every figure here is an average over
    /// thousands of texels and a mip level *is* that average already computed. A 2048-wide image costs
    /// one pass over a 64-wide one, and the answers agree to well under a part in a thousand — the
    /// reduction is area-weighted and so is the integral.
    ///
    /// The sun is clamped out of all three figures. See [`SUN_CLAMP`], which is where the reasoning is.
    #[must_use]
    pub fn lighting(&self) -> SkyLighting {
        // Reduce until the image is small enough to sum cheaply, but never below the resolution at
        // which the horizon band and the zenith cap each still contain a row.
        let chain = self.mip_chain();
        let fallback = (self.width, self.height, self.texels.clone());
        let (width, height, texels) = chain
            .iter()
            .find(|(width, height, _)| *width <= 64 && *height >= 16)
            .or_else(|| chain.first())
            .unwrap_or(&fallback);
        let (width, height) = (*width, *height);

        // Two passes, because the ceiling is relative to the answer. The first is the ordinary
        // cosine-weighted mean, taken per *texel* rather than per row — a row mean would have averaged
        // the sun into its neighbours before anything could recognise it as one.
        let mut rough = Accumulator::default();
        for_each_texel(texels, width, height, |colour, _, area, up| {
            if up > 0.0 {
                rough.add(colour, area * up);
            }
        });
        let ceiling = luminance(rough.mean_or(|| [0.0; 3])) * SUN_CLAMP;

        let mut horizon = Accumulator::default();
        let mut zenith = Accumulator::default();
        let mut ambient = Accumulator::default();
        for_each_texel(texels, width, height, |colour, theta, area, up| {
            // Scaled rather than clipped per channel, so clamping a warm sun leaves it warm instead of
            // pulling it toward white on the way down.
            let level = luminance(colour);
            let colour = if ceiling > 0.0 && level > ceiling {
                let scale = ceiling / level;
                [colour[0] * scale, colour[1] * scale, colour[2] * scale]
            } else {
                colour
            };
            if (theta - PI / 2.0).abs() <= HORIZON_BAND {
                horizon.add(colour, area);
            }
            if theta <= ZENITH_CAP {
                zenith.add(colour, area);
            }
            if up > 0.0 {
                // Cosine *and* area weighted, which together integrate to pi over the hemisphere — so
                // dividing by the accumulated weight yields the irradiance over pi directly.
                ambient.add(colour, area * up);
            }
        });

        SkyLighting {
            // Each falls back to the row nearest what it wanted, so a degenerate image still yields a
            // colour rather than a division by zero. A one-row sky has a horizon.
            horizon: horizon.mean_or(|| row_mean(texels, width, height / 2)),
            zenith: zenith.mean_or(|| row_mean(texels, width, 0)),
            ambient: ambient.mean_or(|| row_mean(texels, width, 0)),
        }
    }
}

/// Walks every texel with the geometry its position implies: polar angle, solid-angle weight, and the
/// cosine against vertical.
///
/// One walker for both passes of [`SkyAsset::lighting`], because the two must agree about the weighting
/// exactly — the second pass's ceiling is derived from the first pass's answer, and a discrepancy
/// between them would be a systematic bias rather than a visible fault.
fn for_each_texel(
    texels: &[f32],
    width: u32,
    height: u32,
    mut visit: impl FnMut([f32; 3], f32, f32, f32),
) {
    for y in 0..height {
        let theta = latitude(y, height);
        // The solid angle a row subtends shrinks toward the poles as `sin(theta)`. Omitting it is the
        // classic equirectangular mistake: the pole rows hold as many texels as the equator and cover
        // almost no sky, so an unweighted mean is dominated by whatever is directly overhead.
        let area = theta.sin();
        let up = theta.cos();
        for x in 0..width {
            let at = (y as usize * width as usize + x as usize) * SKY_CHANNELS;
            let Some(texel) = texels.get(at..at + 3) else {
                continue;
            };
            visit([texel[0], texel[1], texel[2]], theta, area, up);
        }
    }
}

/// Relative luminance, for deciding what counts as the sun.
fn luminance(colour: [f32; 3]) -> f32 {
    0.2126f32.mul_add(colour[0], 0.7152f32.mul_add(colour[1], 0.0722 * colour[2]))
}

/// Sums colours against weights, so each derived figure states its own weighting once.
#[derive(Debug, Default, Clone, Copy)]
struct Accumulator {
    total: [f32; 3],
    weight: f32,
}

impl Accumulator {
    fn add(&mut self, colour: [f32; 3], weight: f32) {
        for (total, value) in self.total.iter_mut().zip(colour) {
            *total += value * weight;
        }
        self.weight += weight;
    }

    fn mean_or(self, fallback: impl FnOnce() -> [f32; 3]) -> [f32; 3] {
        if self.weight <= 0.0 {
            return fallback();
        }
        [
            self.total[0] / self.weight,
            self.total[1] / self.weight,
            self.total[2] / self.weight,
        ]
    }
}

/// The polar angle of a row's centre, zero straight up and pi straight down.
#[allow(clippy::cast_precision_loss)]
fn latitude(y: u32, height: u32) -> f32 {
    (y as f32 + 0.5) / height.max(1) as f32 * PI
}

/// The mean radiance of one row, or black for a row outside the image.
#[allow(clippy::cast_precision_loss)]
fn row_mean(texels: &[f32], width: u32, y: u32) -> [f32; 3] {
    let start = y as usize * width as usize * SKY_CHANNELS;
    let end = start + width as usize * SKY_CHANNELS;
    let Some(row) = texels.get(start..end) else {
        return [0.0; 3];
    };
    let mut total = [0.0f32; 3];
    for texel in row.chunks_exact(SKY_CHANNELS) {
        for channel in 0..3 {
            total[channel] += texel[channel];
        }
    }
    let count = width.max(1) as f32;
    [total[0] / count, total[1] / count, total[2] / count]
}

/// Reduces an equirectangular level by half, wrapping in longitude and clamping in latitude.
fn reduce_equirect(source: &[f32], width: u32, height: u32) -> Vec<f32> {
    let (next_width, next_height) = (halve(width), halve(height));
    let mut output = Vec::with_capacity(next_width as usize * next_height as usize * SKY_CHANNELS);
    let last_y = height.saturating_sub(1);
    for y in 0..next_height {
        let y0 = (y * 2).min(last_y);
        let y1 = (y * 2 + 1).min(last_y);
        for x in 0..next_width {
            // Modulo rather than a clamp: column `width - 1` neighbours column 0, because the two are
            // adjacent directions rather than opposite edges of a page.
            let x0 = (x * 2) % width;
            let x1 = (x * 2 + 1) % width;
            for channel in 0..SKY_CHANNELS {
                let at = |x: u32, y: u32| {
                    source[(y as usize * width as usize + x as usize) * SKY_CHANNELS + channel]
                };
                output.push((at(x0, y0) + at(x1, y0) + at(x0, y1) + at(x1, y1)) * 0.25);
            }
        }
    }
    output
}

fn texel_count(width: u32, height: u32) -> Option<usize> {
    usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)
}

fn check_dimensions(width: u32, height: u32, limits: SkyLimits) -> Result<(), SkyError> {
    if width == 0 || height == 0 {
        return Err(SkyError::EmptyDimension { width, height });
    }
    if width > limits.maximum_dimension || height > limits.maximum_dimension {
        return Err(SkyError::LimitExceeded {
            what: "sky dimension",
            actual: usize::try_from(width.max(height)).unwrap_or(usize::MAX),
            maximum: usize::try_from(limits.maximum_dimension).unwrap_or(usize::MAX),
        });
    }
    texel_count(width, height).ok_or(SkyError::LimitExceeded {
        what: "sky texels",
        actual: usize::MAX,
        maximum: usize::MAX,
    })?;
    Ok(())
}

/// How many source texels along each axis become one, to bring an image to the target size.
///
/// A power of two, so the filter is a clean box and the two axes reduce by the same factor — which they
/// must, or an equirectangular image's latitude and longitude would no longer be in proportion and the
/// sky would be stretched.
fn reduction_for(width: u32, height: u32, limits: SkyLimits) -> u32 {
    let target = limits.target_dimension.max(1);
    let mut factor = 1u32;
    while width.div_ceil(factor) > target || height.div_ceil(factor) > target {
        // Bounded by `maximum_dimension`, so this terminates well before the shift would overflow.
        factor *= 2;
    }
    factor
}

/// Reads a Radiance `.hdr` image into linear radiance.
///
/// Accepts both scanline encodings the format has — the adaptive run-length one every modern writer
/// produces, and the flat four-bytes-per-texel form — plus the original run-length records, because
/// files carrying them are still in circulation and the alternative is decoding one as garbage.
///
/// # What is refused, and why refusing is right
///
/// A `-Y ... +X ...` resolution line is the only orientation read. The format permits eight, and the
/// other seven are rotations and flips no HDRI is distributed in. Reading one *wrong* would put the sky
/// on the ground, which is a bug an author would report as a rendering fault rather than as a bad file;
/// naming the orientation found is what makes it a five-second answer.
///
/// The `32-bit_rle_xyze` colour space is likewise refused by name rather than read as RGB. Its channels
/// are CIE tristimulus values, so treating them as red, green and blue produces a picture that is
/// plausible, wrongly coloured, and impossible to diagnose from the image alone.
///
/// # Errors
///
/// Returns a structured [`SkyError`] when the signature, header, resolution line or scanline data is
/// not what the format specifies, or when a [`SkyLimits`] bound is crossed. Every bound is checked
/// before the allocation it governs.
pub fn decode_radiance(bytes: &[u8], limits: SkyLimits) -> Result<SkyAsset, SkyError> {
    if bytes.len() > limits.maximum_bytes {
        return Err(SkyError::LimitExceeded {
            what: "sky file bytes",
            actual: bytes.len(),
            maximum: limits.maximum_bytes,
        });
    }
    // `#?RADIANCE` is what the reference writer emits and `#?RGBE` what several others do, so the
    // signature is the two bytes both share plus whatever identifier follows.
    if !bytes.starts_with(b"#?") {
        return Err(SkyError::NotRadiance);
    }

    let mut cursor = 0usize;
    let mut exposure = 1.0f32;
    let mut format_seen = false;
    loop {
        let line = read_line(bytes, &mut cursor).ok_or(SkyError::TruncatedHeader)?;
        // The header ends at the first empty line, and the resolution line follows it.
        if line.is_empty() {
            break;
        }
        let text = String::from_utf8_lossy(line);
        let text = text.trim();
        if let Some(value) = text.strip_prefix("FORMAT=") {
            let value = value.trim();
            if value != "32-bit_rle_rgbe" {
                return Err(SkyError::UnsupportedColourFormat {
                    format: value.to_owned(),
                });
            }
            format_seen = true;
        } else if let Some(value) = text.strip_prefix("EXPOSURE=") {
            // The specification says exposures multiply when a file has been through several tools, and
            // that a reader wanting the original radiance divides the product out. A sky is used *as*
            // radiance here — it drives the ambient term — so undoing the exposure is the difference
            // between a physically meaningful figure and one scaled by whoever last touched the file.
            if let Ok(value) = value.trim().parse::<f32>()
                && value.is_finite()
                && value > 0.0
            {
                exposure *= value;
            }
        }
    }
    if !format_seen {
        return Err(SkyError::MissingFormat);
    }

    let resolution = read_line(bytes, &mut cursor).ok_or(SkyError::TruncatedHeader)?;
    let (width, height) = parse_resolution(resolution)?;
    check_dimensions(width, height, limits)?;

    // The size the image is *kept* at, which is the target rather than what the file declares. Every
    // allocation below is against this, so an 8K file never has an 8K buffer made for it.
    let factor = reduction_for(width, height, limits);
    let (kept_width, kept_height) = (width.div_ceil(factor), height.div_ceil(factor));
    let count = texel_count(kept_width, kept_height).ok_or(SkyError::LimitExceeded {
        what: "sky texels",
        actual: usize::MAX,
        maximum: usize::MAX,
    })?;

    let scale = 1.0 / exposure;
    let mut texels = vec![0.0f32; count * SKY_CHANNELS];
    let mut scanline = vec![[0u8; 4]; width as usize];
    // How many source texels landed in each kept one. Counted rather than assumed, because the last
    // block of a dimension that is not a multiple of the factor is short — and dividing a short block
    // by the full factor would darken the final row and column.
    let mut contributions = vec![0u32; kept_width as usize];
    for y in 0..height {
        read_scanline(bytes, &mut cursor, width, &mut scanline)
            .map_err(|kind| SkyError::Scanline { row: y, what: kind })?;
        let kept_y = y / factor;
        let row = kept_y as usize * kept_width as usize * SKY_CHANNELS;
        if y % factor == 0 {
            contributions.fill(0);
        }
        for (x, rgbe) in scanline.iter().enumerate() {
            let [r, g, b] = rgbe_to_radiance(*rgbe);
            let at = row + (x / factor as usize) * SKY_CHANNELS;
            texels[at] += r * scale;
            texels[at + 1] += g * scale;
            texels[at + 2] += b * scale;
            contributions[x / factor as usize] += 1;
        }
        // Divide once the last source row of this block has been added, so a partial block at the
        // bottom edge is divided by what it actually received.
        let last_of_block = y + 1 == height || (y + 1) / factor != kept_y;
        if last_of_block {
            for (x, received) in contributions.iter().enumerate() {
                let at = row + x * SKY_CHANNELS;
                let divisor = f32::from(u16::try_from(*received).unwrap_or(1)).max(1.0);
                texels[at] /= divisor;
                texels[at + 1] /= divisor;
                texels[at + 2] /= divisor;
                texels[at + 3] = 1.0;
            }
        }
    }

    SkyAsset::new(kept_width, kept_height, texels, limits)
}

/// One RGBE quadruple as linear radiance.
///
/// The mantissas are offset by a half step before scaling, which is Radiance's own reconstruction: the
/// stored byte is the floor of the true value in units of the exponent's step, so the midpoint of the
/// interval it represents is half a step above it. Omitting the half biases every texel downward by
/// about a fifth of a percent — invisible in an image and not invisible in an ambient term derived by
/// averaging a million of them.
#[allow(clippy::cast_precision_loss)]
fn rgbe_to_radiance(rgbe: [u8; 4]) -> [f32; 3] {
    if rgbe[3] == 0 {
        // The format's own encoding of black. There is no exponent small enough to mean zero otherwise,
        // since the half step above would still be positive.
        return [0.0; 3];
    }
    // Bounded to -136..=127 by the byte it comes from, which `f32` represents exactly.
    let exponent = i32::from(rgbe[3]) - (128 + 8);
    let step = (exponent as f32).exp2();
    [
        (f32::from(rgbe[0]) + 0.5) * step,
        (f32::from(rgbe[1]) + 0.5) * step,
        (f32::from(rgbe[2]) + 0.5) * step,
    ]
}

/// Reads one newline-terminated line, advancing the cursor past the newline.
///
/// Returns the line without its terminator, or `None` at the end of the input — which in a header is a
/// truncation rather than an end, since the header is required to be followed by more.
fn read_line<'a>(bytes: &'a [u8], cursor: &mut usize) -> Option<&'a [u8]> {
    if *cursor >= bytes.len() {
        return None;
    }
    let rest = &bytes[*cursor..];
    let end = rest.iter().position(|byte| *byte == b'\n')?;
    *cursor += end + 1;
    // A file written on Windows may carry the carriage return the format does not ask for.
    Some(rest[..end].strip_suffix(b"\r").unwrap_or(&rest[..end]))
}

/// Parses the resolution line, which is the one line of the container that is not free text.
fn parse_resolution(line: &[u8]) -> Result<(u32, u32), SkyError> {
    let text = String::from_utf8_lossy(line);
    let fields: Vec<&str> = text.split_whitespace().collect();
    let [vertical, height, horizontal, width] = fields.as_slice() else {
        return Err(SkyError::MalformedResolution {
            line: text.trim().to_owned(),
        });
    };
    if *vertical != "-Y" || *horizontal != "+X" {
        return Err(SkyError::UnsupportedOrientation {
            orientation: format!("{vertical} .. {horizontal} .."),
        });
    }
    let parsed = height.parse::<u32>().ok().zip(width.parse::<u32>().ok());
    let Some((height, width)) = parsed else {
        return Err(SkyError::MalformedResolution {
            line: text.trim().to_owned(),
        });
    };
    Ok((width, height))
}

/// Why one scanline could not be read, as the part of the message that names the cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanlineFault {
    /// The file ended before the scanline was complete.
    Truncated,
    /// An adaptive run-length header declared a width other than the image's.
    WidthMismatch,
    /// A run-length record described more texels than the scanline has room for.
    Overrun,
    /// A repeat record appeared before any texel it could repeat.
    RepeatWithoutPredecessor,
}

impl Display for ScanlineFault {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Truncated => "the file ends mid-scanline",
            Self::WidthMismatch => "its run-length header declares a different width",
            Self::Overrun => "a run reaches past the end of the row",
            Self::RepeatWithoutPredecessor => {
                "it opens with a repeat of a texel that does not exist"
            }
        })
    }
}

/// Reads one scanline into `out`, in whichever of the three encodings it uses.
fn read_scanline(
    bytes: &[u8],
    cursor: &mut usize,
    width: u32,
    out: &mut [[u8; 4]],
) -> Result<(), ScanlineFault> {
    let first = take(bytes, cursor, 4).ok_or(ScanlineFault::Truncated)?;
    let header = [first[0], first[1], first[2], first[3]];

    // The adaptive encoding announces itself with a 2,2 marker and its own width in the last two bytes.
    // A width outside 8..=0x7fff is the format's own signal that this is *not* the adaptive form —
    // which is how a flat image whose first texel happens to be (2, 2, ...) stays readable.
    let declared = (u32::from(header[2]) << 8) | u32::from(header[3]);
    if header[0] == 2 && header[1] == 2 && (8..=0x7fff).contains(&declared) {
        if declared != width {
            return Err(ScanlineFault::WidthMismatch);
        }
        return read_adaptive_scanline(bytes, cursor, width, out);
    }

    // Otherwise the four bytes just read are the first texel, and the rest of the row follows in the
    // original encoding: literal texels, with (1, 1, 1, n) repeating the previous one.
    read_flat_scanline(bytes, cursor, header, out)
}

/// The adaptive run-length encoding: four component planes, each run-length coded independently.
fn read_adaptive_scanline(
    bytes: &[u8],
    cursor: &mut usize,
    width: u32,
    out: &mut [[u8; 4]],
) -> Result<(), ScanlineFault> {
    for channel in 0..4 {
        let mut written = 0usize;
        while written < width as usize {
            let control = *take(bytes, cursor, 1)
                .ok_or(ScanlineFault::Truncated)?
                .first()
                .ok_or(ScanlineFault::Truncated)?;
            if control > 128 {
                // A run: the count is the excess over 128, and one byte follows for all of it.
                let run = control as usize - 128;
                let value = *take(bytes, cursor, 1)
                    .ok_or(ScanlineFault::Truncated)?
                    .first()
                    .ok_or(ScanlineFault::Truncated)?;
                let end = written.checked_add(run).ok_or(ScanlineFault::Overrun)?;
                let slice = out.get_mut(written..end).ok_or(ScanlineFault::Overrun)?;
                for texel in slice {
                    texel[channel] = value;
                }
                written = end;
            } else {
                // A literal span. A zero count would make no progress and is the shape a corrupt file
                // takes when it loops forever, so it is refused rather than skipped.
                let run = control as usize;
                if run == 0 {
                    return Err(ScanlineFault::Overrun);
                }
                let end = written.checked_add(run).ok_or(ScanlineFault::Overrun)?;
                let values = take(bytes, cursor, run).ok_or(ScanlineFault::Truncated)?;
                let slice = out.get_mut(written..end).ok_or(ScanlineFault::Overrun)?;
                for (texel, value) in slice.iter_mut().zip(values) {
                    texel[channel] = *value;
                }
                written = end;
            }
        }
    }
    Ok(())
}

/// The original encoding: literal RGBE texels, with `(1, 1, 1, n)` repeating the previous one.
///
/// Consecutive repeat records shift their counts by a further eight bits each, which is how the format
/// expresses a run longer than 255 — and is the detail a reader that treats each record independently
/// gets wrong, producing a row that is subtly too short and a whole image sheared by one texel per
/// scanline.
fn read_flat_scanline(
    bytes: &[u8],
    cursor: &mut usize,
    first: [u8; 4],
    out: &mut [[u8; 4]],
) -> Result<(), ScanlineFault> {
    let mut written = 0usize;
    let mut texel = first;
    let mut shift = 0u32;
    let mut previous: Option<[u8; 4]> = None;
    loop {
        if texel[0] == 1 && texel[1] == 1 && texel[2] == 1 {
            let source = previous.ok_or(ScanlineFault::RepeatWithoutPredecessor)?;
            let run = (usize::from(texel[3])) << shift;
            let end = written.checked_add(run).ok_or(ScanlineFault::Overrun)?;
            let slice = out.get_mut(written..end).ok_or(ScanlineFault::Overrun)?;
            for entry in slice {
                *entry = source;
            }
            written = end;
            shift += 8;
        } else {
            *out.get_mut(written).ok_or(ScanlineFault::Overrun)? = texel;
            previous = Some(texel);
            written += 1;
            shift = 0;
        }
        if written >= out.len() {
            return Ok(());
        }
        let next = take(bytes, cursor, 4).ok_or(ScanlineFault::Truncated)?;
        texel = [next[0], next[1], next[2], next[3]];
    }
}

/// Takes `count` bytes, advancing the cursor, or `None` when fewer remain.
fn take<'a>(bytes: &'a [u8], cursor: &mut usize, count: usize) -> Option<&'a [u8]> {
    let end = cursor.checked_add(count)?;
    let slice = bytes.get(*cursor..end)?;
    *cursor = end;
    Some(slice)
}

/// A structured failure while reading a sky image.
#[derive(Debug)]
pub enum SkyError {
    /// The file does not open with the `#?` signature every Radiance variant shares.
    NotRadiance,
    /// The header ran to the end of the file without the empty line that terminates it.
    TruncatedHeader,
    /// The header declared no `FORMAT`, so what the bytes mean is unstated.
    MissingFormat,
    /// The header declared a colour format that is not RGBE.
    UnsupportedColourFormat {
        /// What the file said.
        format: String,
    },
    /// The resolution line was not four whitespace-separated fields with two parsable numbers.
    MalformedResolution {
        /// The line as it appeared.
        line: String,
    },
    /// The image is stored in one of the seven orientations that are not `-Y ... +X ...`.
    UnsupportedOrientation {
        /// The two axis specifiers found.
        orientation: String,
    },
    /// A scanline could not be read.
    Scanline {
        /// Which row, counted from the top.
        row: u32,
        /// What went wrong in it.
        what: ScanlineFault,
    },
    /// The image declared a zero width or height.
    EmptyDimension {
        /// Declared width.
        width: u32,
        /// Declared height.
        height: u32,
    },
    /// A buffer's length disagreed with the dimensions it was offered under.
    TexelCountMismatch {
        /// Values the dimensions imply.
        expected: usize,
        /// Values supplied.
        actual: usize,
    },
    /// An explicit [`SkyLimits`] bound was exceeded.
    LimitExceeded {
        /// Which bound was crossed.
        what: &'static str,
        /// Observed value.
        actual: usize,
        /// Configured maximum.
        maximum: usize,
    },
}

impl Display for SkyError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotRadiance => {
                formatter.write_str("not a Radiance image: the file does not begin with `#?`")
            }
            Self::TruncatedHeader => {
                formatter.write_str("the header has no blank line, so the image never begins")
            }
            Self::MissingFormat => formatter.write_str(
                "the header declares no FORMAT, so the scanlines have no stated meaning",
            ),
            Self::UnsupportedColourFormat { format } => write!(
                formatter,
                "unsupported colour format `{format}`; this engine reads 32-bit_rle_rgbe"
            ),
            Self::MalformedResolution { line } => {
                write!(formatter, "`{line}` is not a resolution line")
            }
            Self::UnsupportedOrientation { orientation } => write!(
                formatter,
                "unsupported orientation `{orientation}`; this engine reads `-Y <height> +X <width>`"
            ),
            Self::Scanline { row, what } => write!(formatter, "row {row} is unreadable: {what}"),
            Self::EmptyDimension { width, height } => {
                write!(formatter, "a sky image cannot be {width}x{height}")
            }
            Self::TexelCountMismatch { expected, actual } => write!(
                formatter,
                "the image is {actual} values where its dimensions imply {expected}"
            ),
            Self::LimitExceeded {
                what,
                actual,
                maximum,
            } => write!(formatter, "{what} {actual} exceeds maximum {maximum}"),
        }
    }
}

impl Error for SkyError {}

#[cfg(test)]
mod tests {
    use super::{SKY_CHANNELS, SkyAsset, SkyError, SkyLimits, decode_radiance, rgbe_to_radiance};

    /// Encodes radiance as one RGBE quadruple, which is what a writer does and what the fixtures need.
    ///
    /// The exponent is the base-two logarithm of a finite positive float, so it is inside `-126..=128`
    /// and every cast below is exact.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    fn to_rgbe(colour: [f32; 3]) -> [u8; 4] {
        let peak = colour[0].max(colour[1]).max(colour[2]);
        if peak < 1.0e-32 {
            return [0, 0, 0, 0];
        }
        let exponent = peak.log2().floor() as i32 + 1;
        let scale = 256.0 / (exponent as f32).exp2();
        [
            (colour[0] * scale).clamp(0.0, 255.0) as u8,
            (colour[1] * scale).clamp(0.0, 255.0) as u8,
            (colour[2] * scale).clamp(0.0, 255.0) as u8,
            (exponent + 128) as u8,
        ]
    }

    /// A flat-encoded Radiance file: header, resolution line, and four bytes a texel.
    ///
    /// Built arithmetically rather than committed as a captured environment, which is the provenance
    /// rule this crate works under.
    fn flat_hdr(width: u32, height: u32, colour: impl Fn(u32, u32) -> [f32; 3]) -> Vec<u8> {
        let mut bytes = b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n".to_vec();
        bytes.extend_from_slice(format!("-Y {height} +X {width}\n").as_bytes());
        for y in 0..height {
            for x in 0..width {
                bytes.extend_from_slice(&to_rgbe(colour(x, y)));
            }
        }
        bytes
    }

    /// The same image in the adaptive encoding: a 2,2,width marker then four run-length planes.
    #[allow(clippy::cast_possible_truncation)]
    fn adaptive_hdr(width: u32, height: u32, colour: impl Fn(u32, u32) -> [f32; 3]) -> Vec<u8> {
        let mut bytes = b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n".to_vec();
        bytes.extend_from_slice(format!("-Y {height} +X {width}\n").as_bytes());
        for y in 0..height {
            bytes.extend_from_slice(&[2, 2, (width >> 8) as u8, (width & 0xff) as u8]);
            let row: Vec<[u8; 4]> = (0..width).map(|x| to_rgbe(colour(x, y))).collect();
            for channel in 0..4 {
                // One literal span per 128 texels, which is the simplest legal encoding and exercises
                // the literal branch; the repeat branch is covered by `a_run_length_row_expands`.
                let mut written = 0usize;
                while written < row.len() {
                    let span = (row.len() - written).min(128);
                    bytes.push(span as u8);
                    for texel in &row[written..written + span] {
                        bytes.push(texel[channel]);
                    }
                    written += span;
                }
            }
        }
        bytes
    }

    #[test]
    fn a_flat_and_an_adaptive_file_of_the_same_image_decode_identically() {
        // The claim that makes two encodings one format. They are entirely different code paths, and
        // the only thing that can check either is the other agreeing.
        let colour = |x: u32, y: u32| {
            [
                f32::from(u16::try_from(x).unwrap_or(0)) * 0.25 + 0.5,
                f32::from(u16::try_from(y).unwrap_or(0)) * 0.5 + 1.0,
                f32::from(u16::try_from(x + y).unwrap_or(0)) * 2.0 + 4.0,
            ]
        };
        let limits = SkyLimits::default();
        let flat = decode_radiance(&flat_hdr(16, 8, colour), limits).expect("flat");
        let adaptive = decode_radiance(&adaptive_hdr(16, 8, colour), limits).expect("adaptive");
        assert_eq!(flat, adaptive);
        assert_eq!((flat.width(), flat.height()), (16, 8));
        assert_eq!(flat.texels().len(), 16 * 8 * SKY_CHANNELS);
    }

    #[test]
    fn radiance_far_above_one_survives_the_round_trip() {
        // The whole point of the format, stated as a test. An 8-bit path would clamp every one of these
        // to white and the image would still look like a sky — which is why this is asserted on the
        // numbers rather than left to a capture.
        let limits = SkyLimits::default();
        for value in [0.001f32, 1.0, 40.0, 900.0, 12_000.0] {
            let bytes = flat_hdr(4, 2, |_, _| [value, value, value]);
            let sky = decode_radiance(&bytes, limits).expect("decode");
            let [r, _, _] = sky.texel(0, 0).expect("a texel");
            // RGBE keeps about eight bits of mantissa, so a part in 256 is the format's own precision.
            assert!(
                (r - value).abs() <= value / 200.0,
                "{value} came back as {r}"
            );
        }
    }

    #[test]
    fn a_run_length_row_expands_to_the_texels_it_stands_for() {
        // The repeat branch of the adaptive encoding, which the round-trip fixture deliberately does not
        // produce. A row of one colour is what a real sky's smooth regions compress to, so this is the
        // common case in a genuine file rather than an edge one.
        let width = 64u32;
        let mut bytes = b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n".to_vec();
        bytes.extend_from_slice(format!("-Y 1 +X {width}\n").as_bytes());
        bytes.extend_from_slice(&[2, 2, 0, 64]);
        let value = to_rgbe([3.0, 6.0, 12.0]);
        for channel in value {
            // 128 + 64: a run of the whole row in two bytes.
            bytes.extend_from_slice(&[128 + 64, channel]);
        }
        let sky = decode_radiance(&bytes, SkyLimits::default()).expect("decode");
        for x in 0..width {
            let [r, g, b] = sky.texel(x, 0).expect("a texel");
            assert!((r - 3.0).abs() < 0.05 && (g - 6.0).abs() < 0.1 && (b - 12.0).abs() < 0.2);
        }
    }

    #[test]
    fn the_original_repeat_record_shifts_its_count_across_consecutive_records() {
        // The detail a reader that treats each `(1,1,1,n)` independently gets wrong. Two records with
        // counts 4 and 1 mean 4 + 256 repeats, not 5 — so a naive reader ends the row 255 texels early
        // and every subsequent scanline is read from the wrong offset.
        let mut bytes = b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n".to_vec();
        bytes.extend_from_slice(b"-Y 1 +X 261\n");
        let value = to_rgbe([2.0, 2.0, 2.0]);
        bytes.extend_from_slice(&value);
        bytes.extend_from_slice(&[1, 1, 1, 4]);
        bytes.extend_from_slice(&[1, 1, 1, 1]);
        let sky = decode_radiance(&bytes, SkyLimits::default()).expect("decode");
        assert_eq!(sky.width(), 261);
        // The last texel is only reached if the second record contributed 256 rather than 1.
        let [r, _, _] = sky.texel(260, 0).expect("the last texel");
        assert!((r - 2.0).abs() < 0.05, "got {r}");
    }

    #[test]
    fn a_uniform_sky_reports_its_own_radiance_as_every_derived_figure() {
        // The check on the weighting. An equirectangular image has far more texels near the poles than
        // the sky there subtends, so an unweighted mean of a *non*-uniform sky is wrong in a way no
        // assertion on a uniform one could catch — but a uniform sky has a known answer for all three
        // figures at once, which is what makes it the right fixture for the arithmetic.
        let sky =
            SkyAsset::uniform(64, 32, [0.2, 0.4, 0.8], SkyLimits::default()).expect("uniform");
        let lighting = sky.lighting();
        for figure in [lighting.horizon, lighting.zenith, lighting.ambient] {
            for (channel, expected) in figure.iter().zip([0.2, 0.4, 0.8]) {
                assert!(
                    (channel - expected).abs() < 1.0e-4,
                    "{figure:?} against [0.2, 0.4, 0.8]"
                );
            }
        }
    }

    #[test]
    fn the_horizon_and_the_zenith_are_read_from_where_they_are() {
        // A sky that is bright overhead and dark at the horizon, which is every clear sky. Getting the
        // two the wrong way round would derive a fog colour from the zenith — and fog fades toward the
        // sky *behind* the terrain, which is the horizon, so the band along every silhouette would be
        // the wrong colour. That is invisible in any assertion about brightness.
        let height = 64u32;
        let sky = SkyAsset::new(
            32,
            height,
            (0..height)
                .flat_map(|y| {
                    let up = if y < height / 2 { 1.0 } else { 0.1 };
                    std::iter::repeat_n([0.0, 0.0, up, 1.0], 32).flatten()
                })
                .collect(),
            SkyLimits::default(),
        )
        .expect("gradient");
        let lighting = sky.lighting();
        assert!(lighting.zenith[2] > 0.9, "{:?}", lighting.zenith);
        // The horizon band straddles the boundary, so it is the mean of the two halves rather than
        // either — the assertion is that it is *between* them and nowhere near the zenith.
        assert!(
            lighting.horizon[2] > 0.4 && lighting.horizon[2] < 0.7,
            "{:?}",
            lighting.horizon
        );
        // Cosine weighting concentrates the ambient overhead, so a bright top dominates it.
        assert!(lighting.ambient[2] > lighting.horizon[2]);
    }

    #[test]
    fn a_suns_disc_does_not_dominate_the_ambient_it_is_already_modelled_by() {
        // The double count this clamp exists to prevent, at the ratio a real HDRI has. A sun covering
        // a two-hundredth of the sky at ten thousand times its radiance carries most of the true
        // irradiance — and a renderer that also has a directional light for that sun would then light
        // the shade as brightly as the sunlight, which reads as the shadows having stopped working.
        let (width, height) = (64u32, 32u32);
        let mut texels = vec![0.0f32; (width * height) as usize * SKY_CHANNELS];
        for y in 0..height {
            for x in 0..width {
                let at = (y as usize * width as usize + x as usize) * SKY_CHANNELS;
                // A dim uniform sky over the upper half, black ground below.
                let sky = if y < height / 2 { 0.2 } else { 0.0 };
                for channel in 0..3 {
                    texels[at + channel] = sky;
                }
                texels[at + 3] = 1.0;
            }
        }
        let plain = SkyAsset::new(width, height, texels.clone(), SkyLimits::default())
            .expect("sky")
            .lighting();

        // The same sky with a small, very bright disc near the zenith.
        for y in 0..2u32 {
            for x in 30..34u32 {
                let at = (y as usize * width as usize + x as usize) * SKY_CHANNELS;
                for channel in 0..3 {
                    texels[at + channel] = 2_000.0;
                }
            }
        }
        let sunlit = SkyAsset::new(width, height, texels, SkyLimits::default())
            .expect("sky")
            .lighting();

        // Unclamped, this sun would raise the ambient by orders of magnitude. Clamped, it lifts it a
        // little — the disc is still sky, it is just no longer *all* of it.
        assert!(
            sunlit.ambient[1] < plain.ambient[1] * 3.0,
            "the sun ran away with the ambient: {:?} against {:?}",
            sunlit.ambient,
            plain.ambient
        );
        assert!(
            sunlit.ambient[1] > plain.ambient[1],
            "and it must still count for something"
        );
    }

    #[test]
    fn an_overcast_sky_is_untouched_by_the_sun_clamp() {
        // The case where clamping would be felt if the ceiling were set too low. Nothing in a sky whose
        // brightest part is twice its mean reaches eight times it, so this must be exact rather than
        // merely close: a clamp that bites here is a clamp that dims every overcast scene.
        let (width, height) = (32u32, 16u32);
        let mut texels = vec![0.0f32; (width * height) as usize * SKY_CHANNELS];
        for y in 0..height {
            for x in 0..width {
                let at = (y as usize * width as usize + x as usize) * SKY_CHANNELS;
                // A gentle bright patch, twice the surrounding value at its peak.
                let bright = if x < width / 4 { 0.6 } else { 0.3 };
                for channel in 0..3 {
                    texels[at + channel] = bright;
                }
                texels[at + 3] = 1.0;
            }
        }
        let lighting = SkyAsset::new(width, height, texels, SkyLimits::default())
            .expect("sky")
            .lighting();
        // Three quarters at 0.3 and a quarter at 0.6, uniformly over the hemisphere: 0.375.
        assert!(
            (lighting.ambient[0] - 0.375).abs() < 1.0e-3,
            "{:?} is not the unclamped mean",
            lighting.ambient
        );
    }

    #[test]
    fn a_mip_chain_reaches_one_by_one_and_preserves_a_constant() {
        let sky = SkyAsset::uniform(16, 8, [1.5, 2.5, 3.5], SkyLimits::default()).expect("uniform");
        let chain = sky.mip_chain();
        let sizes: Vec<(u32, u32)> = chain.iter().map(|(w, h, _)| (*w, *h)).collect();
        assert_eq!(sizes, [(16, 8), (8, 4), (4, 2), (2, 1), (1, 1)]);
        for (width, height, texels) in &chain {
            assert_eq!(
                texels.len(),
                *width as usize * *height as usize * SKY_CHANNELS
            );
            for texel in texels.chunks_exact(SKY_CHANNELS) {
                assert!((texel[0] - 1.5).abs() < 1.0e-5, "level {width}x{height}");
                assert!((texel[2] - 3.5).abs() < 1.0e-5, "level {width}x{height}");
            }
        }
    }

    #[test]
    fn a_reduction_wraps_across_the_meridian_rather_than_clamping_at_it() {
        // The one way an equirectangular reduction differs from an ordinary one. Columns 0 and
        // `width - 1` are adjacent *directions*, so the last column of a reduced level must average the
        // two ends together — a clamping reduction averages the last column with itself and leaves a
        // seam down the meridian that every further level widens.
        //
        // The fixture is black everywhere but the first column, so the wrap is the only thing that can
        // put light into the last texel of the reduced row.
        let width = 8u32;
        let mut texels = vec![0.0f32; width as usize * 2 * SKY_CHANNELS];
        for y in 0..2usize {
            texels[y * width as usize * SKY_CHANNELS] = 1.0;
        }
        let sky = SkyAsset::new(width, 2, texels, SkyLimits::default()).expect("stripe");
        let chain = sky.mip_chain();
        let (reduced_width, _, level) = &chain[1];
        assert_eq!(*reduced_width, 4);
        // Column 0 of the reduced row averages source columns 0 and 1: one lit of two.
        assert!((level[0] - 0.5).abs() < 1.0e-5, "got {}", level[0]);
        // And the last column averages source columns 6 and 7, both dark — the seam test is that this
        // stays dark while column 0 does not, which only holds if the wrap took the *modulo* rather
        // than clamping column 8 back onto column 7.
        let last = 3 * SKY_CHANNELS;
        assert!(level[last].abs() < 1.0e-5, "got {}", level[last]);
    }

    #[test]
    fn refuses_what_it_cannot_read_rather_than_guessing() {
        let limits = SkyLimits::default();
        assert!(matches!(
            decode_radiance(b"not radiance at all", limits),
            Err(SkyError::NotRadiance)
        ));
        assert!(matches!(
            decode_radiance(b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n", limits),
            Err(SkyError::TruncatedHeader)
        ));
        assert!(matches!(
            decode_radiance(b"#?RADIANCE\n\n-Y 2 +X 2\n", limits),
            Err(SkyError::MissingFormat)
        ));
        // XYZE is a real Radiance file whose channels are CIE tristimulus values. Read as RGB it
        // produces a plausible, wrongly coloured picture, which is the worst possible outcome.
        assert!(matches!(
            decode_radiance(b"#?RADIANCE\nFORMAT=32-bit_rle_xyze\n\n-Y 2 +X 2\n", limits),
            Err(SkyError::UnsupportedColourFormat { .. })
        ));
        // A rotated orientation. Reading it anyway would put the sky on the ground.
        assert!(matches!(
            decode_radiance(b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n+Y 2 +X 2\n", limits),
            Err(SkyError::UnsupportedOrientation { .. })
        ));
        assert!(matches!(
            decode_radiance(b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\nnonsense\n", limits),
            Err(SkyError::MalformedResolution { .. })
        ));
    }

    #[test]
    fn refuses_a_scanline_the_file_does_not_carry_before_allocating_for_it() {
        // The bound that matters for hostile input: the resolution line is what sizes the buffer, so a
        // header claiming a large image while carrying a handful of bytes must fail on what is present.
        let mut bytes = b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 2048 +X 4096\n".to_vec();
        bytes.extend_from_slice(&[0u8; 32]);
        assert!(matches!(
            decode_radiance(&bytes, SkyLimits::default()),
            Err(SkyError::Scanline { .. })
        ));
    }

    #[test]
    fn refuses_an_image_past_the_declared_bounds() {
        let bytes = flat_hdr(64, 32, |_, _| [1.0, 1.0, 1.0]);
        assert!(matches!(
            decode_radiance(
                &bytes,
                SkyLimits {
                    maximum_dimension: 32,
                    ..SkyLimits::default()
                }
            ),
            Err(SkyError::LimitExceeded {
                what: "sky dimension",
                ..
            })
        ));
        assert!(matches!(
            decode_radiance(
                &bytes,
                SkyLimits {
                    maximum_bytes: 16,
                    ..SkyLimits::default()
                }
            ),
            Err(SkyError::LimitExceeded {
                what: "sky file bytes",
                ..
            })
        ));
    }

    #[test]
    fn an_image_above_the_target_is_reduced_rather_than_refused() {
        // The bound that behaves differently from every other one in this crate, and the reason is
        // the content: HDRIs ship at 8K by convention and 8K is more resolution than a sky can use.
        // Refusing the ordinary size of the ordinary asset would be a bound nobody could keep.
        let bytes = flat_hdr(64, 32, |x, _| {
            // A ramp along x, so an averaging fault shows as the wrong value rather than as the
            // wrong size — which a constant fixture could not tell apart.
            let value = f32::from(u16::try_from(x).unwrap_or(0));
            [value, value, value]
        });
        let sky = decode_radiance(
            &bytes,
            SkyLimits {
                target_dimension: 16,
                ..SkyLimits::default()
            },
        )
        .expect("a large image is reduced, not refused");
        assert_eq!((sky.width(), sky.height()), (16, 8), "reduced by four");

        // Kept texel 0 is the mean of source columns 0..4, which is 1.5. RGBE keeps about eight bits
        // of mantissa, hence the tolerance rather than an exact comparison.
        let [first, _, _] = sky.texel(0, 0).expect("a texel");
        assert!((first - 1.5).abs() < 0.05, "got {first}");
        // And the last, which is the mean of columns 60..64: 61.5.
        let [last, _, _] = sky.texel(15, 0).expect("a texel");
        assert!((last - 61.5).abs() < 0.5, "got {last}");
    }

    #[test]
    fn a_dimension_the_factor_does_not_divide_keeps_its_short_block_at_full_brightness() {
        // The arithmetic trap in an on-the-fly box filter. A 6-wide image reduced by four has a final
        // block of two texels, and dividing it by four rather than by two would leave the right-hand
        // column of every reduced sky half as bright — a dark seam down the meridian, which is exactly
        // the artefact the wrapping reduction elsewhere exists to avoid.
        let bytes = flat_hdr(6, 6, |_, _| [4.0, 4.0, 4.0]);
        let sky = decode_radiance(
            &bytes,
            SkyLimits {
                target_dimension: 2,
                ..SkyLimits::default()
            },
        )
        .expect("decode");
        assert_eq!((sky.width(), sky.height()), (2, 2));
        for y in 0..2 {
            for x in 0..2 {
                let [value, _, _] = sky.texel(x, y).expect("a texel");
                assert!(
                    (value - 4.0).abs() < 0.05,
                    "texel ({x}, {y}) came back as {value} rather than 4.0"
                );
            }
        }
    }

    #[test]
    fn the_reduction_factor_is_a_power_of_two_and_keeps_the_axes_in_proportion() {
        // Both axes reduce by the same factor, which they must: an equirectangular image whose
        // latitude and longitude reduced differently would render as a stretched sky.
        let limits = SkyLimits {
            target_dimension: 2_048,
            ..SkyLimits::default()
        };
        assert_eq!(super::reduction_for(8_192, 4_096, limits), 4);
        assert_eq!(super::reduction_for(4_096, 2_048, limits), 2);
        assert_eq!(super::reduction_for(2_048, 1_024, limits), 1);
        assert_eq!(super::reduction_for(1_024, 512, limits), 1);
        // A non-power-of-two source lands below the target rather than being stretched onto it.
        assert_eq!(super::reduction_for(6_000, 3_000, limits), 4);
    }

    #[test]
    fn an_exposure_recorded_in_the_header_is_divided_back_out() {
        // The specification says a reader wanting original radiance divides the recorded exposure out,
        // and here that is not a nicety: the ambient term this drives is a physical quantity, so a file
        // that has been through a tool which halved it would otherwise light the scene at half strength
        // with nothing to indicate why.
        let mut bytes = b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\nEXPOSURE=2.0\n\n".to_vec();
        bytes.extend_from_slice(b"-Y 1 +X 1\n");
        bytes.extend_from_slice(&to_rgbe([4.0, 4.0, 4.0]));
        let sky = decode_radiance(&bytes, SkyLimits::default()).expect("decode");
        let [r, _, _] = sky.texel(0, 0).expect("a texel");
        assert!((r - 2.0).abs() < 0.05, "got {r}");
    }

    #[test]
    fn the_black_exponent_is_exactly_zero_rather_than_a_half_step() {
        // The half-step reconstruction is right for every other exponent and wrong for this one: there
        // is no value small enough to mean zero otherwise, so the format reserves a zero exponent for it.
        assert!(
            rgbe_to_radiance([255, 255, 255, 0])
                .iter()
                .all(|c| *c == 0.0)
        );
        assert!(rgbe_to_radiance([0, 0, 0, 128])[0] > 0.0);
    }
}
