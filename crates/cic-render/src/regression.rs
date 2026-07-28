//! Comparing a capture against a committed reference image.
//!
//! # Why this exists
//!
//! A green test suite coexists comfortably with a visibly broken frame. Every rendering fault this
//! project has had passed its own assertions and was caught by opening the PNG: reversed layer ramps,
//! two separate tone-mapping mistakes, a shadow camera on the wrong side of the scene, a quad walked in
//! the wrong order. A threshold on luminance spread cannot catch any of them, because each produces an
//! image with a perfectly healthy spread. Comparing against an image that was *looked at* can.
//!
//! # Why this takes bytes rather than a path
//!
//! The layering rule: nothing above the resource layer opens a file. A reference arrives as bytes, the
//! caller decides where bytes come from, and the same code serves a file on disk, a packaged fixture,
//! and a buffer built in a test.
//!
//! # Why a reference belongs to one adapter
//!
//! Two GPUs do not agree to the byte, and since the same eleven scenes are now rendered by an RTX 4080
//! SUPER and by Mesa's lavapipe, *how much* they disagree is measured rather than assumed. The answer is
//! not what this paragraph used to claim. The nine scenes that sample **no texture** agree to within
//! 0.0191% of pixels at a peak channel difference of 9 — inside this tolerance, so each would have
//! passed against the other adapter's reference. The two that **do** sample one are rejected outright:
//! textured models by 0.3487%, and the world-space tiled terrain albedo by **11.4092%**, which is 114
//! times the allowance. Between the worst textured case and the worst untextured one there is a factor
//! of about six hundred.
//!
//! So the split is necessary, and it is necessary for one reason rather than the four that were
//! guessed at. `pow`, `sin`, and `exp` differing in their last places does not register at this
//! tolerance, and neither does the occlusion pass compounding samples. Mip selection under trilinear
//! filtering does, on fine detail at grazing incidence, because the specification leaves it latitude and
//! two implementations spend it differently. A tolerance loose enough to span both adapters on a
//! textured frame would be a hundred times looser than one that catches a real regression, which is
//! what would make the harness a rubber stamp. One set per adapter keeps it tight; the cost is that a
//! new adapter needs its own set generated and looked at once.
//!
//! The useful consequence is a prediction rather than a rule of thumb: as more of the renderer samples
//! textures — normal and roughness maps being next — more of the set diverges across adapters, not
//! less. It also prices the decision to decline anisotropic filtering, since that is precisely the
//! feature that would have narrowed the gap on the terrain case.
//!
//! # What the tolerance is for
//!
//! Not for vendor differences, which the per-adapter split already handles, but for a driver update
//! moving a handful of edge pixels. It is deliberately far too tight to absorb a real change: a
//! regression that alters shading, geometry, or tone moves a large share of the frame, not a fraction
//! of a percent of it.

use crate::RenderError;
use crate::gpu::Capture;

/// How much difference counts as noise rather than as a regression.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tolerance {
    /// Per-channel difference, in 8-bit steps, at or below which two pixels count as equal.
    pub channel: u8,
    /// Fraction of pixels allowed to exceed `channel` before the comparison fails.
    pub differing: f32,
}

impl Tolerance {
    /// Byte-for-byte identical.
    ///
    /// Correct for a comparison between two captures taken in one process, where any difference at all
    /// is a real one. Too brittle for a committed reference, which outlives the driver it was made on.
    pub const EXACT: Self = Self {
        channel: 0,
        differing: 0.0,
    };

    /// The default for a reference committed from the same adapter.
    ///
    /// Rendering the same scene twice on one driver is reproducible, so this is not slack for
    /// nondeterminism — it is room for a driver update to round a few edge pixels differently. At a
    /// 720x480 capture, `differing` permits about 350 pixels to move by more than a step or two, and
    /// the smallest fault worth catching moves tens of thousands.
    pub const SAME_ADAPTER: Self = Self {
        channel: 2,
        differing: 0.001,
    };
}

impl Default for Tolerance {
    fn default() -> Self {
        Self::SAME_ADAPTER
    }
}

