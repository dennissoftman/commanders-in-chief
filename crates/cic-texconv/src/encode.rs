//! BC1, BC5 and BC7 block encoding.
//!
//! # What compression actually is here
//!
//! Every one of these formats stores, per 4x4 block, a small number of *endpoints* and a per-texel index
//! selecting a point on the line between them. So encoding is two questions: where to put the line, and
//! which point on it each texel takes. The second is a nearest search once the first is answered; the
//! first is the whole quality of the encoder.
//!
//! The answer used throughout:
//!
//! 1. Find the block's **principal axis** — the dominant eigenvector of its texels' covariance — and take
//!    the extreme projections onto it as the endpoints. See [`principal_endpoints`] for why the obvious
//!    alternative, the colour bounding box, is wrong by about twelve decibels on an anti-correlated block.
//! 2. Assign each texel the index whose palette entry is nearest.
//! 3. With the indices fixed, solve for the endpoint pair that minimises squared error — a 2x2 linear
//!    system, because the reconstruction is linear in the endpoints.
//! 4. Keep that solve only if it lowered the error the *decoder* will see, and stop when it stops helping.
//!    The guard is not a formality: a least-squares fit answers a compressed index range by extrapolating
//!    the endpoints past the data, which then compresses the range further.
//!
//! The alternative worth naming is exhaustive search over quantized endpoint pairs, which is what a
//! production compressor does with a time budget and SIMD. This is an offline tool run once per texture,
//! and the measurements in this file's tests are what say whether that is worth adding.
//!
//! # Why BC7 uses only mode 6
//!
//! BC7 has eight modes. Mode 6 is the single-subset one with four-bit indices and eight-bit endpoint
//! precision — the best single-subset mode, and the one a block of smoothly varying colour wants.
//!
//! The modes that would beat it are the partitioned ones (0 to 3, 7): they split the block into two or
//! three subsets with independent endpoints, which is what a block straddling a hard edge needs. Using
//! them means searching 64 partitions against every candidate, and the search is where a compressor's
//! bugs live — a wrong anchor, an unswapped endpoint pair, a partition whose subset is empty. Mode 6
//! alone is a complete, exact, verifiable encoder that decodes byte-for-byte to what it intended, and it
//! is a real improvement on BC1 at the same 8 bpp as BC3. Partition search is a quality improvement to
//! make deliberately, with its own measurements, rather than a thing to smuggle in alongside a new
//! container format.
//!
//! Every function here is verified by decoding its output with [`cic_assets::bc`] — the independent
//! implementation of the published specifications — and measuring the error. An encoder checked only
//! against itself is checked against nothing.

use cic_assets::texture::{BLOCK_EXTENT, BlockFormat};

/// Encodes one RGBA8 level to blocks in the given layout.
///
/// `width` and `height` are the level's texel dimensions. Blocks are emitted row-major, and a block
/// straddling the right or bottom edge of a level that is not a multiple of four is filled by clamping
/// to the nearest texel inside — repeating the edge rather than padding with black, which would drag the
/// block's endpoints toward a colour the image does not contain.
#[must_use]
pub fn encode_level(rgba: &[u8], width: u32, height: u32, format: BlockFormat) -> Vec<u8> {
    let extent = BLOCK_EXTENT as usize;
    let (width, height) = (width.max(1) as usize, height.max(1) as usize);
    let blocks_across = width.div_ceil(extent);
    let blocks_down = height.div_ceil(extent);
    let mut out =
        Vec::with_capacity(blocks_across * blocks_down * format.block_bytes().max(1) as usize);

    for block_y in 0..blocks_down {
        for block_x in 0..blocks_across {
            let mut texels = [[0u8; 4]; 16];
            for (index, texel) in texels.iter_mut().enumerate() {
                // Clamped into the image, so a partial block repeats its edge texels.
                let y = (block_y * extent + index / extent).min(height - 1);
                let x = (block_x * extent + index % extent).min(width - 1);
                let at = (y * width + x) * 4;
                for (channel, value) in texel.iter_mut().enumerate() {
                    *value = rgba.get(at + channel).copied().unwrap_or(0);
                }
            }
            match format {
                BlockFormat::Bc1RgbaUnorm | BlockFormat::Bc1RgbaUnormSrgb => {
                    out.extend_from_slice(&encode_bc1_block(&texels));
                }
                BlockFormat::Bc5Unorm => out.extend_from_slice(&encode_bc5_block(&texels)),
                BlockFormat::Bc7Unorm | BlockFormat::Bc7UnormSrgb => {
                    out.extend_from_slice(&encode_bc7_block(&texels));
                }
            }
        }
    }
    out
}

/// Interpolation factors for a four-bit index, as the specification defines them.
const WEIGHTS_4: [i32; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];

