//! Pixel math shared by the runtime uploader and the offline texture tool: colour spaces, bilinear
//! resampling, and mip reduction.
//!
//! # Why this lives on the asset side rather than in the renderer
//!
//! It began in `cic-render`, where mip chains were generated at upload.
//! [ADR 0004](../../../docs/adr/0004-texture-arrays-and-world-space-tiling.md) said that if the upload
//! cost ever became a problem the answer was *precomputed mips in the asset*, not a GPU blit chain —
//! and [ADR 2001](../../../docs/adr/2001-block-compressed-textures.md) took that answer. So there are
//! now two places that reduce an image by half:
//! the renderer, for an uncompressed array it still mips itself, and the converter, which bakes a mip
//! chain into a `.dds` ahead of time.
//!
//! Those two must agree exactly. A converter that averaged encoded sRGB bytes while the renderer
//! averaged linear light would put a *visibly different* mip chain in the asset from the one the
//! renderer would have built, and the difference would appear only as the camera pulls back — on
//! precisely the textures that were converted, which reads as "block compression darkens things"
//! rather than as an arithmetic mistake. One definition, used by both, is what stops that.
//!
//! # Averaging in linear light
//!
//! An sRGB byte is not proportional to light. The transfer curve is concave, so the mean of two
//! encoded values sits *above* the encoding of their mean: averaging 0 and 255 as bytes gives 128,
//! where the correct answer is about 188. On a high-contrast texture the naive version reads as a
//! surface that pales as it recedes.
//!
//! # Why the colour space is a parameter
//!
//! A normal map, a roughness map and an occlusion map are not colours. Their bytes are measurements
//! stored linearly, and running them through the sRGB decode *undoes an encoding that was never
//! applied*: a flat normal's 128 becomes 0.216 instead of 0.502, so the surface tilts everywhere by the
//! same wrong amount and reads as a lighting bug rather than a texture one. Roughness 128 becomes 0.216
//! too, which is a different material.

/// Whether an image's bytes are an sRGB-encoded colour or a linear measurement.
///
/// This governs two things together, and they must agree: the texture format the hardware reads
/// through, and whether the averaging here round-trips through the transfer function. A format without
/// the matching averaging is the more insidious half of the mistake, because it is invisible at the
/// base mip level and only appears as the camera pulls back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColourSpace {
    /// sRGB-encoded colour: albedo, base colour, anything a human picked.
    ///
    /// The hardware decodes on read, and the averaging here happens in linear light.
    Srgb,
    /// Linear data: normals, roughness, metallic, occlusion.
    ///
    /// The hardware returns the stored value, and the averaging here works on the stored values.
    Linear,
}

/// Number of mip levels down to a 1x1 tail.
#[must_use]
pub const fn mip_level_count(width: u32, height: u32) -> u32 {
    let largest = if width > height { width } else { height };
    // `ilog2` of the largest dimension, plus the base level. `max(1)` guards the zero case, which the
    // callers already refuse but this function does not require.
    if largest <= 1 { 1 } else { largest.ilog2() + 1 }
}

/// Halves a dimension, stopping at one.
#[must_use]
pub const fn halve(value: u32) -> u32 {
    if value > 1 { value / 2 } else { 1 }
}

/// Builds a full mip chain from a base level, as `(width, height, rgba)` per level.
///
/// The whole chain at once, because that is what an offline converter writes: a `.dds` stores every
/// level, and the encoder needs each one's pixels before it can compress them. The renderer's own
/// uploader keeps one working buffer of floats instead and reduces in place, which is the same
/// arithmetic with less peak memory — the property that matters there and not here.
///
/// Returns an empty chain for a zero dimension, which is the only input with no valid answer.
#[must_use]
pub fn mip_chain(
    rgba: &[u8],
    width: u32,
    height: u32,
    space: ColourSpace,
) -> Vec<(u32, u32, Vec<u8>)> {
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let levels = mip_level_count(width, height);
    let mut chain = Vec::with_capacity(levels as usize);
    let mut working = decode_in(rgba, space);
    let (mut level_width, mut level_height) = (width, height);
    for level in 0..levels {
        if level > 0 {
            let (next_width, next_height) = (halve(level_width), halve(level_height));
            working = reduce(&working, level_width, level_height);
            level_width = next_width;
            level_height = next_height;
        }
        chain.push((level_width, level_height, encode_in(&working, space)));
    }
    chain
}