/// What a comparison found.
#[derive(Debug, Clone, Copy)]
pub struct Comparison {
    /// Frame width the two images shared.
    pub width: u32,
    /// Frame height the two images shared.
    pub height: u32,
    /// Fraction of pixels whose largest channel difference exceeded the tolerance.
    pub differing: f32,
    /// The largest single channel difference anywhere in the frame.
    pub peak: u8,
    /// Mean absolute channel difference over every channel, in 8-bit steps.
    pub mean: f32,
    tolerance: Tolerance,
}

impl Comparison {
    /// Whether the capture is within tolerance of the reference.
    #[must_use]
    pub fn passes(&self) -> bool {
        self.differing <= self.tolerance.differing
    }

    /// Returns the tolerance this comparison was measured against.
    #[must_use]
    pub const fn tolerance(&self) -> Tolerance {
        self.tolerance
    }
}

impl std::fmt::Display for Comparison {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{:.4}% of pixels differ by more than {} (allowed {:.4}%), \
             peak channel difference {}, mean {:.3}, over {}x{}",
            self.differing * 100.0,
            self.tolerance.channel,
            self.tolerance.differing * 100.0,
            self.peak,
            self.mean,
            self.width,
            self.height
        )
    }
}

/// Decodes an 8-bit RGBA PNG into `(width, height, rgba)`.
///
/// Deliberately strict about the format rather than converting whatever it is handed. Every reference
/// this compares against was written by [`Capture::png`], so anything else is a mistake worth reporting
/// — the same posture the asset decoders take.
///
/// # Errors
///
/// Returns [`RenderError::DecodePng`] when the bytes are not a readable PNG, or are not 8-bit RGBA.
pub fn decode_png(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), RenderError> {
    // A cursor rather than the slice itself: the decoder needs `Seek`, which `&[u8]` does not provide.
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder
        .read_info()
        .map_err(|error: png::DecodingError| RenderError::DecodePng(error.to_string()))?;
    let (colour, depth) = {
        let info = reader.info();
        (info.color_type, info.bit_depth)
    };
    if colour != png::ColorType::Rgba || depth != png::BitDepth::Eight {
        return Err(RenderError::DecodePng(format!(
            "expected 8-bit RGBA, found {colour:?} at {depth:?}"
        )));
    }
    let mut rgba = vec![0u8; reader.output_buffer_size().unwrap_or(0)];
    let frame = reader
        .next_frame(&mut rgba)
        .map_err(|error: png::DecodingError| RenderError::DecodePng(error.to_string()))?;
    let (width, height) = (frame.width, frame.height);
    rgba.truncate(frame.buffer_size());
    Ok((width, height, rgba))
}

/// Compares two tightly packed RGBA buffers of the same size.
///
/// Operates on buffers rather than on [`Capture`] so it can be exercised without a GPU — which matters
/// more than it sounds, because a machine with no adapter skips every rendering test, and the
/// comparison itself would otherwise be the one part of this harness that nothing ever checks.
#[must_use]
pub fn compare_rgba(
    actual: &[u8],
    reference: &[u8],
    width: u32,
    height: u32,
    tolerance: Tolerance,
) -> Comparison {
    let mut differing = 0usize;
    let mut peak = 0u8;
    let mut total = 0u64;
    let mut pixels = 0usize;
    for (left, right) in actual.chunks_exact(4).zip(reference.chunks_exact(4)) {
        pixels += 1;
        let mut worst = 0u8;
        for channel in 0..4 {
            let difference = left[channel].abs_diff(right[channel]);
            worst = worst.max(difference);
            total += u64::from(difference);
        }
        peak = peak.max(worst);
        if worst > tolerance.channel {
            differing += 1;
        }
    }
    // Pixel and channel counts are bounded by the capture limits, far inside exact f32 range.
    #[allow(clippy::cast_precision_loss)]
    let (differing_fraction, mean) = if pixels == 0 {
        (0.0, 0.0)
    } else {
        (
            differing as f32 / pixels as f32,
            total as f32 / (pixels as f32 * 4.0),
        )
    };
    Comparison {
        width,
        height,
        differing: differing_fraction,
        peak,
        mean,
        tolerance,
    }
}