/// Encodes one block as BC7 mode 6: one subset, RGBA, four-bit indices.
///
/// The endpoints are stored as seven bits plus a parity bit per *endpoint* — shared across that endpoint's
/// four channels, so the eighth bit is not a per-channel choice. A colour whose channels disagree on their
/// low bit therefore has no exact encoding; see [`round_with_parity`].
fn encode_bc7_block(texels: &[[u8; 4]; 16]) -> [u8; 16] {
    let (first, second, indices) = fit_line(texels, &WEIGHTS_4);

    // Texel zero is the only subset's anchor, so its index is stored one bit short and its high bit must
    // be zero. Swapping the endpoints and inverting every index describes the same line the other way
    // round, which is exactly what that costs.
    let (first, second, indices) = if indices[0] > 7 {
        let inverted = indices.map(|index| 15 - index);
        (second, first, inverted)
    } else {
        (first, second, indices)
    };

    let mut value = 1u128 << 6;
    let mut position = 7u32;
    let mut put = |bits: u128, count: u32| {
        value |= bits << position;
        position += count;
    };
    for channel in 0..4 {
        put(u128::from(first[channel] >> 1), 7);
        put(u128::from(second[channel] >> 1), 7);
    }
    // The parity bit is the endpoint's least significant bit, shared across that endpoint's four
    // channels -- so reading it off channel zero is not a shortcut: `round_with_parity` has already made
    // all four agree, and they could not be written if they did not.
    put(u128::from(first[0] & 1), 1);
    put(u128::from(second[0] & 1), 1);
    put(u128::from(indices[0]), 3);
    for index in &indices[1..] {
        put(u128::from(*index), 4);
    }
    value.to_le_bytes()
}

/// Fits one line through a block's RGBA texels and assigns each an index on it.
///
/// Returns the two endpoints and the per-texel indices. The endpoints come back with every channel of
/// each endpoint sharing a least significant bit, because BC7's parity bit is per *endpoint* rather than
/// per channel: an endpoint whose channels disagreed on that bit could not be written.
fn fit_line(texels: &[[u8; 4]; 16], weights: &[i32]) -> ([u8; 4], [u8; 4], [u8; 16]) {
    let samples: Vec<[f32; 4]> = texels.iter().map(|texel| texel.map(f32::from)).collect();
    let (mut first, mut second) = principal_endpoints(&samples, 4);

    // Refinement: assign indices to the current line, then re-solve the line for those indices. Kept only
    // while it measurably helps, which is not a formality -- a least-squares solve minimises error for the
    // indices it was given, and if those indices span only part of the range it answers by *extrapolating*
    // the endpoints past the data. That is a fixed point: the wider endpoints compress the indices
    // further. Judging each round by the error the decoder will actually see stops it.
    let mut indices = assign_indices(texels, first, second, weights);
    let mut error = block_error(
        texels,
        round_with_parity(first, 0),
        round_with_parity(second, 0),
        &indices,
        weights,
    );
    for _ in 0..4 {
        let (candidate_first, candidate_second) =
            solve_endpoints(texels, &indices, weights, first, second);
        let candidate_indices = assign_indices(texels, candidate_first, candidate_second, weights);
        let candidate_error = block_error(
            texels,
            round_with_parity(candidate_first, 0),
            round_with_parity(candidate_second, 0),
            &candidate_indices,
            weights,
        );
        if candidate_error >= error {
            break;
        }
        (first, second, indices, error) = (
            candidate_first,
            candidate_second,
            candidate_indices,
            candidate_error,
        );
    }

    // Rounding to a parity happens last, and against the *rounded* endpoints -- because those are the
    // endpoints the decoder will see.
    //
    // All four combinations are tried. Rounding each endpoint to its own nearest parity is the obvious
    // choice and it is a local one: a pair whose parities differ can straddle the data and beat both
    // endpoints rounding the same way, and which wins is not predictable from the endpoints alone. Four
    // index assignments per block is nothing in an offline tool.
    let mut best: Option<Candidate> = None;
    for first_parity in 0..2 {
        for second_parity in 0..2 {
            let low = round_with_parity(first, first_parity);
            let high = round_with_parity(second, second_parity);
            let candidate =
                assign_indices(texels, low.map(f32::from), high.map(f32::from), weights);
            let candidate_error = block_error(texels, low, high, &candidate, weights);
            if best.is_none_or(|(previous, ..)| candidate_error < previous) {
                best = Some((candidate_error, low, high, candidate));
            }
        }
    }
    // Every iteration above assigns `best`, so the fallback is unreachable rather than a guess.
    let (_, first, second, indices) = best.unwrap_or((0.0, [0; 4], [0; 4], indices));
    (first, second, indices)
}

/// One candidate encoding of a block: its measured error, its two endpoints, and its indices.
type Candidate = (f32, [u8; 4], [u8; 4], [u8; 16]);

