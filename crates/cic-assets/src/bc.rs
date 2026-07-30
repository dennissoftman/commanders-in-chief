//! BC1, BC5 and BC7 block decoding to straight-alpha RGBA8.
//!
//! # Why a software decoder exists at all
//!
//! Block compression is a *hardware* format: the texture unit decompresses it on read, so the whole
//! point is that these bytes reach the GPU untouched. This module is what happens when they cannot.
//!
//! 1. **Adapters without `TEXTURE_COMPRESSION_BC`.** `wgpu` correctly refuses a format the adapter does not
//!    advertise, and a renderer that could only draw block-compressed textures where the hardware
//!    decompresses them would be a renderer that cannot run everywhere it is asked to. (The CI runner is
//!    *not* such an adapter, as it happens — Mesa's `llvmpipe` advertises the feature and decompresses in
//!    software — but a device that lacks it is an ordinary thing rather than a hypothetical one.)
//! 2. **Being able to assert on pixels.** A test can say what colour a block decodes to. It cannot say
//!    what a texture unit did, and the reference captures in `cic-render` are the wrong instrument for
//!    "is this block encoded correctly" because they answer it through six other passes.
//!
//! So this is a fallback and an oracle, not a fast path. It is written for exactness against the
//! specification rather than for speed, and nothing in a frame calls it.
//!
//! # Where the constants come from
//!
//! The mode table, the partition tables, the anchor tables and the interpolation weights are the
//! specification's own, transcribed from the BPTC/BC7 definition in
//! `ARB_texture_compression_bptc`. They are arbitrary — hand-chosen partitions and a
//! hand-chosen weight ramp — so there is nothing to derive and nothing to sanity-check by inspection.
//! What *can* be checked is their internal consistency, and
//! [`tests::the_partition_and_anchor_tables_agree_with_each_other`] does: every two-subset partition
//! must use both subsets, every three-subset partition all three, and every anchor must land on a texel
//! belonging to the subset it anchors. That catches the transcription error this module is most exposed
//! to, which is a single wrong digit in a thousand.

use crate::texture::BLOCK_EXTENT;

/// Decodes BC1 blocks to straight-alpha RGBA8, row-major from the top-left.
///
/// BC1 stores one RGB endpoint pair per block as 5:6:5 and two bits per texel. When the first endpoint
/// compares *below* the second, the block switches to a three-colour interpretation whose fourth index
/// is transparent black — the "punch-through" alpha that makes BC1 usable for a cutout mask at 4 bpp.
/// That comparison is on the raw 16-bit words, not on the expanded colours.
#[must_use]
pub fn decode_bc1(blocks: &[u8], width: u32, height: u32) -> Vec<u8> {
    decode_blocks(blocks, width, height, 8, decode_bc1_block)
}

/// Decodes BC5 blocks to straight-alpha RGBA8, row-major from the top-left.
///
/// Two independent BC4 halves, red then green. Blue is zero and alpha is opaque, which is what the
/// hardware returns for this format — a normal map's `z` is rebuilt from `xy` in the shader precisely
/// because the format never carried it. Matching the hardware matters more here than producing a
/// "nicer" third channel would: the software path exists to stand in for the hardware one, and a
/// fallback that returned a reconstructed `z` would render differently from the machine it is standing
/// in for.
#[must_use]
pub fn decode_bc5(blocks: &[u8], width: u32, height: u32) -> Vec<u8> {
    decode_blocks(blocks, width, height, 16, decode_bc5_block)
}

/// Decodes BC7 blocks to straight-alpha RGBA8, row-major from the top-left.
///
/// A block reserving the low byte entirely — the one encoding the specification declares invalid —
/// decodes to transparent black rather than to a guess.
#[must_use]
pub fn decode_bc7(blocks: &[u8], width: u32, height: u32) -> Vec<u8> {
    decode_blocks(blocks, width, height, 16, decode_bc7_block)
}

/// Builds the one block that encodes a single flat colour, in any of the layouts here.
///
/// # Why an encoder lives in a decoding module
///
/// This is not a compressor. Compression is choosing endpoints and indices to approximate sixteen
/// different texels, which is the offline tool's job and a large one. A *flat* block needs no choosing:
/// both endpoints are the colour and every index selects the first of them, so there is exactly one
/// answer and it is a dozen lines.
///
/// The runtime needs it because a block-compressed array still has to fill the slices a slot does not
/// sample — a model's normal array has a layer for every image the model carries, including the ones that
/// are base colours, and those layers have to hold *something* in the array's format. That something is
/// the slot's identity value: opaque white for a colour, the encoded flat normal for a normal map. See
/// `cic-render`'s `upload_arrays`.
///
/// BC1 quantizes to 5:6:5, so a colour that is not representable there comes back approximated — which is
/// correct for an identity fill (white, black and mid grey are all exact) and is why the identity values
/// this is called with are chosen from that set.
#[must_use]
pub fn solid_block(format: crate::texture::BlockFormat, colour: [u8; 4]) -> Vec<u8> {
    use crate::texture::BlockFormat;
    match format {
        BlockFormat::Bc1RgbaUnorm | BlockFormat::Bc1RgbaUnormSrgb => {
            // Both endpoints equal, so `color0 > color1` is false and the block is in the three-colour
            // interpretation -- where index 0 is still the first endpoint exactly, and its alpha is
            // opaque. Every index is zero.
            let packed = pack_565(colour);
            let mut block = Vec::with_capacity(8);
            block.extend_from_slice(&packed.to_le_bytes());
            block.extend_from_slice(&packed.to_le_bytes());
            block.extend_from_slice(&[0u8; 4]);
            block
        }
        BlockFormat::Bc5Unorm => {
            // Two BC4 halves. Equal endpoints make the whole ramp that value, so the indices do not
            // matter -- but they are zeroed anyway, which selects the first endpoint by name.
            let mut block = Vec::with_capacity(16);
            for channel in [colour[0], colour[1]] {
                block.extend_from_slice(&[channel, channel]);
                block.extend_from_slice(&[0u8; 6]);
            }
            block
        }
        BlockFormat::Bc7Unorm | BlockFormat::Bc7UnormSrgb => {
            // Mode 6: one subset, eight bits per channel per endpoint once the parity bit is counted, and
            // four-bit indices. The top seven bits of each byte are the stored endpoint and the last one
            // is that endpoint's parity bit, so index 0 reproduces the colour exactly.
            let mut value = 1u128 << 6;
            let mut position = 7u32;
            let mut put = |bits: u128, count: u32| {
                value |= bits << position;
                position += count;
            };
            for channel in colour {
                let stored = u128::from(channel >> 1);
                put(stored, 7);
                put(stored, 7);
            }
            let parity = u128::from(colour[0] & 1);
            put(parity, 1);
            put(parity, 1);
            // Indices are all zero, including the anchor's three-bit one.
            value.to_le_bytes().to_vec()
        }
    }
}