/// Compares a capture against an encoded reference image.
///
/// # Errors
///
/// Returns [`RenderError::DecodePng`] when the reference cannot be decoded, or
/// [`RenderError::ReferenceSizeMismatch`] when it was rendered at a different size — which is a
/// mismatch to fix rather than to measure, since comparing frames of different sizes has no meaning.
pub fn compare(
    capture: &Capture,
    reference_png: &[u8],
    tolerance: Tolerance,
) -> Result<Comparison, RenderError> {
    let (width, height, reference) = decode_png(reference_png)?;
    if width != capture.width() || height != capture.height() {
        return Err(RenderError::ReferenceSizeMismatch {
            reference: [width, height],
            capture: [capture.width(), capture.height()],
        });
    }
    Ok(compare_rgba(
        capture.rgba(),
        &reference,
        width,
        height,
        tolerance,
    ))
}

/// How much a difference image amplifies the difference it shows.
///
/// Chosen so the *subtlest* change the tolerance rejects is still plainly visible: a three-step shift
/// becomes mid-grey rather than near-black. A large regression saturates instead of showing its
/// magnitude, which costs nothing — the comparison's own numbers carry magnitude, and the image is
/// there to say *where*.
const DIFFERENCE_GAIN: u16 = 24;

/// Encodes an amplified per-channel difference between a capture and a reference.
///
/// # Errors
///
/// Returns [`RenderError::DecodePng`] or [`RenderError::ReferenceSizeMismatch`] as [`compare`] does,
/// or [`RenderError::EncodePng`] when the result cannot be encoded.
pub fn difference_png(capture: &Capture, reference_png: &[u8]) -> Result<Vec<u8>, RenderError> {
    let (width, height, reference) = decode_png(reference_png)?;
    if width != capture.width() || height != capture.height() {
        return Err(RenderError::ReferenceSizeMismatch {
            reference: [width, height],
            capture: [capture.width(), capture.height()],
        });
    }
    let mut rgba = Vec::with_capacity(reference.len());
    for (left, right) in capture
        .rgba()
        .chunks_exact(4)
        .zip(reference.chunks_exact(4))
    {
        for channel in 0..3 {
            let difference = u16::from(left[channel].abs_diff(right[channel]));
            rgba.push(u8::try_from((difference * DIFFERENCE_GAIN).min(255)).unwrap_or(u8::MAX));
        }
        // Opaque, so the image is legible in any viewer rather than showing the difference as
        // transparency over whatever happens to be behind it.
        rgba.push(u8::MAX);
    }
    encode_rgba(&rgba, width, height)
}

/// Encodes tightly packed RGBA as a PNG.
fn encode_rgba(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, RenderError> {
    let mut output = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut output, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|error| RenderError::EncodePng(error.to_string()))?;
        writer
            .write_image_data(rgba)
            .map_err(|error| RenderError::EncodePng(error.to_string()))?;
        writer
            .finish()
            .map_err(|error| RenderError::EncodePng(error.to_string()))?;
    }
    Ok(output)
}

/// A filesystem-safe name for an adapter's reference set.
///
/// Backend as well as adapter name, because one card reached through Vulkan and through DX12 is two
/// different rasterisers as far as a committed image is concerned.
#[must_use]
pub fn adapter_slug(backend: wgpu::Backend, adapter_name: &str) -> String {
    let mut slug = format!("{backend:?}-{adapter_name}").to_lowercase();
    slug = slug
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect();
    // Collapse runs, so "GeForce RTX 4080 (SUPER)" does not become a name full of empty segments.
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    slug.trim_matches('-').to_owned()
}

#[cfg(test)]
mod tests {
    use super::{Tolerance, adapter_slug, compare_rgba, decode_png, encode_rgba};

    /// A solid frame, as tightly packed RGBA.
    fn solid(width: u32, height: u32, colour: [u8; 4]) -> Vec<u8> {
        colour
            .iter()
            .copied()
            .cycle()
            .take((width * height * 4) as usize)
            .collect()
    }