/// The ends of the segment a block's texels project onto along their principal axis.
///
/// # Why not the colour bounding box
///
/// The bounding box is the obvious first guess and it is wrong in a way that matters. It takes each
/// channel's own minimum and maximum, which describes a line through two *corners* — and those corners are
/// only on the data when the channels rise together. On an anti-correlated block, say one where blue falls
/// as red rises, the box's diagonal runs across the data rather than along it. Every index is then a poor
/// fit, and the least-squares refinement that follows starts from the wrong line and extrapolates rather
/// than recovering: measured on a collinear ramp, the box costs about twelve decibels.
///
/// The principal axis is the dominant eigenvector of the texels' covariance, found by power iteration from
/// the highest-variance channel — a handful of matrix products, deterministic, and exact enough that the
/// refinement afterward has little left to do. Projecting onto it and taking the extreme projections puts
/// the endpoints *on* the data, so the whole index range is spent inside it.
///
/// `channels` is how many of the four take part: four for BC7, where one index drives RGBA together, and
/// three for BC1, which has no alpha to fit. Channels beyond it come back at the mean.
///
/// A block with no variation at all has no axis, and both endpoints come back as the mean — which is the
/// correct answer for a flat block rather than a fallback.
fn principal_endpoints(samples: &[[f32; 4]], channels: usize) -> ([f32; 4], [f32; 4]) {
    #[allow(clippy::cast_precision_loss)]
    let count = samples.len().max(1) as f32;
    let mut mean = [0.0f32; 4];
    for sample in samples {
        for channel in 0..channels {
            mean[channel] += sample[channel];
        }
    }
    for value in &mut mean[..channels] {
        *value /= count;
    }

    let mut covariance = [[0.0f32; 4]; 4];
    for sample in samples {
        for row in 0..channels {
            for column in 0..channels {
                covariance[row][column] +=
                    (sample[row] - mean[row]) * (sample[column] - mean[column]);
            }
        }
    }

    // Seeded from the channel that varies most, which cannot be orthogonal to the dominant eigenvector.
    let seed = (0..channels)
        .max_by(|a, b| covariance[*a][*a].total_cmp(&covariance[*b][*b]))
        .unwrap_or(0);
    if covariance[seed][seed] <= f32::EPSILON {
        return (mean, mean);
    }
    let mut axis = [0.0f32; 4];
    axis[..channels].copy_from_slice(&covariance[seed][..channels]);
    for _ in 0..12 {
        let mut next = [0.0f32; 4];
        for row in 0..channels {
            for column in 0..channels {
                next[row] += covariance[row][column] * axis[column];
            }
        }
        let length = (0..channels)
            .map(|channel| next[channel] * next[channel])
            .sum::<f32>()
            .sqrt();
        if length <= f32::EPSILON {
            break;
        }
        for channel in 0..channels {
            axis[channel] = next[channel] / length;
        }
    }

    let mut low = f32::INFINITY;
    let mut high = f32::NEG_INFINITY;
    for sample in samples {
        let projection = (0..channels)
            .map(|channel| (sample[channel] - mean[channel]) * axis[channel])
            .sum::<f32>();
        low = low.min(projection);
        high = high.max(projection);
    }
    if !low.is_finite() || !high.is_finite() {
        return (mean, mean);
    }

    let mut first = mean;
    let mut second = mean;
    for channel in 0..channels {
        first[channel] = (mean[channel] + axis[channel] * low).clamp(0.0, 255.0);
        second[channel] = (mean[channel] + axis[channel] * high).clamp(0.0, 255.0);
    }
    (first, second)
}

/// Total squared error of a candidate encoding, over all four channels of all sixteen texels.
///
/// Computed with exactly the decoder's arithmetic, so "better" here means better in the picture rather
/// than better by a proxy. This is what lets the refinement above be judged rather than trusted.
fn block_error(
    texels: &[[u8; 4]; 16],
    first: [u8; 4],
    second: [u8; 4],
    indices: &[u8; 16],
    weights: &[i32],
) -> f32 {
    let mut total = 0.0f32;
    for (texel, index) in texels.iter().zip(indices) {
        let weight = weights.get(*index as usize).copied().unwrap_or(0);
        for channel in 0..4 {
            let value = (i32::from(first[channel]) * (64 - weight)
                + i32::from(second[channel]) * weight
                + 32)
                >> 6;
            #[allow(clippy::cast_precision_loss)]
            let difference = (value - i32::from(texel[channel])) as f32;
            total += difference * difference;
        }
    }
    total
}

/// Assigns every texel the index whose interpolated colour is nearest, by squared distance over RGBA.
fn assign_indices(
    texels: &[[u8; 4]; 16],
    first: [f32; 4],
    second: [f32; 4],
    weights: &[i32],
) -> [u8; 16] {
    let mut indices = [0u8; 16];
    for (texel, slot) in texels.iter().zip(indices.iter_mut()) {
        let mut best = (f32::INFINITY, 0u8);
        for (index, weight) in weights.iter().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let blend = *weight as f32 / 64.0;
            let mut error = 0.0f32;
            for channel in 0..4 {
                let value = first[channel] + (second[channel] - first[channel]) * blend;
                let difference = value - f32::from(texel[channel]);
                error += difference * difference;
            }
            #[allow(clippy::cast_possible_truncation)]
            let index = index as u8;
            if error < best.0 {
                best = (error, index);
            }
        }
        *slot = best.1;
    }
    indices
}