/// Packs an RGBA colour into the 5:6:5 word BC1 stores, rounding each channel to its own width.
fn pack_565(colour: [u8; 4]) -> u16 {
    let quantize = |value: u8, bits: u32| -> u16 {
        let maximum = (1u32 << bits) - 1;
        // Rounded rather than truncated, so 255 reaches the top of the range and mid grey lands in the
        // middle. This is a quantization, not the interpolation the specifications pin to truncation.
        let scaled = (u32::from(value) * maximum + 127) / 255;
        #[allow(clippy::cast_possible_truncation)]
        {
            scaled.min(maximum) as u16
        }
    };
    (quantize(colour[0], 5) << 11) | (quantize(colour[1], 6) << 5) | quantize(colour[2], 5)
}

/// Walks the block grid, decoding each and writing the texels it covers that lie inside the image.
///
/// A level whose dimensions are not a multiple of four still stores whole blocks, so the last block of a
/// row or column is partly outside the image. Those texels are decoded and discarded, which is what
/// every hardware decoder does with them.
///
/// Total by construction: a block the payload does not reach decodes as an all-zero one rather than
/// panicking, so a caller that has not checked its lengths gets a black region and not a crash. The
/// lengths *are* checked, in [`crate::texture::TextureAsset::new`].
fn decode_blocks(
    blocks: &[u8],
    width: u32,
    height: u32,
    block_bytes: usize,
    decode: impl Fn(&[u8]) -> [[u8; 4]; 16],
) -> Vec<u8> {
    let (Ok(width_usize), Ok(height_usize)) = (usize::try_from(width), usize::try_from(height))
    else {
        return Vec::new();
    };
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let extent = usize::try_from(BLOCK_EXTENT).unwrap_or(4);
    let blocks_across = width_usize.div_ceil(extent);
    let blocks_down = height_usize.div_ceil(extent);

    let mut rgba = vec![0u8; width_usize * height_usize * 4];
    for block_y in 0..blocks_down {
        for block_x in 0..blocks_across {
            let offset = (block_y * blocks_across + block_x) * block_bytes;
            let texels = decode(blocks.get(offset..offset + block_bytes).unwrap_or(&[]));
            for row in 0..extent {
                let y = block_y * extent + row;
                if y >= height_usize {
                    break;
                }
                for column in 0..extent {
                    let x = block_x * extent + column;
                    if x >= width_usize {
                        break;
                    }
                    let at = (y * width_usize + x) * 4;
                    rgba[at..at + 4].copy_from_slice(&texels[row * extent + column]);
                }
            }
        }
    }
    rgba
}

/// Decodes one 8-byte BC1 block.
fn decode_bc1_block(block: &[u8]) -> [[u8; 4]; 16] {
    let low = u16::from(byte(block, 0)) | (u16::from(byte(block, 1)) << 8);
    let high = u16::from(byte(block, 2)) | (u16::from(byte(block, 3)) << 8);
    let indices = u32::from_le_bytes([
        byte(block, 4),
        byte(block, 5),
        byte(block, 6),
        byte(block, 7),
    ]);

    let first = expand_565(low);
    let second = expand_565(high);
    let palette = if low > high {
        // Four opaque colours: the endpoints and two points a third of the way between them.
        [
            first,
            second,
            blend(first, second, 2, 1, 3),
            blend(first, second, 1, 2, 3),
        ]
    } else {
        // Three opaque colours and a transparent one.
        [first, second, blend(first, second, 1, 1, 2), [0, 0, 0, 0]]
    };

    let mut texels = [[0u8; 4]; 16];
    for (texel, slot) in texels.iter_mut().enumerate() {
        // Two bits per texel, x-major from the top-left, which puts texel zero in the low bits.
        let index = (indices >> (texel * 2)) & 0b11;
        *slot = palette[index as usize];
    }
    texels
}

/// Decodes one 16-byte BC5 block: a BC4 red half followed by a BC4 green half.
fn decode_bc5_block(block: &[u8]) -> [[u8; 4]; 16] {
    let red = decode_bc4_half(block.get(..8).unwrap_or(&[]));
    let green = decode_bc4_half(block.get(8..16).unwrap_or(&[]));
    let mut texels = [[0u8; 4]; 16];
    for (texel, slot) in texels.iter_mut().enumerate() {
        *slot = [red[texel], green[texel], 0, u8::MAX];
    }
    texels
}

/// Decodes one 8-byte BC4 block to sixteen single-channel values.
///
/// Two 8-bit endpoints and three bits per texel. As in BC1 the endpoint comparison switches
/// interpretations, but here it trades two of the eight ramp steps for exact zero and exact one — which
/// is what lets a mask reach both extremes without spending an endpoint on either.
fn decode_bc4_half(block: &[u8]) -> [u8; 16] {
    let first = byte(block, 0);
    let second = byte(block, 1);
    let mut palette = [0u8; 8];
    palette[0] = first;
    palette[1] = second;
    if first > second {
        for (step, slot) in palette[2..8].iter_mut().enumerate() {
            let step = u32::try_from(step).unwrap_or(0) + 1;
            *slot = mix(first, second, 7 - step, step, 7);
        }
    } else {
        for (step, slot) in palette[2..6].iter_mut().enumerate() {
            let step = u32::try_from(step).unwrap_or(0) + 1;
            *slot = mix(first, second, 5 - step, step, 5);
        }
        palette[6] = 0;
        palette[7] = u8::MAX;
    }

    // Six bytes of three-bit indices, x-major from the top-left. Read as one 48-bit word because a
    // three-bit field straddles byte boundaries.
    let mut packed = 0u64;
    for position in 0..6 {
        packed |= u64::from(byte(block, 2 + position)) << (position * 8);
    }
    let mut values = [0u8; 16];
    for (texel, slot) in values.iter_mut().enumerate() {
        let index = (packed >> (texel * 3)) & 0b111;
        *slot = palette[index as usize];
    }
    values
}