    #[test]
    fn identical_buffers_differ_nowhere() {
        let frame = solid(8, 4, [10, 20, 30, 255]);
        let comparison = compare_rgba(&frame, &frame, 8, 4, Tolerance::EXACT);
        assert!(comparison.passes(), "{comparison}");
        assert_eq!(comparison.peak, 0);
        assert!(comparison.differing.abs() < f32::EPSILON);
    }

    #[test]
    fn a_difference_within_the_channel_tolerance_is_not_counted() {
        // The case the tolerance exists for: a driver rounding a channel by one step. It has to show up
        // in `peak`, so it is still *visible*, while not counting as a differing pixel.
        let reference = solid(4, 4, [100, 100, 100, 255]);
        let actual = solid(4, 4, [102, 100, 100, 255]);
        let comparison = compare_rgba(&actual, &reference, 4, 4, Tolerance::SAME_ADAPTER);
        assert!(comparison.passes(), "{comparison}");
        assert_eq!(comparison.peak, 2, "the difference must still be reported");
        assert!(comparison.differing.abs() < f32::EPSILON);
    }

    #[test]
    fn a_difference_beyond_the_channel_tolerance_fails() {
        let reference = solid(4, 4, [100, 100, 100, 255]);
        let actual = solid(4, 4, [140, 100, 100, 255]);
        let comparison = compare_rgba(&actual, &reference, 4, 4, Tolerance::SAME_ADAPTER);
        assert!(!comparison.passes(), "{comparison}");
        assert_eq!(comparison.peak, 40);
        // Every pixel moved, so the fraction is one rather than merely over the allowance.
        assert!((comparison.differing - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn a_regression_confined_to_a_few_pixels_still_fails_at_a_realistic_frame_size() {
        // The tolerance is a fraction, so the guard against it absorbing a small real change is that
        // the frame is large. At 720x480 the allowance is about 345 pixels; 4,000 changed pixels — a
        // patch some 60 pixels square — has to fail, or a broken shadow on one building would pass.
        let (width, height) = (720u32, 480u32);
        let reference = solid(width, height, [80, 90, 100, 255]);
        let mut actual = reference.clone();
        for pixel in actual.chunks_exact_mut(4).take(4_000) {
            pixel[0] = 200;
        }
        let comparison = compare_rgba(&actual, &reference, width, height, Tolerance::SAME_ADAPTER);
        assert!(!comparison.passes(), "{comparison}");
    }

    #[test]
    fn a_round_trip_through_png_preserves_every_byte() {
        // If it did not, every reference would fail against the capture it was written from.
        let frame = solid(6, 3, [1, 2, 3, 255]);
        let encoded = encode_rgba(&frame, 6, 3).expect("encode");
        let (width, height, decoded) = decode_png(&encoded).expect("decode");
        assert_eq!((width, height), (6, 3));
        assert_eq!(decoded, frame);
    }

    #[test]
    fn a_png_that_is_not_eight_bit_rgba_is_refused() {
        // Rather than silently comparing against a channel order or depth that is not what was written.
        let mut output = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut output, 2, 2);
            encoder.set_color(png::ColorType::Grayscale);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("header");
            writer.write_image_data(&[0, 1, 2, 3]).expect("data");
            writer.finish().expect("finish");
        }
        assert!(decode_png(&output).is_err());
    }

    #[test]
    fn refusing_garbage_beats_guessing_at_it() {
        assert!(decode_png(&[]).is_err());
        assert!(decode_png(b"this is not a png").is_err());
    }

    #[test]
    fn an_adapter_slug_is_filesystem_safe_and_names_the_backend() {
        let slug = adapter_slug(wgpu::Backend::Vulkan, "NVIDIA GeForce RTX 4080 SUPER");
        assert_eq!(slug, "vulkan-nvidia-geforce-rtx-4080-super");
        // Punctuation collapses rather than leaving empty segments behind.
        assert_eq!(
            adapter_slug(wgpu::Backend::Dx12, "Radeon (TM) / Pro:: 580"),
            "dx12-radeon-tm-pro-580"
        );
    }
}