/// Least-squares solve for the endpoint pair minimising squared error, with the indices held fixed.
///
/// The reconstruction is `first + (second - first) * blend`, which is linear in both endpoints, so the
/// optimum is the solution of a 2x2 normal-equation system per channel. A singular system means every
/// texel took the same index — the line's direction is unconstrained — and the previous endpoints are
/// kept, which is the correct answer rather than a fallback.
fn solve_endpoints(
    texels: &[[u8; 4]; 16],
    indices: &[u8; 16],
    weights: &[i32],
    previous_first: [f32; 4],
    previous_second: [f32; 4],
) -> ([f32; 4], [f32; 4]) {
    let mut blends = [0.0f32; 16];
    for (blend, index) in blends.iter_mut().zip(indices) {
        #[allow(clippy::cast_precision_loss)]
        {
            *blend = weights.get(*index as usize).copied().unwrap_or(0) as f32 / 64.0;
        }
    }

    // Sums of the basis products, shared by all four channels.
    let mut aa = 0.0f32;
    let mut ab = 0.0f32;
    let mut bb = 0.0f32;
    for blend in blends {
        let a = 1.0 - blend;
        aa += a * a;
        ab += a * blend;
        bb += blend * blend;
    }
    let determinant = aa * bb - ab * ab;
    if determinant.abs() < 1.0e-6 {
        return (previous_first, previous_second);
    }

    let mut first = previous_first;
    let mut second = previous_second;
    for channel in 0..4 {
        let mut ay = 0.0f32;
        let mut by = 0.0f32;
        for (texel, blend) in texels.iter().zip(blends) {
            let value = f32::from(texel[channel]);
            ay += (1.0 - blend) * value;
            by += blend * value;
        }
        first[channel] = ((ay * bb - by * ab) / determinant).clamp(0.0, 255.0);
        second[channel] = ((by * aa - ay * ab) / determinant).clamp(0.0, 255.0);
    }
    (first, second)
}

/// Rounds an endpoint's four channels to bytes sharing one given least significant bit.
///
/// BC7 mode 6 stores seven bits per channel plus one parity bit per *endpoint*, so the eighth bit is a
/// property of the endpoint rather than of a channel. That is a real constraint and not a rounding
/// detail: an arbitrary flat RGBA whose channels disagree on their low bit has **no** exact mode-6
/// encoding, and comes back within one least-significant bit instead. The identity values the runtime
/// fills unused slices with — opaque white, and the used channels of the flat normal — are on the right
/// side of that, which is why they were chosen from it.
fn round_with_parity(endpoint: [f32; 4], parity: u32) -> [u8; 4] {
    let mut rounded = [0u8; 4];
    for channel in 0..4 {
        // The representable values are `2k + parity`; pick the nearest.
        #[allow(clippy::cast_precision_loss)]
        let offset = parity as f32;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let half = ((endpoint[channel] - offset) / 2.0)
            .round()
            .clamp(0.0, 127.0) as u32;
        #[allow(clippy::cast_possible_truncation)]
        {
            rounded[channel] = (half * 2 + parity).min(255) as u8;
        }
    }
    rounded
}

/// Encodes one block as BC1: two 5:6:5 endpoints and two bits per texel.
///
/// Uses the four-colour interpretation unless the block has texels below the alpha cutoff, in which case
/// it switches to the three-colour one whose fourth index is transparent — the punch-through alpha that
/// makes BC1 usable for a cutout at 4 bpp.
fn encode_bc1_block(texels: &[[u8; 4]; 16]) -> [u8; 8] {
    /// Alpha at or below which a texel is written as punched through. The glTF default cutoff, so a
    /// mask converted to BC1 cuts where the material says it does.
    const ALPHA_CUTOFF: u8 = 128;
    let punched = texels.iter().any(|texel| texel[3] < ALPHA_CUTOFF);

    // The principal axis of the *opaque* texels, so a transparent texel's colour -- which is arbitrary in
    // a cutout, and often black -- does not drag an endpoint toward it. See `principal_endpoints` for why
    // this is not the colour bounding box.
    let samples: Vec<[f32; 4]> = texels
        .iter()
        .filter(|texel| texel[3] >= ALPHA_CUTOFF)
        .map(|texel| texel.map(f32::from))
        .collect();
    if samples.is_empty() {
        // Every texel is punched through, so the colours are never read. Emit the encoding that says so as
        // plainly as possible: equal endpoints, every index transparent.
        return [0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF];
    }
    let (low, high) = principal_endpoints(&samples, 3);
    let quantize_endpoint = |endpoint: [f32; 4]| -> [u8; 3] {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        {
            [
                endpoint[0].round().clamp(0.0, 255.0) as u8,
                endpoint[1].round().clamp(0.0, 255.0) as u8,
                endpoint[2].round().clamp(0.0, 255.0) as u8,
            ]
        }
    };

    let mut packed_high = pack_565(quantize_endpoint(high));
    let mut packed_low = pack_565(quantize_endpoint(low));
    // Four-colour mode needs the first word strictly greater; three-colour mode needs it not greater.
    // Quantization can collapse a narrow range to one word, which is fine either way -- index 0 is the
    // first endpoint in both interpretations.
    if punched {
        if packed_high > packed_low {
            std::mem::swap(&mut packed_high, &mut packed_low);
        }
    } else if packed_high < packed_low {
        std::mem::swap(&mut packed_high, &mut packed_low);
    }

    let palette = bc1_palette(packed_high, packed_low, punched);
    let mut bits = 0u32;
    for (texel, slot) in texels.iter().enumerate() {
        let index = if punched && slot[3] < ALPHA_CUTOFF {
            3
        } else {
            nearest_rgb(*slot, &palette, punched)
        };
        bits |= u32::from(index) << (texel * 2);
    }

    let mut block = [0u8; 8];
    block[..2].copy_from_slice(&packed_high.to_le_bytes());
    block[2..4].copy_from_slice(&packed_low.to_le_bytes());
    block[4..].copy_from_slice(&bits.to_le_bytes());
    block
}