/// Decodes one 16-byte BC7 block.
///
/// The structure the specification describes, in the order it describes it: a unary mode, then the
/// partition, rotation and index-selection selectors, then the endpoints channel by channel, then the
/// parity bits that extend them, then the indices — whose *width* depends on where the anchors fall.
#[allow(clippy::too_many_lines)]
fn decode_bc7_block(block: &[u8]) -> [[u8; 4]; 16] {
    let mut payload = [0u8; 16];
    for (slot, source) in payload.iter_mut().zip(block) {
        *slot = *source;
    }
    let mut bits = Bits {
        value: u128::from_le_bytes(payload),
        position: 0,
    };

    // The mode is the count of zeros before the first set bit, within the low byte. An all-zero low byte
    // is the one encoding the specification reserves.
    let mut mode_number = None;
    for candidate in 0..8u32 {
        if bits.read(1) == 1 {
            mode_number = Some(candidate);
            break;
        }
    }
    let Some(mode) = mode_number.and_then(|number| MODES.get(number as usize)) else {
        return [[0u8; 4]; 16];
    };

    let partition = bits.read(mode.partition_bits) as usize;
    let rotation = bits.read(mode.rotation_bits);
    // Read unconditionally at zero width, so the field order stays literal rather than conditional.
    let index_selection = bits.read(mode.index_selection_bits);

    let endpoint_count = (mode.subsets * 2) as usize;
    // Channel-major: every subset's red pair, then every subset's green pair, and so on. Alpha follows
    // the three colour channels in the same organisation, or is opaque when the mode carries none.
    let mut endpoints = [[0u32; 4]; 6];
    for channel in 0..3 {
        for endpoint in endpoints.iter_mut().take(endpoint_count) {
            endpoint[channel] = bits.read(mode.colour_bits);
        }
    }
    if mode.alpha_bits > 0 {
        for endpoint in endpoints.iter_mut().take(endpoint_count) {
            endpoint[3] = bits.read(mode.alpha_bits);
        }
    }

    // A parity bit is appended *below* the stored value, widening every channel of the endpoints it
    // applies to by one bit. Per-endpoint parity gives each endpoint its own; shared parity gives one
    // to both endpoints of a subset, which is how mode 1 affords seven-bit precision in six bits.
    let mut parity = [0u32; 6];
    let has_parity = mode.endpoint_p_bits > 0 || mode.shared_p_bits > 0;
    if mode.endpoint_p_bits > 0 {
        for slot in parity.iter_mut().take(endpoint_count) {
            *slot = bits.read(1);
        }
    } else if mode.shared_p_bits > 0 {
        for subset in 0..mode.subsets as usize {
            let shared = bits.read(1);
            parity[subset * 2] = shared;
            parity[subset * 2 + 1] = shared;
        }
    }

    let colour_width = mode.colour_bits + u32::from(has_parity);
    let alpha_width = mode.alpha_bits + u32::from(has_parity);
    let mut resolved = [[0u8; 4]; 6];
    for (index, endpoint) in endpoints.iter().enumerate().take(endpoint_count) {
        let widen = |value: u32, width: u32| {
            expand(
                if has_parity {
                    (value << 1) | parity[index]
                } else {
                    value
                },
                width,
            )
        };
        for channel in 0..3 {
            resolved[index][channel] = widen(endpoint[channel], colour_width);
        }
        resolved[index][3] = if mode.alpha_bits == 0 {
            // "If not, alpha is overridden to 1.0", says the specification -- fully opaque, and not
            // whatever an all-ones alpha would have expanded to. Widening a stand-in through the parity
            // path instead comes out at 247 in a four-bit mode whose parity bit happens to be zero, which
            // makes every opaque surface very slightly translucent: invisible on an opaque pass and
            // wrong the moment one of them is blended.
            u8::MAX
        } else {
            widen(endpoint[3], alpha_width)
        };
    }

    // Which texel is the anchor of each subset decides which indices are stored a bit short, so this has
    // to be known before a single index is read.
    let anchors = anchors(mode.subsets, partition);
    let mut primary = [0u32; 16];
    for (texel, slot) in primary.iter_mut().enumerate() {
        let width = mode.index_bits - u32::from(anchors.contains(&Some(texel)));
        *slot = bits.read(width);
    }
    let mut secondary = [0u32; 16];
    if mode.index_bits_2 > 0 {
        for (texel, slot) in secondary.iter_mut().enumerate() {
            let width = mode.index_bits_2 - u32::from(anchors.contains(&Some(texel)));
            *slot = bits.read(width);
        }
    }

    let mut texels = [[0u8; 4]; 16];
    for (texel, slot) in texels.iter_mut().enumerate() {
        let subset = subset_of(mode.subsets, partition, texel);
        let first = resolved[subset * 2];
        let second = resolved[subset * 2 + 1];

        // The index selection bit swaps which set drives colour and which drives alpha. Without a
        // secondary set both come from the primary one.
        let (colour_index, colour_bits, alpha_index, alpha_bits) = if mode.index_bits_2 == 0 {
            (
                primary[texel],
                mode.index_bits,
                primary[texel],
                mode.index_bits,
            )
        } else if mode.index_selection_bits > 0 && index_selection == 1 {
            (
                secondary[texel],
                mode.index_bits_2,
                primary[texel],
                mode.index_bits,
            )
        } else {
            (
                primary[texel],
                mode.index_bits,
                secondary[texel],
                mode.index_bits_2,
            )
        };

        let colour_weight = weight(colour_bits, colour_index);
        let alpha_weight = weight(alpha_bits, alpha_index);
        let mut texel_value = [
            interpolate(first[0], second[0], colour_weight),
            interpolate(first[1], second[1], colour_weight),
            interpolate(first[2], second[2], colour_weight),
            interpolate(first[3], second[3], alpha_weight),
        ];
        // A rotation moves the alpha channel into one of the colour slots, which is how a mode with a
        // single endpoint pair can still spend its precision on whichever channel needs it.
        match rotation {
            1 => texel_value.swap(0, 3),
            2 => texel_value.swap(1, 3),
            3 => texel_value.swap(2, 3),
            _ => {}
        }
        *slot = texel_value;
    }
    texels
}

/// Which subset a texel belongs to, for the mode's subset count and partition number.
fn subset_of(subsets: u32, partition: usize, texel: usize) -> usize {
    let table = match subsets {
        2 => PARTITION_2.get(partition),
        3 => PARTITION_3.get(partition),
        _ => return 0,
    };
    table
        .and_then(|row| row.get(texel))
        .map_or(0, |subset| usize::from(*subset))
}

/// The anchor texel of each subset: subset zero's is always texel zero, the others come from the tables.
fn anchors(subsets: u32, partition: usize) -> [Option<usize>; 3] {
    let lookup = |table: &[u8; 64]| table.get(partition).map(|texel| usize::from(*texel));
    match subsets {
        2 => [Some(0), lookup(&ANCHOR_2), None],
        3 => [Some(0), lookup(&ANCHOR_3_SECOND), lookup(&ANCHOR_3_THIRD)],
        _ => [Some(0), None, None],
    }
}

/// The six-bit interpolation factor for an index of the given width.
fn weight(bits: u32, index: u32) -> u32 {
    let table: &[u32] = match bits {
        2 => &WEIGHTS_2,
        3 => &WEIGHTS_3,
        4 => &WEIGHTS_4,
        // No other width exists in any mode; zero is the identity, which keeps this total.
        _ => return 0,
    };
    table.get(index as usize).copied().unwrap_or(0)
}

/// Interpolates two endpoints by a six-bit factor, as every BC format defines it.
fn interpolate(first: u8, second: u8, weight: u32) -> u8 {
    let value = (u32::from(first) * (64 - weight) + u32::from(second) * weight + 32) >> 6;
    // The weights run 0..=64, so the result is a convex combination of two bytes and cannot leave the
    // range.
    #[allow(clippy::cast_possible_truncation)]
    {
        value as u8
    }
}

/// Widens a value of `bits` significant bits to a full byte by replicating its high bits downward.
///
/// Not a shift alone: a five-bit maximum must expand to 255 rather than to 248, or every white texel in
/// a BC1 texture would come out slightly grey.
fn expand(value: u32, bits: u32) -> u8 {
    if bits == 0 || bits > 8 {
        return 0;
    }
    let shifted = (value << (8 - bits)) & 0xFF;
    #[allow(clippy::cast_possible_truncation)]
    {
        (shifted | (shifted >> bits)) as u8
    }
}