/// Expands stored bytes to the floats this module averages, for a given space.
///
/// Alpha is never gamma encoded, in either space — it is a coverage fraction, and running it through the
/// colour transfer function would make every partially transparent edge the wrong opacity.
///
/// [`ColourSpace::Linear`] is a plain scale by 1/255 on every channel including alpha, so the round trip
/// through [`encode_in`] is the identity up to rounding — which is the property a normal or roughness
/// map needs and the sRGB path deliberately does not have.
#[must_use]
pub fn decode_in(rgba: &[u8], space: ColourSpace) -> Vec<f32> {
    rgba.iter()
        .enumerate()
        .map(|(index, value)| {
            if space == ColourSpace::Linear || index % 4 == 3 {
                f32::from(*value) / 255.0
            } else {
                srgb_to_linear(*value)
            }
        })
        .collect()
}

/// Re-encodes averaged floats as stored bytes, in the space they were decoded from.
#[must_use]
pub fn encode_in(values: &[f32], space: ColourSpace) -> Vec<u8> {
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            if space == ColourSpace::Linear || index % 4 == 3 {
                quantize(*value)
            } else {
                quantize(linear_to_srgb(*value))
            }
        })
        .collect()
}

/// Decodes one sRGB byte to linear light.
#[must_use]
pub fn srgb_to_linear(value: u8) -> f32 {
    let value = f32::from(value) / 255.0;
    if value <= 0.040_45 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

/// Encodes one linear-light value as sRGB, in `0..=1`.
#[must_use]
pub fn linear_to_srgb(value: f32) -> f32 {
    if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

/// Rounds a `0..=1` value to a byte.
#[must_use]
pub fn quantize(value: f32) -> u8 {
    // Clamped into `0..=255` immediately before the cast, so neither bound can be crossed.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    {
        (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
    }
}

/// Bilinearly resamples RGBA already expanded by [`decode_in`].
///
/// Sample positions use the half-texel convention: destination texel centres map to source texel
/// centres, so a resample to the same size is the identity rather than a half-pixel shift.
// Every cast here is from a coordinate already clamped non-negative and below a dimension bound, so
// neither the truncation nor the sign can bite.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
#[must_use]
pub fn resample(
    source: &[f32],
    source_width: u32,
    source_height: u32,
    width: u32,
    height: u32,
) -> Vec<f32> {
    let mut output = Vec::with_capacity((width as usize) * (height as usize) * 4);
    let x_ratio = source_width as f32 / width as f32;
    let y_ratio = source_height as f32 / height as f32;
    let last_x = source_width.saturating_sub(1);
    let last_y = source_height.saturating_sub(1);

    for y in 0..height {
        let source_y = ((y as f32 + 0.5) * y_ratio - 0.5).max(0.0);
        let y0 = (source_y as u32).min(last_y);
        let y1 = (y0 + 1).min(last_y);
        let fy = source_y - y0 as f32;
        for x in 0..width {
            let source_x = ((x as f32 + 0.5) * x_ratio - 0.5).max(0.0);
            let x0 = (source_x as u32).min(last_x);
            let x1 = (x0 + 1).min(last_x);
            let fx = source_x - x0 as f32;
            for channel in 0..4 {
                let at = |x: u32, y: u32| {
                    source[((y as usize * source_width as usize) + x as usize) * 4 + channel]
                };
                let top = at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx;
                let bottom = at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx;
                output.push(top * (1.0 - fy) + bottom * fy);
            }
        }
    }
    output
}

/// Reduces expanded RGBA to the next mip level by averaging each 2x2 block.
///
/// An odd dimension halves downward and the trailing row or column is folded into its neighbour by the
/// clamp, which is what keeps a 3-wide level from losing its last column entirely.
#[must_use]
pub fn reduce(source: &[f32], width: u32, height: u32) -> Vec<f32> {
    let (next_width, next_height) = (halve(width), halve(height));
    let mut output = Vec::with_capacity((next_width as usize) * (next_height as usize) * 4);
    let last_x = width.saturating_sub(1);
    let last_y = height.saturating_sub(1);
    for y in 0..next_height {
        let y0 = (y * 2).min(last_y);
        let y1 = (y * 2 + 1).min(last_y);
        for x in 0..next_width {
            let x0 = (x * 2).min(last_x);
            let x1 = (x * 2 + 1).min(last_x);
            for channel in 0..4 {
                let at = |x: u32, y: u32| {
                    source[((y as usize * width as usize) + x as usize) * 4 + channel]
                };
                output.push((at(x0, y0) + at(x1, y0) + at(x0, y1) + at(x1, y1)) * 0.25);
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{ColourSpace, decode_in, encode_in, mip_chain, mip_level_count, reduce, resample};

    #[test]
    fn the_linear_round_trip_is_the_identity_and_the_srgb_one_is_not() {
        // The property that makes a normal or roughness map survive a resample. Every byte, because the
        // failure worth catching is a transfer function applied to data that never carried one -- and it
        // is worst in the mid range, where sRGB and linear differ by more than a factor of two.
        let bytes: Vec<u8> = (0u8..=255).collect();
        let round_tripped = encode_in(&decode_in(&bytes, ColourSpace::Linear), ColourSpace::Linear);
        assert_eq!(round_tripped, bytes, "linear must not transform anything");
        // And the sRGB path genuinely does transform: 128 decodes to 0.216, not 0.502.
        let mid = decode_in(&[128, 128, 128, 128], ColourSpace::Srgb);
        assert!(
            (mid[0] - 0.216).abs() < 0.002,
            "sRGB mid grey is 0.216 in linear light, got {}",
            mid[0]
        );
        // Alpha is a coverage fraction in either space, so it is never gamma encoded.
        assert!((mid[3] - 128.0 / 255.0).abs() < 1.0e-6);
    }

    #[test]
    fn a_linear_mip_reduction_averages_the_stored_values() {
        // The same fixture as the sRGB case below, and the correct answer is the *other* one: for data
        // that was never encoded, the mean of 0 and 255 is 128 rather than 188. Getting this wrong is
        // invisible at the base level and shows up only as the camera pulls back, which is why it is
        // pinned rather than left to the format alone.
        let reduced = encode_in(
            &reduce(&decode_in(&black_and_white(), ColourSpace::Linear), 2, 2),
            ColourSpace::Linear,
        );
        assert_eq!(reduced[0], 128, "a linear average of 0 and 255");
    }

    #[test]
    fn reduction_averages_in_linear_light_not_in_stored_bytes() {
        // The whole reason the mip generator round-trips through linear. Half black and half white,
        // reduced to one texel: the correct answer is mid *grey in linear terms*, which encodes to
        // about 188 -- far from the 128 a naive average of the stored bytes produces.
        let reduced = encode_in(
            &reduce(&decode_in(&black_and_white(), ColourSpace::Srgb), 2, 2),
            ColourSpace::Srgb,
        );
        assert_eq!(reduced.len(), 4);
        assert!(
            (185..=190).contains(&reduced[0]),
            "expected a linear-light average near 188, got {}",
            reduced[0]
        );
    }

    /// A 2x2 checker of opaque black and opaque white.
    fn black_and_white() -> Vec<u8> {
        [[0, 0, 0, 255], [255, 255, 255, 255]]
            .into_iter()
            .cycle()
            .take(4)
            .flatten()
            .collect()
    }

    #[test]
    fn a_mip_chain_reaches_one_by_one() {
        assert_eq!(mip_level_count(1, 1), 1);
        assert_eq!(mip_level_count(2, 1), 2);
        assert_eq!(mip_level_count(256, 64), 9);
        // A non-power-of-two largest dimension still terminates, one level past its floor log.
        assert_eq!(mip_level_count(300, 7), 9);
    }

    #[test]
    fn a_generated_chain_has_every_level_at_its_own_size() {
        // A non-square base, because the failure worth catching is a chain that halves one axis and not
        // the other -- which produces the right *number* of levels and the wrong pixels in each.
        let chain = mip_chain(&[64; 8 * 4 * 4], 8, 4, ColourSpace::Linear);
        let sizes: Vec<(u32, u32)> = chain.iter().map(|(w, h, _)| (*w, *h)).collect();
        assert_eq!(sizes, [(8, 4), (4, 2), (2, 1), (1, 1)]);
        for (width, height, rgba) in &chain {
            assert_eq!(rgba.len(), (*width as usize) * (*height as usize) * 4);
            // A constant image must stay constant at every level, whatever the axis ratio does.
            assert!(
                rgba.iter().all(|byte| *byte == 64),
                "level {width}x{height}"
            );
        }
    }

    #[test]
    fn a_chain_of_a_degenerate_image_is_empty_rather_than_a_panic() {
        assert!(mip_chain(&[], 0, 4, ColourSpace::Srgb).is_empty());
        assert!(mip_chain(&[], 4, 0, ColourSpace::Srgb).is_empty());
    }

    #[test]
    fn resampling_preserves_a_constant_colour() {
        // A constant image must survive any resize exactly. Interpolation weights that fail to sum to
        // one show up here as a shift the eye reads as a darkened texture.
        let flat: Vec<f32> = std::iter::repeat_n(0.25f32, 4 * 4 * 4).collect();
        for value in resample(&flat, 4, 4, 7, 3) {
            assert!((value - 0.25).abs() < 1.0e-6, "got {value}");
        }
    }
}