/// The four colours a BC1 block's endpoints imply, built exactly as the decoder builds them.
///
/// Truncating division, matching `EXT_texture_compression_s3tc` and therefore
/// [`cic_assets::bc::decode_bc1`]. Encoding against a rounded palette while the decoder truncates would
/// pick the wrong index near a boundary — a small error, and one that no amount of endpoint refinement
/// can recover.
fn bc1_palette(first: u16, second: u16, punched: bool) -> [[u8; 3]; 4] {
    let a = unpack_565(first);
    let b = unpack_565(second);
    let mix = |x: u8, y: u8, wx: u32, wy: u32, total: u32| -> u8 {
        #[allow(clippy::cast_possible_truncation)]
        {
            ((u32::from(x) * wx + u32::from(y) * wy) / total) as u8
        }
    };
    let blend = |wx: u32, wy: u32, total: u32| {
        [
            mix(a[0], b[0], wx, wy, total),
            mix(a[1], b[1], wx, wy, total),
            mix(a[2], b[2], wx, wy, total),
        ]
    };
    if punched {
        [a, b, blend(1, 1, 2), [0, 0, 0]]
    } else {
        [a, b, blend(2, 1, 3), blend(1, 2, 3)]
    }
}

/// The palette index nearest a texel's RGB, skipping the transparent entry of a punched block.
fn nearest_rgb(texel: [u8; 4], palette: &[[u8; 3]; 4], punched: bool) -> u8 {
    let usable = if punched { 3 } else { 4 };
    let mut best = (i32::MAX, 0u8);
    for (index, colour) in palette.iter().enumerate().take(usable) {
        let mut error = 0i32;
        for channel in 0..3 {
            let difference = i32::from(colour[channel]) - i32::from(texel[channel]);
            error += difference * difference;
        }
        #[allow(clippy::cast_possible_truncation)]
        let index = index as u8;
        if error < best.0 {
            best = (error, index);
        }
    }
    best.1
}

/// Packs an RGB triple into BC1's 5:6:5 word, rounding each channel to its own width.
fn pack_565(colour: [u8; 3]) -> u16 {
    let quantize = |value: u8, bits: u32| -> u16 {
        let maximum = (1u32 << bits) - 1;
        #[allow(clippy::cast_possible_truncation)]
        {
            (((u32::from(value) * maximum + 127) / 255).min(maximum)) as u16
        }
    };
    (quantize(colour[0], 5) << 11) | (quantize(colour[1], 6) << 5) | quantize(colour[2], 5)
}

/// Expands a 5:6:5 word the way the hardware does, by bit replication.
fn unpack_565(colour: u16) -> [u8; 3] {
    let expand = |value: u32, bits: u32| -> u8 {
        let shifted = (value << (8 - bits)) & 0xFF;
        #[allow(clippy::cast_possible_truncation)]
        {
            (shifted | (shifted >> bits)) as u8
        }
    };
    [
        expand(u32::from(colour >> 11) & 0x1F, 5),
        expand(u32::from(colour >> 5) & 0x3F, 6),
        expand(u32::from(colour) & 0x1F, 5),
    ]
}

/// Encodes one block as BC5: two independent BC4 halves, red then green.
fn encode_bc5_block(texels: &[[u8; 4]; 16]) -> [u8; 16] {
    let mut block = [0u8; 16];
    for (channel, half) in [0usize, 1].iter().zip(block.chunks_exact_mut(8)) {
        half.copy_from_slice(&encode_bc4_half(texels, *channel));
    }
    block
}