/// Expands a 5:6:5 colour to opaque RGBA8.
///
/// By bit replication, which is what Direct3D specifies and what every desktop texture unit does. The
/// GL S3TC extension words this as unpacking an `UNSIGNED_SHORT_5_6_5` pixel, and GL's conversion rule is
/// the exact rational `v * 255 / 31` rounded — which differs from replication by one least-significant
/// bit at some values (a five-bit 3 is 24 replicated and 25 rounded). Replication is the right of the two
/// here for the same reason the BC5 blue channel is left at zero: this decoder exists to stand in for the
/// hardware, so it must agree with the hardware rather than with the looser of two specifications.
fn expand_565(colour: u16) -> [u8; 4] {
    [
        expand(u32::from(colour >> 11) & 0x1F, 5),
        expand(u32::from(colour >> 5) & 0x3F, 6),
        expand(u32::from(colour) & 0x1F, 5),
        u8::MAX,
    ]
}

/// A weighted blend of two opaque colours, `(first * a + second * b) / total`, rounded.
fn blend(first: [u8; 4], second: [u8; 4], a: u32, b: u32, total: u32) -> [u8; 4] {
    [
        mix(first[0], second[0], a, b, total),
        mix(first[1], second[1], a, b, total),
        mix(first[2], second[2], a, b, total),
        u8::MAX,
    ]
}

/// A weighted mean of two bytes, `(first * a + second * b) / total`, **truncating**.
///
/// Truncating rather than rounded, because that is what both specifications write. `EXT_texture_-`
/// `compression_s3tc` gives BC1's interior colours as `(2*RGB0+RGB1)/3` and `ARB_texture_compression_-`
/// `rgtc` gives BC4's ramp as `(6*RED0+RED1)/7`, both as integer division on the already-expanded 8-bit
/// values. Adding a rounding term is the obvious "improvement" and it is wrong: it shifts every interior
/// index by up to one least-significant bit away from what the hardware this stands in for produces,
/// which is a difference small enough to survive review and large enough to fail a reference capture.
fn mix(first: u8, second: u8, a: u32, b: u32, total: u32) -> u8 {
    let value = (u32::from(first) * a + u32::from(second) * b) / total;
    // A weighted mean of two bytes with weights summing to `total` stays inside `0..=255`.
    #[allow(clippy::cast_possible_truncation)]
    {
        value as u8
    }
}

/// One byte of a block, or zero past its end, so decoding a truncated block cannot panic.
fn byte(block: &[u8], index: usize) -> u8 {
    block.get(index).copied().unwrap_or(0)
}

/// A little-endian bit reader over one 128-bit block.
struct Bits {
    value: u128,
    position: u32,
}

impl Bits {
    /// Reads the next `count` bits, low bit first. Returns zero for a zero-width or exhausted read,
    /// which is what keeps a decoder that mis-tracks its budget total rather than panicking.
    fn read(&mut self, count: u32) -> u32 {
        if count == 0 || self.position >= 128 {
            return 0;
        }
        let count = count.min(128 - self.position);
        let mask = (1u128 << count) - 1;
        let value = (self.value >> self.position) & mask;
        self.position += count;
        // No field in any mode is wider than eight bits.
        #[allow(clippy::cast_possible_truncation)]
        {
            value as u32
        }
    }
}

/// One row of the specification's mode table.
struct Mode {
    /// Subsets the block is partitioned into.
    subsets: u32,
    /// Bits holding the partition number.
    partition_bits: u32,
    /// Bits holding the channel rotation.
    rotation_bits: u32,
    /// Bits holding the index-set selector.
    index_selection_bits: u32,
    /// Bits per colour channel per endpoint, before any parity bit.
    colour_bits: u32,
    /// Bits per alpha endpoint, or zero when the mode is opaque.
    alpha_bits: u32,
    /// Parity bits, one per endpoint.
    endpoint_p_bits: u32,
    /// Parity bits, one per subset and shared by both its endpoints.
    shared_p_bits: u32,
    /// Bits per primary index.
    index_bits: u32,
    /// Bits per secondary index, or zero when the mode has one index set.
    index_bits_2: u32,
}

/// The specification's Table.M, in mode order.
const MODES: [Mode; 8] = [
    Mode {
        subsets: 3,
        partition_bits: 4,
        rotation_bits: 0,
        index_selection_bits: 0,
        colour_bits: 4,
        alpha_bits: 0,
        endpoint_p_bits: 1,
        shared_p_bits: 0,
        index_bits: 3,
        index_bits_2: 0,
    },
    Mode {
        subsets: 2,
        partition_bits: 6,
        rotation_bits: 0,
        index_selection_bits: 0,
        colour_bits: 6,
        alpha_bits: 0,
        endpoint_p_bits: 0,
        shared_p_bits: 1,
        index_bits: 3,
        index_bits_2: 0,
    },
    Mode {
        subsets: 3,
        partition_bits: 6,
        rotation_bits: 0,
        index_selection_bits: 0,
        colour_bits: 5,
        alpha_bits: 0,
        endpoint_p_bits: 0,
        shared_p_bits: 0,
        index_bits: 2,
        index_bits_2: 0,
    },
    Mode {
        subsets: 2,
        partition_bits: 6,
        rotation_bits: 0,
        index_selection_bits: 0,
        colour_bits: 7,
        alpha_bits: 0,
        endpoint_p_bits: 1,
        shared_p_bits: 0,
        index_bits: 2,
        index_bits_2: 0,
    },
    Mode {
        subsets: 1,
        partition_bits: 0,
        rotation_bits: 2,
        index_selection_bits: 1,
        colour_bits: 5,
        alpha_bits: 6,
        endpoint_p_bits: 0,
        shared_p_bits: 0,
        index_bits: 2,
        index_bits_2: 3,
    },
    Mode {
        subsets: 1,
        partition_bits: 0,
        rotation_bits: 2,
        index_selection_bits: 0,
        colour_bits: 7,
        alpha_bits: 8,
        endpoint_p_bits: 0,
        shared_p_bits: 0,
        index_bits: 2,
        index_bits_2: 2,
    },
    Mode {
        subsets: 1,
        partition_bits: 0,
        rotation_bits: 0,
        index_selection_bits: 0,
        colour_bits: 7,
        alpha_bits: 7,
        endpoint_p_bits: 1,
        shared_p_bits: 0,
        index_bits: 4,
        index_bits_2: 0,
    },
    Mode {
        subsets: 2,
        partition_bits: 6,
        rotation_bits: 0,
        index_selection_bits: 0,
        colour_bits: 5,
        alpha_bits: 5,
        endpoint_p_bits: 1,
        shared_p_bits: 0,
        index_bits: 2,
        index_bits_2: 0,
    },
];

/// Interpolation factors for a two-bit index.
const WEIGHTS_2: [u32; 4] = [0, 21, 43, 64];
/// Interpolation factors for a three-bit index.
const WEIGHTS_3: [u32; 8] = [0, 9, 18, 27, 37, 46, 55, 64];
/// Interpolation factors for a four-bit index.
const WEIGHTS_4: [u32; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];

/// The specification's Table.P2: which of two subsets each texel belongs to, per partition
/// number. Machine-generated from the BPTC specification text; see the module note on why
/// these are checked for internal consistency rather than read.
const PARTITION_2: [[u8; 16]; 64] = [
    [0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1],
    [0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1],
    [0, 1, 1, 1, 0, 1, 1, 1, 0, 1, 1, 1, 0, 1, 1, 1],
    [0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 1, 1, 1],
    [0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 1, 1],
    [0, 0, 1, 1, 0, 1, 1, 1, 0, 1, 1, 1, 1, 1, 1, 1],
    [0, 0, 0, 1, 0, 0, 1, 1, 0, 1, 1, 1, 1, 1, 1, 1],
    [0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 1, 1, 0, 1, 1, 1],
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 1, 1],
    [0, 0, 1, 1, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
    [0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 1, 1, 1, 1, 1, 1],
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 1, 1],
    [0, 0, 0, 1, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
    [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1],
    [0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1],
    [0, 0, 0, 0, 1, 0, 0, 0, 1, 1, 1, 0, 1, 1, 1, 1],
    [0, 1, 1, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 1, 1, 0],
    [0, 1, 1, 1, 0, 0, 1, 1, 0, 0, 0, 1, 0, 0, 0, 0],
    [0, 0, 1, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 1, 0, 0, 0, 1, 1, 0, 0, 1, 1, 1, 0],
    [0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 1, 0, 0],
    [0, 1, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 0, 1],
    [0, 0, 1, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0],
    [0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 1, 0, 0],
    [0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0],
    [0, 0, 1, 1, 0, 1, 1, 0, 0, 1, 1, 0, 1, 1, 0, 0],
    [0, 0, 0, 1, 0, 1, 1, 1, 1, 1, 1, 0, 1, 0, 0, 0],
    [0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0],
    [0, 1, 1, 1, 0, 0, 0, 1, 1, 0, 0, 0, 1, 1, 1, 0],
    [0, 0, 1, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 1, 0, 0],
    [0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1],
    [0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0, 1, 1, 1, 1],
    [0, 1, 0, 1, 1, 0, 1, 0, 0, 1, 0, 1, 1, 0, 1, 0],
    [0, 0, 1, 1, 0, 0, 1, 1, 1, 1, 0, 0, 1, 1, 0, 0],
    [0, 0, 1, 1, 1, 1, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0],
    [0, 1, 0, 1, 0, 1, 0, 1, 1, 0, 1, 0, 1, 0, 1, 0],
    [0, 1, 1, 0, 1, 0, 0, 1, 0, 1, 1, 0, 1, 0, 0, 1],
    [0, 1, 0, 1, 1, 0, 1, 0, 1, 0, 1, 0, 0, 1, 0, 1],
    [0, 1, 1, 1, 0, 0, 1, 1, 1, 1, 0, 0, 1, 1, 1, 0],
    [0, 0, 0, 1, 0, 0, 1, 1, 1, 1, 0, 0, 1, 0, 0, 0],
    [0, 0, 1, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 1, 0, 0],
    [0, 0, 1, 1, 1, 0, 1, 1, 1, 1, 0, 1, 1, 1, 0, 0],
    [0, 1, 1, 0, 1, 0, 0, 1, 1, 0, 0, 1, 0, 1, 1, 0],
    [0, 0, 1, 1, 1, 1, 0, 0, 1, 1, 0, 0, 0, 0, 1, 1],
    [0, 1, 1, 0, 0, 1, 1, 0, 1, 0, 0, 1, 1, 0, 0, 1],
    [0, 0, 0, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 0, 0, 0],
    [0, 1, 0, 0, 1, 1, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0],
    [0, 0, 1, 0, 0, 1, 1, 1, 0, 0, 1, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 0, 0, 1, 0, 0, 1, 1, 1, 0, 0, 1, 0],
    [0, 0, 0, 0, 0, 1, 0, 0, 1, 1, 1, 0, 0, 1, 0, 0],
    [0, 1, 1, 0, 1, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 1],
    [0, 0, 1, 1, 0, 1, 1, 0, 1, 1, 0, 0, 1, 0, 0, 1],
    [0, 1, 1, 0, 0, 0, 1, 1, 1, 0, 0, 1, 1, 1, 0, 0],
    [0, 0, 1, 1, 1, 0, 0, 1, 1, 1, 0, 0, 0, 1, 1, 0],
    [0, 1, 1, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 0, 0, 1],
    [0, 1, 1, 0, 0, 0, 1, 1, 0, 0, 1, 1, 1, 0, 0, 1],
    [0, 1, 1, 1, 1, 1, 1, 0, 1, 0, 0, 0, 0, 0, 0, 1],
    [0, 0, 0, 1, 1, 0, 0, 0, 1, 1, 1, 0, 0, 1, 1, 1],
    [0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1],
    [0, 0, 1, 1, 0, 0, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0],
    [0, 0, 1, 0, 0, 0, 1, 0, 1, 1, 1, 0, 1, 1, 1, 0],
    [0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 1, 1, 0, 1, 1, 1],
];