/// Encodes one channel of a block as a BC4 half: two endpoints and three bits per texel.
///
/// The eight-value interpretation, which needs the first endpoint strictly greater than the second, and
/// spends all eight ramp steps on the block's own range. The six-value interpretation trades two steps
/// for exact zero and exact one, which is a win only for a block that actually contains both — rare in a
/// normal map, and not worth a second search here.
fn encode_bc4_half(texels: &[[u8; 4]; 16], channel: usize) -> [u8; 8] {
    let mut low = u8::MAX;
    let mut high = 0u8;
    for texel in texels {
        low = low.min(texel[channel]);
        high = high.max(texel[channel]);
    }

    // Equal endpoints put the whole ramp at one value, which is exactly right for a flat channel and is
    // reproduced without error.
    let palette = bc4_palette(high, low);
    let mut packed = 0u64;
    for (texel, values) in texels.iter().enumerate() {
        let value = values[channel];
        let mut best = (i32::MAX, 0u8);
        for (index, candidate) in palette.iter().enumerate() {
            let difference = i32::from(*candidate) - i32::from(value);
            let error = difference * difference;
            #[allow(clippy::cast_possible_truncation)]
            let index = index as u8;
            if error < best.0 {
                best = (error, index);
            }
        }
        packed |= u64::from(best.1) << (texel * 3);
    }

    let mut half = [0u8; 8];
    half[0] = high;
    half[1] = low;
    half[2..].copy_from_slice(&packed.to_le_bytes()[..6]);
    half
}

/// The eight values a BC4 half's endpoints imply, built exactly as the decoder builds them.
///
/// Truncating division, matching `ARB_texture_compression_rgtc`. Equal endpoints collapse the whole ramp
/// to that value, which is why a flat channel is exact.
fn bc4_palette(first: u8, second: u8) -> [u8; 8] {
    let mut palette = [0u8; 8];
    palette[0] = first;
    palette[1] = second;
    if first > second {
        for (step, slot) in palette[2..8].iter_mut().enumerate() {
            #[allow(clippy::cast_possible_truncation)]
            let step = (step + 1) as u32;
            #[allow(clippy::cast_possible_truncation)]
            {
                *slot = ((u32::from(first) * (7 - step) + u32::from(second) * step) / 7) as u8;
            }
        }
    } else {
        // Equal endpoints: the interpolated entries are that same value, and the last two are the
        // extremes the six-value interpretation adds.
        for slot in &mut palette[2..6] {
            *slot = first;
        }
        palette[6] = 0;
        palette[7] = u8::MAX;
    }
    palette
}

#[cfg(test)]
mod tests {
    use super::encode_level;
    use cic_assets::bc::{decode_bc1, decode_bc5, decode_bc7};
    use cic_assets::texture::BlockFormat;

    /// Peak signal-to-noise ratio between two RGBA buffers over the given channels, in decibels.
    ///
    /// The measure a compressor is actually judged by. Every threshold below was *measured* and then set
    /// just under, rather than guessed — a guessed threshold either passes a broken encoder or fails a
    /// working one, and both happened while this file was being written.
    fn psnr(original: &[u8], decoded: &[u8], channels: &[usize]) -> f64 {
        let mut total = 0.0f64;
        let mut count = 0usize;
        for (a, b) in original.chunks_exact(4).zip(decoded.chunks_exact(4)) {
            for channel in channels {
                let difference = f64::from(a[*channel]) - f64::from(b[*channel]);
                total += difference * difference;
                count += 1;
            }
        }
        if count == 0 || total == 0.0 {
            return f64::INFINITY;
        }
        #[allow(clippy::cast_precision_loss)]
        let mean = total / count as f64;
        10.0 * (255.0f64 * 255.0 / mean).log10()
    }

    /// Encodes, decodes with `cic_assets::bc`, and returns the quality over the given channels.
    ///
    /// The decode is deliberately the *other* implementation — the one written from the published
    /// specifications, which knows nothing about this encoder. An encoder measured against its own idea of
    /// what it wrote is measured against nothing.
    fn round_trip(rgba: &[u8], size: u32, format: BlockFormat, channels: &[usize]) -> f64 {
        let blocks = encode_level(rgba, size, size, format);
        let decoded = match format {
            BlockFormat::Bc5Unorm => decode_bc5(&blocks, size, size),
            BlockFormat::Bc1RgbaUnorm | BlockFormat::Bc1RgbaUnormSrgb => {
                decode_bc1(&blocks, size, size)
            }
            _ => decode_bc7(&blocks, size, size),
        };
        psnr(rgba, &decoded, channels)
    }