/// The specification's Table.P3, for three-subset partitions.
const PARTITION_3: [[u8; 16]; 64] = [
    [0, 0, 1, 1, 0, 0, 1, 1, 0, 2, 2, 1, 2, 2, 2, 2],
    [0, 0, 0, 1, 0, 0, 1, 1, 2, 2, 1, 1, 2, 2, 2, 1],
    [0, 0, 0, 0, 2, 0, 0, 1, 2, 2, 1, 1, 2, 2, 1, 1],
    [0, 2, 2, 2, 0, 0, 2, 2, 0, 0, 1, 1, 0, 1, 1, 1],
    [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 2, 2, 1, 1, 2, 2],
    [0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 2, 2, 0, 0, 2, 2],
    [0, 0, 2, 2, 0, 0, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1],
    [0, 0, 1, 1, 0, 0, 1, 1, 2, 2, 1, 1, 2, 2, 1, 1],
    [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2],
    [0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2],
    [0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2],
    [0, 0, 1, 2, 0, 0, 1, 2, 0, 0, 1, 2, 0, 0, 1, 2],
    [0, 1, 1, 2, 0, 1, 1, 2, 0, 1, 1, 2, 0, 1, 1, 2],
    [0, 1, 2, 2, 0, 1, 2, 2, 0, 1, 2, 2, 0, 1, 2, 2],
    [0, 0, 1, 1, 0, 1, 1, 2, 1, 1, 2, 2, 1, 2, 2, 2],
    [0, 0, 1, 1, 2, 0, 0, 1, 2, 2, 0, 0, 2, 2, 2, 0],
    [0, 0, 0, 1, 0, 0, 1, 1, 0, 1, 1, 2, 1, 1, 2, 2],
    [0, 1, 1, 1, 0, 0, 1, 1, 2, 0, 0, 1, 2, 2, 0, 0],
    [0, 0, 0, 0, 1, 1, 2, 2, 1, 1, 2, 2, 1, 1, 2, 2],
    [0, 0, 2, 2, 0, 0, 2, 2, 0, 0, 2, 2, 1, 1, 1, 1],
    [0, 1, 1, 1, 0, 1, 1, 1, 0, 2, 2, 2, 0, 2, 2, 2],
    [0, 0, 0, 1, 0, 0, 0, 1, 2, 2, 2, 1, 2, 2, 2, 1],
    [0, 0, 0, 0, 0, 0, 1, 1, 0, 1, 2, 2, 0, 1, 2, 2],
    [0, 0, 0, 0, 1, 1, 0, 0, 2, 2, 1, 0, 2, 2, 1, 0],
    [0, 1, 2, 2, 0, 1, 2, 2, 0, 0, 1, 1, 0, 0, 0, 0],
    [0, 0, 1, 2, 0, 0, 1, 2, 1, 1, 2, 2, 2, 2, 2, 2],
    [0, 1, 1, 0, 1, 2, 2, 1, 1, 2, 2, 1, 0, 1, 1, 0],
    [0, 0, 0, 0, 0, 1, 1, 0, 1, 2, 2, 1, 1, 2, 2, 1],
    [0, 0, 2, 2, 1, 1, 0, 2, 1, 1, 0, 2, 0, 0, 2, 2],
    [0, 1, 1, 0, 0, 1, 1, 0, 2, 0, 0, 2, 2, 2, 2, 2],
    [0, 0, 1, 1, 0, 1, 2, 2, 0, 1, 2, 2, 0, 0, 1, 1],
    [0, 0, 0, 0, 2, 0, 0, 0, 2, 2, 1, 1, 2, 2, 2, 1],
    [0, 0, 0, 0, 0, 0, 0, 2, 1, 1, 2, 2, 1, 2, 2, 2],
    [0, 2, 2, 2, 0, 0, 2, 2, 0, 0, 1, 2, 0, 0, 1, 1],
    [0, 0, 1, 1, 0, 0, 1, 2, 0, 0, 2, 2, 0, 2, 2, 2],
    [0, 1, 2, 0, 0, 1, 2, 0, 0, 1, 2, 0, 0, 1, 2, 0],
    [0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 0, 0, 0, 0],
    [0, 1, 2, 0, 1, 2, 0, 1, 2, 0, 1, 2, 0, 1, 2, 0],
    [0, 1, 2, 0, 2, 0, 1, 2, 1, 2, 0, 1, 0, 1, 2, 0],
    [0, 0, 1, 1, 2, 2, 0, 0, 1, 1, 2, 2, 0, 0, 1, 1],
    [0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 0, 0, 0, 0, 1, 1],
    [0, 1, 0, 1, 0, 1, 0, 1, 2, 2, 2, 2, 2, 2, 2, 2],
    [0, 0, 0, 0, 0, 0, 0, 0, 2, 1, 2, 1, 2, 1, 2, 1],
    [0, 0, 2, 2, 1, 1, 2, 2, 0, 0, 2, 2, 1, 1, 2, 2],
    [0, 0, 2, 2, 0, 0, 1, 1, 0, 0, 2, 2, 0, 0, 1, 1],
    [0, 2, 2, 0, 1, 2, 2, 1, 0, 2, 2, 0, 1, 2, 2, 1],
    [0, 1, 0, 1, 2, 2, 2, 2, 2, 2, 2, 2, 0, 1, 0, 1],
    [0, 0, 0, 0, 2, 1, 2, 1, 2, 1, 2, 1, 2, 1, 2, 1],
    [0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 2, 2, 2, 2],
    [0, 2, 2, 2, 0, 1, 1, 1, 0, 2, 2, 2, 0, 1, 1, 1],
    [0, 0, 0, 2, 1, 1, 1, 2, 0, 0, 0, 2, 1, 1, 1, 2],
    [0, 0, 0, 0, 2, 1, 1, 2, 2, 1, 1, 2, 2, 1, 1, 2],
    [0, 2, 2, 2, 0, 1, 1, 1, 0, 1, 1, 1, 0, 2, 2, 2],
    [0, 0, 0, 2, 1, 1, 1, 2, 1, 1, 1, 2, 0, 0, 0, 2],
    [0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 2, 2, 2, 2],
    [0, 0, 0, 0, 0, 0, 0, 0, 2, 1, 1, 2, 2, 1, 1, 2],
    [0, 1, 1, 0, 0, 1, 1, 0, 2, 2, 2, 2, 2, 2, 2, 2],
    [0, 0, 2, 2, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 2, 2],
    [0, 0, 2, 2, 1, 1, 2, 2, 1, 1, 2, 2, 0, 0, 2, 2],
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 1, 1, 2],
    [0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 1],
    [0, 2, 2, 2, 1, 2, 2, 2, 0, 2, 2, 2, 1, 2, 2, 2],
    [0, 1, 0, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2],
    [0, 1, 1, 1, 2, 0, 1, 1, 2, 2, 0, 1, 2, 2, 2, 0],
];

/// The specification's Table.A2: the anchor texel of the second subset, per two-subset partition.
const ANCHOR_2: [u8; 64] = [
    15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 2, 8, 2, 2, 8, 8, 15, 2, 8,
    2, 2, 8, 8, 2, 2, 15, 15, 6, 8, 2, 8, 15, 15, 2, 8, 2, 2, 2, 15, 15, 6, 6, 2, 6, 8, 15, 15, 2,
    2, 15, 15, 15, 15, 15, 2, 2, 15,
];

/// The specification's Table.A3a: the anchor texel of the second subset, per three-subset partition.
const ANCHOR_3_SECOND: [u8; 64] = [
    3, 3, 15, 15, 8, 3, 15, 15, 8, 8, 6, 6, 6, 5, 3, 3, 3, 3, 8, 15, 3, 3, 6, 10, 5, 8, 8, 6, 8, 5,
    15, 15, 8, 15, 3, 5, 6, 10, 8, 15, 15, 3, 15, 5, 15, 15, 15, 15, 3, 15, 5, 5, 5, 8, 5, 10, 5,
    10, 8, 13, 15, 12, 3, 3,
];

/// The specification's Table.A3b: the anchor texel of the third subset, per three-subset partition.
const ANCHOR_3_THIRD: [u8; 64] = [
    15, 8, 8, 3, 15, 15, 3, 8, 15, 15, 15, 15, 15, 15, 15, 8, 15, 8, 15, 3, 15, 8, 15, 8, 3, 15, 6,
    10, 15, 15, 10, 8, 15, 3, 15, 10, 10, 8, 9, 10, 6, 15, 8, 15, 3, 6, 6, 8, 15, 3, 15, 15, 15,
    15, 15, 15, 15, 15, 15, 15, 3, 15, 15, 8,
];

#[cfg(test)]
mod tests {
    use super::{
        ANCHOR_2, ANCHOR_3_SECOND, ANCHOR_3_THIRD, MODES, PARTITION_2, PARTITION_3, decode_bc1,
        decode_bc5, decode_bc7, expand,
    };

    /// A BC1 block of two 5:6:5 endpoints and sixteen two-bit indices.
    fn bc1_block(first: u16, second: u16, indices: [u8; 16]) -> Vec<u8> {
        let mut block = Vec::with_capacity(8);
        block.extend_from_slice(&first.to_le_bytes());
        block.extend_from_slice(&second.to_le_bytes());
        let mut packed = 0u32;
        for (texel, index) in indices.iter().enumerate() {
            packed |= u32::from(*index) << (texel * 2);
        }
        block.extend_from_slice(&packed.to_le_bytes());
        block
    }

    /// A BC4 half of two endpoints and sixteen three-bit indices.
    fn bc4_half(first: u8, second: u8, indices: [u8; 16]) -> Vec<u8> {
        let mut packed = 0u64;
        for (texel, index) in indices.iter().enumerate() {
            packed |= u64::from(*index) << (texel * 3);
        }
        let mut block = vec![first, second];
        block.extend_from_slice(&packed.to_le_bytes()[..6]);
        block
    }