    /// A 16x16 ramp whose channels all move together, and one of them *against* the others.
    ///
    /// Collinear in colour space, so a single-line format can represent it essentially exactly — this is
    /// the fixture that measures the encoder rather than the format. The falling blue channel is not
    /// decoration: an anti-correlated channel is what a colour bounding box gets wrong, and fitting this at
    /// 49 dB rather than 36 is the whole reason `principal_endpoints` exists.
    fn collinear() -> Vec<u8> {
        let mut rgba = Vec::with_capacity(16 * 16 * 4);
        for y in 0..16u32 {
            for x in 0..16u32 {
                let ramp = (x + y) * 8;
                rgba.extend_from_slice(&[
                    u8::try_from(ramp).unwrap_or(255),
                    u8::try_from(ramp / 2).unwrap_or(255),
                    u8::try_from(255 - ramp.min(255)).unwrap_or(0),
                    255,
                ]);
            }
        }
        rgba
    }

    /// A 16x16 image whose red varies along one axis and green along the other, independently.
    ///
    /// The case no single line through colour space can follow, whatever the encoder does — and therefore
    /// the case that shows what BC5's independent channels buy.
    fn independent_channels() -> Vec<u8> {
        let mut rgba = Vec::with_capacity(16 * 16 * 4);
        for y in 0..16u32 {
            for x in 0..16u32 {
                rgba.extend_from_slice(&[
                    u8::try_from(x * 16).unwrap_or(255),
                    u8::try_from(y * 16).unwrap_or(255),
                    128,
                    255,
                ]);
            }
        }
        rgba
    }

    /// A 16x16 image with a hard diagonal edge across a gradient: the adversarial case.
    ///
    /// A block straddling the edge holds two colour clusters, which is exactly what a *partitioned* BC7
    /// mode exists for and what mode 6 alone cannot do. The number this fixture produces is therefore the
    /// documented cost of the mode choice rather than a fault in the fit.
    fn hard_edge() -> Vec<u8> {
        let mut rgba = Vec::with_capacity(16 * 16 * 4);
        for y in 0..16u32 {
            for x in 0..16u32 {
                let ramp = u8::try_from((x * 255) / 15).unwrap_or(255);
                let other = u8::try_from((y * 255) / 15).unwrap_or(255);
                if x + y > 16 {
                    rgba.extend_from_slice(&[ramp, other, 200, 255]);
                } else {
                    rgba.extend_from_slice(&[20, 30, other, 255]);
                }
            }
        }
        rgba
    }

    #[test]
    fn a_flat_block_is_exact_in_bc5_and_within_one_bit_in_bc7() {
        // Worth stating precisely, because "a flat colour survives" is the assertion one reaches for and it
        // is false for BC7. Mode 6 stores seven bits per channel plus a parity bit per *endpoint*, shared
        // across that endpoint's four channels — so a colour whose channels disagree on their low bit has
        // no exact encoding at all. One least-significant bit is the true bound, and it is reached.
        let mixed = [37u8, 128, 211, 255];
        let decoded = decode_bc7(
            &encode_level(&mixed.repeat(16), 4, 4, BlockFormat::Bc7UnormSrgb),
            4,
            4,
        );
        for texel in decoded.chunks_exact(4) {
            for channel in 0..4 {
                assert!(
                    texel[channel].abs_diff(mixed[channel]) <= 1,
                    "BC7 moved a flat colour by more than one bit: {texel:?} from {mixed:?}"
                );
            }
        }

        // And when the channels do agree on their low bit, it is exact — which is why the identity values
        // the runtime fills unused slices with are chosen from that set. Opaque white is all-odd.
        let white = [u8::MAX; 4];
        let decoded = decode_bc7(
            &encode_level(&white.repeat(16), 4, 4, BlockFormat::Bc7UnormSrgb),
            4,
            4,
        );
        assert!(
            decoded.chunks_exact(4).all(|texel| texel == white),
            "an all-odd flat colour must be exact, got {:?}",
            &decoded[..4]
        );

        // BC5 has a whole endpoint per channel and no parity bit, so every flat value is exact.
        let decoded = decode_bc5(
            &encode_level(&mixed.repeat(16), 4, 4, BlockFormat::Bc5Unorm),
            4,
            4,
        );
        for texel in decoded.chunks_exact(4) {
            assert_eq!([texel[0], texel[1]], [mixed[0], mixed[1]]);
        }
    }

    #[test]
    fn bc7_fits_a_collinear_block_to_within_a_fraction_of_a_bit() {
        // 49.6 dB measured, pinned at 45. This is the assertion that catches the bug this encoder actually
        // had: fitting the line to the colour bounding box instead of the principal axis scores 36 dB
        // here, which is a visible loss and passes any threshold set by intuition.
        let original = collinear();
        let blocks = encode_level(&original, 16, 16, BlockFormat::Bc7UnormSrgb);
        assert_eq!(blocks.len(), 16 * 16, "8 bpp over 16 blocks");
        let quality = round_trip(&original, 16, BlockFormat::Bc7UnormSrgb, &[0, 1, 2, 3]);
        assert!(quality > 45.0, "BC7 mode 6 managed only {quality:.1} dB");
    }