    /// A BC7 mode-6 block: two RGBA endpoints of seven bits and a parity bit each, then four-bit
    /// indices. The single mode this project's own encoder emits for an opaque or blended surface, so
    /// it is worth being able to build one by hand.
    fn bc7_mode6(first: [u8; 4], second: [u8; 4], indices: [u8; 16]) -> Vec<u8> {
        let mut value = 1u128 << 6;
        let mut position = 7;
        let put = |bits: u128, count: u32, value: &mut u128, position: &mut u32| {
            *value |= (bits & ((1u128 << count) - 1)) << *position;
            *position += count;
        };
        // Endpoints are channel-major: both ends of red, then green, then blue, then alpha. The top
        // seven bits of each byte are stored and the eighth becomes that endpoint's parity bit.
        for channel in 0..4 {
            put(
                u128::from(first[channel] >> 1),
                7,
                &mut value,
                &mut position,
            );
            put(
                u128::from(second[channel] >> 1),
                7,
                &mut value,
                &mut position,
            );
        }
        put(u128::from(first[0] & 1), 1, &mut value, &mut position);
        put(u128::from(second[0] & 1), 1, &mut value, &mut position);
        // Texel zero is the anchor of the only subset, so its index is stored a bit short.
        put(u128::from(indices[0]), 3, &mut value, &mut position);
        for index in &indices[1..] {
            put(u128::from(*index), 4, &mut value, &mut position);
        }
        value.to_le_bytes().to_vec()
    }

    #[test]
    fn the_partition_and_anchor_tables_agree_with_each_other() {
        // The check that catches a transcribed digit. A two-subset partition that used only one subset,
        // or an anchor pointing at a texel of the wrong subset, would decode plausible-looking garbage
        // -- the failure mode this project calls a wrong answer presented confidently.
        for (partition, row) in PARTITION_2.iter().enumerate() {
            assert!(
                row.contains(&0) && row.contains(&1),
                "two-subset partition {partition} does not use both subsets: {row:?}"
            );
            assert!(
                row.iter().all(|subset| *subset < 2),
                "two-subset partition {partition} names a third subset: {row:?}"
            );
            let anchor = usize::from(ANCHOR_2[partition]);
            assert_eq!(
                row[anchor], 1,
                "two-subset partition {partition} anchors subset 1 at texel {anchor}, which is in \
                 subset {}",
                row[anchor]
            );
        }
        for (partition, row) in PARTITION_3.iter().enumerate() {
            for subset in 0..3u8 {
                assert!(
                    row.contains(&subset),
                    "three-subset partition {partition} never uses subset {subset}: {row:?}"
                );
            }
            let second = usize::from(ANCHOR_3_SECOND[partition]);
            let third = usize::from(ANCHOR_3_THIRD[partition]);
            assert_eq!(row[second], 1, "partition {partition} anchor for subset 1");
            assert_eq!(row[third], 2, "partition {partition} anchor for subset 2");
            assert_ne!(second, third, "partition {partition} reuses one anchor");
        }
    }

    #[test]
    fn every_mode_consumes_exactly_one_hundred_and_twenty_eight_bits() {
        // The strongest single statement about the mode table: BC7 packs a block with nothing left over,
        // so a wrong figure in any column shows up here rather than as a texture that is subtly wrong on
        // some blocks. The mode number contributes its own unary bits.
        for (number, mode) in MODES.iter().enumerate() {
            let endpoints = mode.subsets * 2;
            let anchors = mode.subsets;
            let total = u32::try_from(number).expect("eight modes")
                + 1
                + mode.partition_bits
                + mode.rotation_bits
                + mode.index_selection_bits
                + endpoints * mode.colour_bits * 3
                + endpoints * mode.alpha_bits
                + endpoints * mode.endpoint_p_bits
                + mode.subsets * mode.shared_p_bits
                + (16 * mode.index_bits - anchors)
                + if mode.index_bits_2 > 0 {
                    16 * mode.index_bits_2 - anchors
                } else {
                    0
                };
            assert_eq!(total, 128, "mode {number} packs {total} bits");
        }
    }

    #[test]
    fn a_five_bit_maximum_expands_to_full_white_rather_than_to_248() {
        // The reason expansion replicates the high bits instead of only shifting. A shift alone would
        // make every white texel in a BC1 texture slightly grey, which reads as a washed-out texture
        // rather than as an arithmetic mistake.
        assert_eq!(expand(0b1_1111, 5), 255);
        assert_eq!(expand(0b11_1111, 6), 255);
        assert_eq!(expand(0, 5), 0);
        assert_eq!(expand(0b1_0000, 5), 132);
        assert_eq!(expand(255, 8), 255);
    }

    #[test]
    fn bc1_interpolates_four_opaque_colours_when_the_endpoints_are_ordered() {
        // Endpoint order is the switch between BC1's two interpretations, and it is compared on the
        // *stored* words. Red (0xF800) above black gives four opaque colours.
        let block = bc1_block(
            0xF800,
            0x0000,
            [0, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3],
        );
        let rgba = decode_bc1(&block, 4, 4);
        assert_eq!(
            &rgba[0..4],
            [255, 0, 0, 255],
            "index 0 is the first endpoint"
        );
        assert_eq!(&rgba[4..8], [0, 0, 0, 255], "index 1 is the second");
        assert_eq!(
            &rgba[8..12],
            [170, 0, 0, 255],
            "index 2 is two thirds of the way"
        );
        assert_eq!(&rgba[12..16], [85, 0, 0, 255], "index 3 is one third");
    }

    #[test]
    fn bc1_punches_through_alpha_when_the_endpoints_are_reversed() {
        // The same two colours the other way round: three opaque colours and a transparent fourth. This
        // is what a 4 bpp cutout mask relies on, and reading the comparison backwards would make every
        // masked texture fully opaque.
        let block = bc1_block(
            0x0000,
            0xF800,
            [0, 1, 2, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        );
        let rgba = decode_bc1(&block, 4, 4);
        assert_eq!(&rgba[0..4], [0, 0, 0, 255]);
        assert_eq!(&rgba[4..8], [255, 0, 0, 255]);
        assert_eq!(
            &rgba[8..12],
            [127, 0, 0, 255],
            "the specification's (RGB0+RGB1)/2 truncates: 255/2 is 127, not 128"
        );
        assert_eq!(&rgba[12..16], [0, 0, 0, 0], "index 3 is transparent");
    }

    #[test]
    fn the_interior_ramps_truncate_exactly_as_the_specifications_write_them() {
        // Pinned against the published arithmetic, digit for digit, because the tempting change here is
        // to add a rounding term -- which moves every interior index by up to one least-significant bit
        // away from the hardware this decoder stands in for.
        //
        // `EXT_texture_compression_s3tc`: (2*RGB0+RGB1)/3 and (RGB0+2*RGB1)/3, integer division.
        // Green 0x07E0 is 255 and endpoint 1 is 0, so the two interior colours are 170 and 85.
        let ordered = bc1_block(
            0x07E0,
            0x0000,
            [2, 3, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        );
        let rgba = decode_bc1(&ordered, 4, 1);
        assert_eq!(rgba[1], 170, "(2*255 + 0)/3 is two thirds of the way");
        assert_eq!(rgba[5], 85, "(255 + 2*0)/3 is one third");

        // `ARB_texture_compression_rgtc`, the six-step branch: (4*RED0+RED1)/5 down to (RED0+4*RED1)/5,
        // then exact zero and exact one. The formula is written once, here, so each expectation below is
        // the specification's weights rather than a number somebody worked out.
        let ramp = |first: u32, second: u32, weights: (u32, u32), total: u32| -> u8 {
            u8::try_from((first * weights.0 + second * weights.1) / total)
                .expect("a weighted mean of two bytes is a byte")
        };

        // Endpoints 10 and 200 with 10 <= 200 selects the six-step branch. 4x2, so both rows of indices
        // sit inside the one block supplied: an 8x1 request would span two.
        let mut block = bc4_half(10, 200, [0, 1, 2, 3, 4, 5, 6, 7, 0, 0, 0, 0, 0, 0, 0, 0]);
        block.extend(bc4_half(0, 0, [0; 16]));
        let ramp_values = decode_bc5(&block, 4, 2);
        let red: Vec<u8> = ramp_values.chunks_exact(4).map(|texel| texel[0]).collect();
        assert_eq!(
            red[..8],
            [
                10,
                200,
                ramp(10, 200, (4, 1), 5),
                ramp(10, 200, (3, 2), 5),
                ramp(10, 200, (2, 3), 5),
                ramp(10, 200, (1, 4), 5),
                0,
                255,
            ],
            "got {red:?}"
        );

        // And the eight-step branch, which has no exact zero or one: 200 > 10 selects it.
        let mut block = bc4_half(200, 10, [0, 1, 2, 3, 4, 5, 6, 7, 0, 0, 0, 0, 0, 0, 0, 0]);
        block.extend(bc4_half(0, 0, [0; 16]));
        let ramp_values = decode_bc5(&block, 4, 2);
        let red: Vec<u8> = ramp_values.chunks_exact(4).map(|texel| texel[0]).collect();
        assert_eq!(
            red[..8],
            [
                200,
                10,
                ramp(200, 10, (6, 1), 7),
                ramp(200, 10, (5, 2), 7),
                ramp(200, 10, (4, 3), 7),
                ramp(200, 10, (3, 4), 7),
                ramp(200, 10, (2, 5), 7),
                ramp(200, 10, (1, 6), 7),
            ],
            "got {red:?}"
        );
    }

    #[test]
    fn an_opaque_bc7_mode_returns_alpha_exactly_opaque() {
        // The specification says alpha is *overridden* to 1.0 when a mode carries no alpha bits. Deriving
        // it instead -- widening an all-ones stand-in through the parity path -- yields 247 for mode 0,
        // whose parity bit may be zero. That is invisible on an opaque pass and wrong the moment such a
        // material is blended or alpha-tested, so it is pinned for every opaque mode at once.
        //
        // Mode `n` is the bit pattern with a single set bit at position `n`; modes 0 to 3 are the opaque
        // ones. The remaining payload is arbitrary here: whatever the endpoints and indices say, alpha
        // must not depend on them.
        for mode in 0..4u32 {
            let mut block = [0x5Au8; 16];
            block[0] = 1u8 << mode;
            for texel in decode_bc7(&block, 4, 4).chunks_exact(4) {
                assert_eq!(
                    texel[3], 255,
                    "mode {mode} carries no alpha bits and must decode fully opaque"
                );
            }
        }
    }

    #[test]
    fn bc5_decodes_two_independent_channels_and_leaves_blue_at_zero() {
        // The property that makes BC5 right for a normal map: the two channels share no endpoints, so a
        // red ramp and a constant green do not contaminate each other. Blue is zero because the format
        // has no third channel -- the shader rebuilds `z` from `xy`.
        let mut block = bc4_half(0, 255, [0, 1, 2, 3, 4, 5, 6, 7, 0, 0, 0, 0, 0, 0, 0, 0]);
        block.extend(bc4_half(64, 64, [0; 16]));
        let rgba = decode_bc5(&block, 4, 4);
        // First endpoint below the second is BC4's six-step ramp with exact zero and one at the top.
        assert_eq!(rgba[0], 0, "index 0 is the first endpoint");
        assert_eq!(rgba[4], 255, "index 1 is the second");
        assert_eq!(rgba[8], 51, "index 2 is one fifth of the way");
        assert_eq!(rgba[24], 0, "index 6 is exact zero");
        assert_eq!(rgba[28], 255, "index 7 is exact one");
        for texel in rgba.chunks_exact(4) {
            assert_eq!(texel[1], 64, "green is constant and unaffected by red");
            assert_eq!(texel[2], 0, "blue is not carried by BC5");
            assert_eq!(texel[3], 255, "alpha is opaque");
        }
    }

    #[test]
    fn bc7_mode_six_reproduces_its_endpoints_exactly() {
        // Mode 6 spends all 128 bits on one endpoint pair at eight bits per channel, so index 0 and the
        // last index must come back byte-exact. Anything else means the parity bit, the anchor's short
        // index, or the channel order is wrong -- and all three would be invisible on a flat texture.
        let first = [10, 20, 30, 40];
        let second = [200, 210, 220, 230];
        let mut indices = [0u8; 16];
        indices[1] = 15;
        indices[2] = 8;
        let block = bc7_mode6(first, second, indices);
        let rgba = decode_bc7(&block, 4, 4);
        assert_eq!(&rgba[0..4], first, "index 0 is the first endpoint exactly");
        assert_eq!(
            &rgba[4..8],
            second,
            "index 15 is the second endpoint exactly"
        );
        // Index 8 sits at weight 34 of 64, a little past the midpoint.
        assert_eq!(&rgba[8..12], [111, 121, 131, 141]);
    }

    #[test]
    fn a_reserved_bc7_block_decodes_to_transparent_black_rather_than_to_a_guess() {
        // The specification reserves an all-zero low byte. Returning something plausible would hide an
        // encoder writing empty blocks.
        assert_eq!(decode_bc7(&[0u8; 16], 4, 4), vec![0u8; 4 * 4 * 4]);
    }

    #[test]
    fn a_level_narrower_than_a_block_keeps_only_the_texels_inside_it() {
        // Every level below 4x4 in a mip chain is this case, so it is the ordinary path rather than an
        // edge one. A decoder that wrote all sixteen texels would run off the end of the buffer.
        let block = bc1_block(0xF800, 0x0000, [0; 16]);
        let rgba = decode_bc1(&block, 1, 1);
        assert_eq!(rgba, vec![255, 0, 0, 255]);
        let two_by_one = decode_bc1(&block, 2, 1);
        assert_eq!(two_by_one.len(), 8);
    }

    #[test]
    fn a_truncated_payload_decodes_black_rather_than_panicking() {
        // The decoders are total: the length invariant is enforced in `crate::texture`, and this is what
        // happens if it ever is not.
        assert_eq!(decode_bc7(&[], 4, 4), vec![0u8; 64]);
        assert_eq!(decode_bc1(&[1, 2, 3], 4, 4).len(), 64);
        assert!(decode_bc5(&[0u8; 16], 0, 4).is_empty());
    }
}