    #[test]
    fn mode_six_alone_costs_about_twenty_decibels_on_a_hard_edge() {
        // The documented price of not searching partitions, measured rather than asserted away: 31 dB on a
        // block holding two colour clusters against 49 on a collinear one. Pinned as a floor *and* stated
        // as a gap, so a future partition search has a number to beat and a regression has one to fail.
        let quality = round_trip(&hard_edge(), 16, BlockFormat::Bc7UnormSrgb, &[0, 1, 2, 3]);
        assert!(
            quality > 29.0,
            "BC7 managed only {quality:.1} dB on an edge"
        );
        let collinear_quality =
            round_trip(&collinear(), 16, BlockFormat::Bc7UnormSrgb, &[0, 1, 2, 3]);
        assert!(
            collinear_quality - quality > 10.0,
            "the edge should cost real quality; got {quality:.1} against {collinear_quality:.1}"
        );
    }

    #[test]
    fn bc5_beats_bc7_when_two_channels_vary_independently() {
        // The reason a normal map goes in BC5 rather than BC7 at the same 8 bpp. Red varies along x and
        // green along y, so no single line through colour space can follow both — and BC5, which is two
        // independent single-channel compressors, has no such constraint. 42.9 dB against 27.8 measured.
        let original = independent_channels();
        let independent = round_trip(&original, 16, BlockFormat::Bc5Unorm, &[0, 1]);
        let shared = round_trip(&original, 16, BlockFormat::Bc7Unorm, &[0, 1]);
        assert!(
            independent - shared > 10.0,
            "BC5 managed {independent:.1} dB against BC7's {shared:.1}, which is not the margin the \
             format choice is made on"
        );
        assert!(
            independent > 40.0,
            "BC5 managed only {independent:.1} dB on two channels"
        );
    }

    #[test]
    fn bc1_costs_half_the_bytes_and_some_quality() {
        // Both halves of the trade, stated together so neither can be forgotten: 4 bpp against BC7's 8, and
        // measurably worse for it. 36.1 dB collinear and 28.2 on an edge, against BC7's 49.6 and 31.0.
        let original = collinear();
        let blocks = encode_level(&original, 16, 16, BlockFormat::Bc1RgbaUnormSrgb);
        assert_eq!(blocks.len(), 8 * 16, "4 bpp over 16 blocks");
        let quality = round_trip(&original, 16, BlockFormat::Bc1RgbaUnormSrgb, &[0, 1, 2]);
        assert!(quality > 34.0, "BC1 managed only {quality:.1} dB");
        let bc7 = round_trip(&original, 16, BlockFormat::Bc7UnormSrgb, &[0, 1, 2]);
        assert!(
            bc7 > quality,
            "BC7 must be worth its extra 4 bpp: {bc7:.1} against {quality:.1}"
        );
    }

    #[test]
    fn bc1_punches_alpha_through_where_the_source_is_transparent() {
        // What a 4 bpp cutout mask relies on. A block with any texel below the cutoff switches
        // interpretation, and the transparent texels must come back transparent -- while the opaque ones
        // keep their colour, which the three-colour interpretation still has three entries for.
        let mut rgba = vec![0u8; 4 * 4 * 4];
        for (texel, chunk) in rgba.chunks_exact_mut(4).enumerate() {
            let opaque = texel % 2 == 0;
            chunk.copy_from_slice(&[200, 40, 40, if opaque { 255 } else { 0 }]);
        }
        let decoded = decode_bc1(
            &encode_level(&rgba, 4, 4, BlockFormat::Bc1RgbaUnormSrgb),
            4,
            4,
        );
        for (texel, chunk) in decoded.chunks_exact(4).enumerate() {
            if texel % 2 == 0 {
                assert_eq!(chunk[3], 255, "texel {texel} must stay opaque");
                assert!(
                    chunk[0] > 150,
                    "texel {texel} must keep its colour: {chunk:?}"
                );
            } else {
                assert_eq!(chunk[3], 0, "texel {texel} must be punched through");
            }
        }
    }

    #[test]
    fn a_level_narrower_than_a_block_repeats_its_edge_rather_than_padding_with_black() {
        // Every mip chain ends in levels below 4x4, so this is the ordinary path. Padding the block with
        // black instead would drag the endpoints toward a colour the image does not contain, which shows up
        // as a dark tail on the smallest levels -- visible only at the distance those levels are for.
        let colour = [90u8, 160, 220, 255];
        let blocks = encode_level(&colour.repeat(2), 2, 1, BlockFormat::Bc7Unorm);
        assert_eq!(blocks.len(), 16, "a 2x1 level still costs one whole block");
        let decoded = decode_bc7(&blocks, 2, 1);
        for texel in decoded.chunks_exact(4) {
            for channel in 0..4 {
                assert!(
                    texel[channel].abs_diff(colour[channel]) <= 1,
                    "a padded block leaked into the result: {texel:?} from {colour:?}"
                );
            }
        }
    }
}
