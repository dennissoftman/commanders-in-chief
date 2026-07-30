//! Block-compressed textures in a DDS container.
//!
//! # Why block compression at all
//!
//! A 2048-pixel base-colour texture is 16 MiB as RGBA8, and a model carries three of them. The same
//! image as BC7 is 4 MiB *including its whole mip chain*, and — this is the part that matters at
//! runtime — it stays compressed in video memory and is decompressed by fixed-function hardware inside
//! the texture unit. So the saving is not only disk and upload bandwidth: every sample touches a
//! quarter as much cache.
//!
//! Loading is faster for a second reason that has nothing to do with size.
//! [ADR 0004](../../../docs/adr/0004-texture-arrays-and-world-space-tiling.md) generates mip chains on
//! the CPU at upload, which is a full-image pass per level per slice, and it noted that if that cost
//! ever bit, the answer was precomputed mips in the asset. A `.dds` *is* that: the chain is already
//! there, and the upload becomes a copy.
//!
//! # Which formats, and what goes in them
//!
//! | Slot | Format | Why |
//! |---|---|---|
//! | Base colour | **BC7** sRGB | 8 bpp with a per-block choice of eight endpoint/partition modes. The only BC format that carries an alpha channel without visibly banding it, which foliage cutouts need. |
//! | Normal | **BC5** | Two independent 8-bit channels at 8 bpp, each compressed like a greyscale image with no shared endpoints. `z` is rebuilt in the shader from `xy`, so the third channel was never needed. |
//! | Occlusion / roughness / metallic | **BC7** linear | Three unrelated measurements in R, G and B — exactly glTF's own packing, so nothing in the shader changes. |
//! | Anything flat or masked | **BC1** | 4 bpp. One RGB endpoint pair per block with three-bit-per-texel interpolation, so it couples the channels; fine for colour, wrong for packed data. |
//! | Terrain detail layer | **BC7** sRGB, or **BC1** | A tiled colour, so the base-colour reasoning applies — and this is where it pays most, being sampled by up to eight layers in one fragment across the whole visible map. |
//!
//! **BC5 cannot hold ORM.** It is two BC4 blocks, red and green, and there is no third channel — so a
//! packed occlusion/roughness/metallic map goes in BC7 (same 8 bpp, all three channels, and the
//! per-channel error of a data map is what BC7's mode choice is good at). BC1 would fit three channels
//! at half the size and is the wrong trade here: its single RGB endpoint pair means a roughness gradient
//! drags the metallic channel along with it.
//!
//! # Why DDS rather than KTX2
//!
//! KTX2 is the better-specified container and the one glTF's own extension uses. DDS wins here on one
//! practical point: every texture tool a content author already has — `texconv`, Compressonator,
//! NVIDIA's tools, Photoshop's plugin — writes it, and the header is 128 bytes of fixed-offset fields
//! that need no dependency to parse. KTX2 additionally allows a supercompressed payload (Zstd, Basis),
//! which would mean a decompressor in the runtime before the texture unit ever sees a block; that is a
//! real feature and not one this engine needs, because the package is already a zip.
//!
//! Like every other decoder in this crate, this one is bounded and total: explicit limits, a structured
//! error naming what it found and what it expected, and no panic on hostile input.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

use cic_vfs::{ResourceReadError, Vfs, VirtualPath};

use crate::bc;
use crate::image::{ColourSpace, halve};

/// Texels along each axis of one compressed block, in every format here.
pub const BLOCK_EXTENT: u32 = 4;

/// A block-compressed pixel layout.
///
/// Deliberately not every format DDS can carry. Each of these is here because a slot needs it — see the
/// module table — and a format nothing writes is a decoder nobody tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockFormat {
    /// BC1 with the one-bit alpha the format implies, treated as linear data.
    Bc1RgbaUnorm,
    /// BC1 whose RGB is sRGB-encoded.
    Bc1RgbaUnormSrgb,
    /// BC5: two independent unsigned channels, red and green.
    Bc5Unorm,
    /// BC7 read as linear data. What a packed ORM map uses.
    Bc7Unorm,
    /// BC7 whose RGB is sRGB-encoded. What a base colour uses.
    Bc7UnormSrgb,
}

impl BlockFormat {
    /// Bytes one 4x4 block occupies.
    #[must_use]
    pub const fn block_bytes(self) -> u32 {
        match self {
            // One RGB endpoint pair and two bits per texel.
            Self::Bc1RgbaUnorm | Self::Bc1RgbaUnormSrgb => 8,
            Self::Bc5Unorm | Self::Bc7Unorm | Self::Bc7UnormSrgb => 16,
        }
    }

    /// The space this format's stored values are in, which decides how a mip chain built from them had
    /// to be averaged.
    ///
    /// See [`crate::image::ColourSpace`] for why getting this wrong is invisible at the base level.
    #[must_use]
    pub const fn colour_space(self) -> ColourSpace {
        match self {
            Self::Bc1RgbaUnormSrgb | Self::Bc7UnormSrgb => ColourSpace::Srgb,
            Self::Bc1RgbaUnorm | Self::Bc5Unorm | Self::Bc7Unorm => ColourSpace::Linear,
        }
    }

    /// The same format in the other colour space, for the two that have both.
    ///
    /// `None` for [`Self::Bc5Unorm`], which has no sRGB variant because two independent channels are
    /// never a colour.
    #[must_use]
    pub const fn with_colour_space(self, space: ColourSpace) -> Option<Self> {
        match (self, space) {
            (Self::Bc1RgbaUnorm | Self::Bc1RgbaUnormSrgb, ColourSpace::Srgb) => {
                Some(Self::Bc1RgbaUnormSrgb)
            }
            (Self::Bc1RgbaUnorm | Self::Bc1RgbaUnormSrgb, ColourSpace::Linear) => {
                Some(Self::Bc1RgbaUnorm)
            }
            (Self::Bc7Unorm | Self::Bc7UnormSrgb, ColourSpace::Srgb) => Some(Self::Bc7UnormSrgb),
            (Self::Bc7Unorm | Self::Bc7UnormSrgb, ColourSpace::Linear) => Some(Self::Bc7Unorm),
            (Self::Bc5Unorm, _) => None,
        }
    }

    /// The `DXGI_FORMAT` value this layout is written as in a DX10 header.
    #[must_use]
    pub const fn dxgi_format(self) -> u32 {
        match self {
            Self::Bc1RgbaUnorm => DXGI_BC1_UNORM,
            Self::Bc1RgbaUnormSrgb => DXGI_BC1_UNORM_SRGB,
            Self::Bc5Unorm => DXGI_BC5_UNORM,
            Self::Bc7Unorm => DXGI_BC7_UNORM,
            Self::Bc7UnormSrgb => DXGI_BC7_UNORM_SRGB,
        }
    }

    /// The name this format is known by in tooling, for an error message a content author can act on.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Bc1RgbaUnorm => "BC1_UNORM",
            Self::Bc1RgbaUnormSrgb => "BC1_UNORM_SRGB",
            Self::Bc5Unorm => "BC5_UNORM",
            Self::Bc7Unorm => "BC7_UNORM",
            Self::Bc7UnormSrgb => "BC7_UNORM_SRGB",
        }
    }
}

/// Explicit bounds applied while reading an untrusted texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextureLimits {
    /// Maximum texels along either axis of the base level.
    pub maximum_dimension: u32,
    /// Maximum mip levels retained.
    pub maximum_levels: u32,
    /// Maximum compressed bytes summed across every level.
    pub maximum_bytes: usize,
}

impl Default for TextureLimits {
    fn default() -> Self {
        Self {
            // The baseline device limit for a 2D texture, matching `ModelLimits`. Anything larger
            // cannot be uploaded, so accepting it would only move the failure somewhere less
            // informative.
            maximum_dimension: 8_192,
            // A full chain from 8192 reaches 1x1 in fourteen levels, so this bounds nothing a
            // well-formed file does and refuses a header claiming hundreds.
            maximum_levels: 14,
            maximum_bytes: 512 * 1_024 * 1_024,
        }
    }
}

/// One block-compressed image and its mip chain.
///
/// A single 2D image, not an array or a cube: the renderer composes array *slices* from several of
/// these, so a container that carried its own layers would be a second way to express the same thing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureAsset {
    width: u32,
    height: u32,
    format: BlockFormat,
    levels: Vec<Vec<u8>>,
}

impl TextureAsset {
    /// Wraps compressed levels, checking each against the size its position implies.
    ///
    /// `levels` runs base level first. A chain may stop early — a file with only level 0 is legal DDS —
    /// and the renderer will create a texture with exactly the levels present. That aliases at distance,
    /// which is what the mip chain exists to prevent, so the converter always writes a full chain; this
    /// accepts a short one because refusing it would reject files other tools legitimately produce.
    ///
    /// # Errors
    ///
    /// Returns [`TextureError::NoLevels`] for an empty chain, [`TextureError::EmptyDimension`] for a
    /// zero axis, [`TextureError::LevelSizeMismatch`] when a level's byte length disagrees with its
    /// derived size, or [`TextureError::LimitExceeded`] against `limits`.
    pub fn new(
        width: u32,
        height: u32,
        format: BlockFormat,
        levels: Vec<Vec<u8>>,
        limits: TextureLimits,
    ) -> Result<Self, TextureError> {
        if width == 0 || height == 0 {
            return Err(TextureError::EmptyDimension { width, height });
        }
        if width > limits.maximum_dimension || height > limits.maximum_dimension {
            return Err(TextureError::LimitExceeded {
                what: "texture dimension",
                actual: as_usize(width.max(height)),
                maximum: as_usize(limits.maximum_dimension),
            });
        }
        if levels.is_empty() {
            return Err(TextureError::NoLevels);
        }
        let level_count = u32::try_from(levels.len()).unwrap_or(u32::MAX);
        if level_count > limits.maximum_levels {
            return Err(TextureError::LimitExceeded {
                what: "mip level count",
                actual: levels.len(),
                maximum: as_usize(limits.maximum_levels),
            });
        }

        let mut total = 0usize;
        for (index, payload) in levels.iter().enumerate() {
            let level = u32::try_from(index).unwrap_or(u32::MAX);
            let (level_width, level_height) = level_size(width, height, level);
            let expected = level_bytes(level_width, level_height, format)?;
            if payload.len() != expected {
                return Err(TextureError::LevelSizeMismatch {
                    level,
                    expected,
                    actual: payload.len(),
                });
            }
            total = total
                .checked_add(expected)
                .ok_or(TextureError::LimitExceeded {
                    what: "texture bytes",
                    actual: usize::MAX,
                    maximum: limits.maximum_bytes,
                })?;
            if total > limits.maximum_bytes {
                return Err(TextureError::LimitExceeded {
                    what: "texture bytes",
                    actual: total,
                    maximum: limits.maximum_bytes,
                });
            }
        }

        Ok(Self {
            width,
            height,
            format,
            levels,
        })
    }

    /// A texture of one flat colour, with a full mip chain, in a given block layout.
    ///
    /// What a block-compressed array uses for a slice its slot never samples: the array's layers are its
    /// model's images, and a normal-map array still needs a layer where the model carried a base colour.
    /// Every level is the same block repeated, so this costs a `memcpy` rather than a compression pass —
    /// see [`crate::bc::solid_block`] for why a flat block has exactly one encoding.
    ///
    /// # Errors
    ///
    /// As [`Self::new`].
    pub fn solid(
        width: u32,
        height: u32,
        format: BlockFormat,
        colour: [u8; 4],
        limits: TextureLimits,
    ) -> Result<Self, TextureError> {
        let block = bc::solid_block(format, colour);
        let levels = (0..crate::image::mip_level_count(width.max(1), height.max(1)))
            .map(|level| {
                let (level_width, level_height) = level_size(width.max(1), height.max(1), level);
                let blocks = as_usize(level_width.div_ceil(BLOCK_EXTENT).max(1))
                    * as_usize(level_height.div_ceil(BLOCK_EXTENT).max(1));
                block.repeat(blocks)
            })
            .collect();
        Self::new(width, height, format, levels, limits)
    }

    /// Returns the base level's width in texels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the base level's height in texels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Returns the compressed layout every level is in.
    #[must_use]
    pub const fn format(&self) -> BlockFormat {
        self.format
    }

    /// Returns how many mip levels are present, base level included.
    #[must_use]
    pub fn level_count(&self) -> u32 {
        u32::try_from(self.levels.len()).unwrap_or(u32::MAX)
    }

    /// Returns one level's compressed blocks, row-major from the top-left in 4x4 block order.
    #[must_use]
    pub fn level(&self, level: u32) -> Option<&[u8]> {
        self.levels
            .get(usize::try_from(level).ok()?)
            .map(Vec::as_slice)
    }

    /// Returns one level's texel dimensions, whether or not that level is present.
    ///
    /// Each axis halves and stops at one, which is how both DDS and every graphics API define a chain —
    /// so a 5x3 base has a 2x1 level 1 rather than a 3x2 one.
    #[must_use]
    pub const fn level_size(&self, level: u32) -> (u32, u32) {
        level_size(self.width, self.height, level)
    }

    /// Returns the total compressed bytes across every level.
    #[must_use]
    pub fn byte_count(&self) -> usize {
        self.levels.iter().map(Vec::len).sum()
    }

    /// Decodes one level to straight-alpha RGBA8, row-major from the top-left.
    ///
    /// The software path: what an adapter without hardware block compression uploads, and what makes
    /// this format testable with no device at all. See [`crate::bc`].
    #[must_use]
    pub fn decode_level(&self, level: u32) -> Option<Vec<u8>> {
        let blocks = self.level(level)?;
        let (width, height) = self.level_size(level);
        Some(match self.format {
            BlockFormat::Bc1RgbaUnorm | BlockFormat::Bc1RgbaUnormSrgb => {
                bc::decode_bc1(blocks, width, height)
            }
            BlockFormat::Bc5Unorm => bc::decode_bc5(blocks, width, height),
            BlockFormat::Bc7Unorm | BlockFormat::Bc7UnormSrgb => {
                bc::decode_bc7(blocks, width, height)
            }
        })
    }

    /// Decodes the base level to straight-alpha RGBA8.
    #[must_use]
    pub fn decode(&self) -> Vec<u8> {
        self.decode_level(0).unwrap_or_default()
    }

    /// Writes this texture as a DDS container with a DX10 header.
    ///
    /// Always the DX10 header, never the legacy four-character code, for two reasons: BC7 has no legacy
    /// code at all, and the legacy codes cannot express the sRGB/linear distinction — which is the one
    /// property of a texture that nothing downstream can recover by looking at the pixels. [`decode_dds`]
    /// still reads the legacy codes, because other tools write them.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out =
            Vec::with_capacity(MAGIC_BYTES + HEADER_BYTES + DX10_BYTES + self.byte_count());
        out.extend_from_slice(&DDS_MAGIC.to_le_bytes());

        let mut header = [0u32; HEADER_BYTES / 4];
        header[0] = 124;
        header[1] = DDSD_CAPS
            | DDSD_HEIGHT
            | DDSD_WIDTH
            | DDSD_PIXELFORMAT
            | DDSD_MIPMAPCOUNT
            | DDSD_LINEARSIZE;
        header[2] = self.height;
        header[3] = self.width;
        // For a block-compressed surface this field is the base level's total byte count, not a pitch.
        header[4] = u32::try_from(self.levels.first().map_or(0, Vec::len)).unwrap_or(u32::MAX);
        header[5] = 0;
        header[6] = self.level_count();
        // 7..=17 is `dwReserved1`, left zero.
        header[18] = 32; // ddspf.dwSize
        header[19] = DDPF_FOURCC;
        header[20] = FOURCC_DX10;
        // 21..=25 are the bit count and channel masks, meaningless for a four-character code.
        header[26] = DDSCAPS_TEXTURE
            | if self.level_count() > 1 {
                DDSCAPS_COMPLEX | DDSCAPS_MIPMAP
            } else {
                0
            };
        // 27..=30 are caps2..caps4 and reserved2, all zero: not a cube map, not a volume.
        for field in header {
            out.extend_from_slice(&field.to_le_bytes());
        }

        for field in [
            self.format.dxgi_format(),
            RESOURCE_DIMENSION_TEXTURE2D,
            0, // miscFlag: not a cube map
            1, // arraySize
            0, // miscFlags2: DDS_ALPHA_MODE_UNKNOWN, and straight alpha is the assumption throughout
        ] {
            out.extend_from_slice(&field.to_le_bytes());
        }

        for level in &self.levels {
            out.extend_from_slice(level);
        }
        out
    }
}

/// Directory block-compressed textures are looked up in, relative to the mount root.
///
/// One directory for the whole package rather than one beside each asset, because the *name* is already
/// the sharing mechanism: two models that reference an image of the same name, or a model and a terrain
/// layer that do, are meant to get the same texture, and a per-asset directory would silently give them
/// several.
pub const TEXTURE_DIRECTORY: &str = "textures";

/// Looks up `textures/<name>.dds` for each of a list of names.
///
/// The one place the sidecar convention lives, shared by
/// [`crate::model::resolve_model_textures`] and [`crate::terrain::resolve_terrain_textures`]. Both ask the
/// same question of different names — a glTF image's name in one case, a terrain layer's in the other —
/// and neither should own the answer.
///
/// An empty name resolves to `None` without a lookup: there is no key, and deriving one from the
/// entry's position would silently give two different assets the same texture.
///
/// # Errors
///
/// An **absent** sidecar is not an error. That is the ordinary state of content that has not been
/// converted, and the caller has working pixels either way.
///
/// A sidecar that **exists and will not read** is an error, and deliberately so: it means a converted
/// texture is being silently rendered from whatever stood in for it, which is exactly the failure a
/// content author needs told about rather than left to notice.
pub fn resolve_named_textures<'a>(
    names: impl IntoIterator<Item = &'a str>,
    vfs: &Vfs,
    limits: TextureLimits,
) -> Result<Vec<Option<TextureAsset>>, TextureResolveError> {
    let mut assets = Vec::new();
    for name in names {
        if name.is_empty() {
            assets.push(None);
            continue;
        }
        let path = format!("{TEXTURE_DIRECTORY}/{name}.dds");
        let virtual_path = VirtualPath::new(&path).map_err(|error| TextureResolveError::Path {
            path: path.clone(),
            error,
        })?;
        let Some(entry) = vfs.resolve(&virtual_path) else {
            assets.push(None);
            continue;
        };
        let bytes =
            entry
                .read(limits.maximum_bytes)
                .map_err(|error| TextureResolveError::Read {
                    path: path.clone(),
                    error,
                })?;
        let texture = decode_dds(&bytes, limits)
            .map_err(|error| TextureResolveError::Texture { path, error })?;
        assets.push(Some(texture));
    }
    Ok(assets)
}

/// A structured failure while resolving a block-compressed sidecar.
#[derive(Debug)]
pub enum TextureResolveError {
    /// A name did not form a safe virtual path.
    Path {
        /// The path that was attempted.
        path: String,
        /// The underlying normalization failure.
        error: cic_vfs::PathError,
    },
    /// A sidecar existed but could not be read.
    Read {
        /// Mount-relative path of the sidecar.
        path: String,
        /// The underlying read failure.
        error: ResourceReadError,
    },
    /// A sidecar was read but is not a texture this engine can upload.
    Texture {
        /// Mount-relative path of the sidecar.
        path: String,
        /// The underlying container or format failure.
        error: TextureError,
    },
}

impl Display for TextureResolveError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Path { path, error } => {
                write!(formatter, "texture path `{path}` is unusable: {error}")
            }
            Self::Read { path, error } => {
                write!(formatter, "texture `{path}` could not be read: {error}")
            }
            Self::Texture { path, error } => {
                write!(formatter, "texture `{path}` is unusable: {error}")
            }
        }
    }
}

impl Error for TextureResolveError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Path { error, .. } => Some(error),
            Self::Read { error, .. } => Some(error),
            Self::Texture { error, .. } => Some(error),
        }
    }
}

/// Reads a DDS container holding a BC1, BC5 or BC7 surface.
///
/// # Errors
///
/// Returns a structured [`TextureError`] when the magic or a header size is wrong, the pixel format is
/// one this engine does not use, the surface is a cube map, a volume or an array, the payload is shorter
/// than the header's own declarations, or a [`TextureLimits`] bound is crossed.
#[allow(clippy::too_many_lines)]
pub fn decode_dds(bytes: &[u8], limits: TextureLimits) -> Result<TextureAsset, TextureError> {
    if bytes.len() < MAGIC_BYTES + HEADER_BYTES {
        return Err(TextureError::TruncatedHeader {
            expected: MAGIC_BYTES + HEADER_BYTES,
            actual: bytes.len(),
        });
    }
    if u32_at(bytes, 0) != DDS_MAGIC {
        return Err(TextureError::NotDds);
    }

    // Both size fields are constants in the specification, and a file disagreeing with them is not a
    // DDS this reader can interpret -- the field offsets below are only meaningful at those sizes.
    let header_size = u32_at(bytes, 4);
    if header_size != 124 {
        return Err(TextureError::UnexpectedHeaderSize {
            what: "DDS_HEADER",
            size: header_size,
        });
    }
    let pixel_format_size = u32_at(bytes, 76);
    if pixel_format_size != 32 {
        return Err(TextureError::UnexpectedHeaderSize {
            what: "DDS_PIXELFORMAT",
            size: pixel_format_size,
        });
    }

    // Offsets into `DDS_HEADER`, which begins after the four-byte magic: height, width, then the
    // pitch-or-linear-size this reader ignores at 20, then depth and the mip count.
    let height = u32_at(bytes, 12);
    let width = u32_at(bytes, 16);
    let depth = u32_at(bytes, 24);
    let declared_levels = u32_at(bytes, 28);
    let pixel_flags = u32_at(bytes, 80);
    let four_cc = u32_at(bytes, 84);
    let caps2 = u32_at(bytes, 112);

    if caps2 & DDSCAPS2_CUBEMAP != 0 {
        return Err(TextureError::Cubemap);
    }
    if caps2 & DDSCAPS2_VOLUME != 0 || depth > 1 {
        return Err(TextureError::Volume { depth });
    }
    if pixel_flags & DDPF_FOURCC == 0 {
        return Err(TextureError::Uncompressed { flags: pixel_flags });
    }

    let (format, payload_offset) = if four_cc == FOURCC_DX10 {
        if bytes.len() < MAGIC_BYTES + HEADER_BYTES + DX10_BYTES {
            return Err(TextureError::TruncatedHeader {
                expected: MAGIC_BYTES + HEADER_BYTES + DX10_BYTES,
                actual: bytes.len(),
            });
        }
        let dxgi = u32_at(bytes, 128);
        let dimension = u32_at(bytes, 132);
        let misc = u32_at(bytes, 136);
        let layers = u32_at(bytes, 140);
        if misc & DDS_RESOURCE_MISC_TEXTURECUBE != 0 {
            return Err(TextureError::Cubemap);
        }
        if dimension == RESOURCE_DIMENSION_TEXTURE3D {
            return Err(TextureError::Volume {
                depth: depth.max(1),
            });
        }
        // Zero appears in files written before the field was consistently set, and means one.
        if layers > 1 {
            return Err(TextureError::ArrayTexture { layers });
        }
        (from_dxgi(dxgi)?, MAGIC_BYTES + HEADER_BYTES + DX10_BYTES)
    } else {
        (from_four_cc(four_cc)?, MAGIC_BYTES + HEADER_BYTES)
    };

    // Zero means one: `DDSD_MIPMAPCOUNT` is frequently unset on a single-level file, and the count is
    // then left at zero rather than at one.
    let level_count = declared_levels.max(1);
    if level_count > limits.maximum_levels {
        return Err(TextureError::LimitExceeded {
            what: "mip level count",
            actual: as_usize(level_count),
            maximum: as_usize(limits.maximum_levels),
        });
    }
    if width == 0 || height == 0 {
        return Err(TextureError::EmptyDimension { width, height });
    }
    if width > limits.maximum_dimension || height > limits.maximum_dimension {
        return Err(TextureError::LimitExceeded {
            what: "texture dimension",
            actual: as_usize(width.max(height)),
            maximum: as_usize(limits.maximum_dimension),
        });
    }

    // Sizes are derived from the dimensions rather than read from `dwPitchOrLinearSize`, which tools
    // disagree about and which a hostile file would simply lie in. Each level is checked against the
    // bytes actually present before it is copied, so a header claiming a gigabyte of level 0 fails here
    // rather than after the allocation.
    let mut levels = Vec::with_capacity(as_usize(level_count));
    let mut offset = payload_offset;
    for level in 0..level_count {
        let (level_width, level_height) = level_size(width, height, level);
        let expected = level_bytes(level_width, level_height, format)?;
        let end = offset
            .checked_add(expected)
            .ok_or(TextureError::LimitExceeded {
                what: "texture bytes",
                actual: usize::MAX,
                maximum: limits.maximum_bytes,
            })?;
        let payload = bytes
            .get(offset..end)
            .ok_or(TextureError::TruncatedPayload {
                level,
                expected,
                actual: bytes.len().saturating_sub(offset),
            })?;
        levels.push(payload.to_vec());
        offset = end;
    }

    TextureAsset::new(width, height, format, levels, limits)
}

/// One level's texel dimensions: each axis halved `level` times, stopping at one.
const fn level_size(width: u32, height: u32, level: u32) -> (u32, u32) {
    let (mut level_width, mut level_height) = (width, height);
    let mut remaining = level;
    while remaining > 0 {
        level_width = halve(level_width);
        level_height = halve(level_height);
        remaining -= 1;
    }
    (level_width, level_height)
}

/// Compressed bytes one level occupies: whole blocks, so a 5x5 level still costs 2x2 of them.
fn level_bytes(width: u32, height: u32, format: BlockFormat) -> Result<usize, TextureError> {
    let blocks_across = width.div_ceil(BLOCK_EXTENT).max(1);
    let blocks_down = height.div_ceil(BLOCK_EXTENT).max(1);
    as_usize(blocks_across)
        .checked_mul(as_usize(blocks_down))
        .and_then(|blocks| blocks.checked_mul(as_usize(format.block_bytes())))
        .ok_or(TextureError::LimitExceeded {
            what: "level bytes",
            actual: usize::MAX,
            maximum: usize::MAX,
        })
}

/// Maps a `DXGI_FORMAT` to a layout this engine reads, naming the ones deliberately left out.
fn from_dxgi(format: u32) -> Result<BlockFormat, TextureError> {
    match format {
        DXGI_BC1_UNORM => Ok(BlockFormat::Bc1RgbaUnorm),
        DXGI_BC1_UNORM_SRGB => Ok(BlockFormat::Bc1RgbaUnormSrgb),
        DXGI_BC5_UNORM => Ok(BlockFormat::Bc5Unorm),
        DXGI_BC7_UNORM => Ok(BlockFormat::Bc7Unorm),
        DXGI_BC7_UNORM_SRGB => Ok(BlockFormat::Bc7UnormSrgb),
        _ => Err(TextureError::UnsupportedDxgiFormat {
            format,
            // The name is for a content author reading a build log, so the ones worth naming are the
            // near misses: a converter set to the wrong preset, not an arbitrary number.
            name: dxgi_name(format),
        }),
    }
}

/// Maps a legacy four-character code to a layout, for files from tools that predate the DX10 header.
///
/// Only the two codes that have an unambiguous meaning. `DXT1` is BC1 and `ATI2`/`BC5U` is BC5; both are
/// read as *linear*, because a legacy header cannot say otherwise — and a base colour, which is the one
/// slot where that would be wrong, is BC7 and therefore always arrives with a DX10 header.
fn from_four_cc(four_cc: u32) -> Result<BlockFormat, TextureError> {
    match four_cc {
        FOURCC_DXT1 => Ok(BlockFormat::Bc1RgbaUnorm),
        FOURCC_ATI2 | FOURCC_BC5U => Ok(BlockFormat::Bc5Unorm),
        _ => Err(TextureError::UnsupportedFourCc {
            code: four_cc_name(four_cc),
        }),
    }
}

/// A `DXGI_FORMAT` number as the name a tool would have shown, for the ones a texture might plausibly
/// arrive as.
const fn dxgi_name(format: u32) -> &'static str {
    match format {
        70 => "BC1_TYPELESS",
        73..=75 => "BC2 (DXT3)",
        76..=78 => "BC3 (DXT5)",
        79..=81 => "BC4",
        82 => "BC5_TYPELESS",
        84 => "BC5_SNORM",
        94..=96 => "BC6H",
        97 => "BC7_TYPELESS",
        _ => "not a block-compressed format",
    }
}

/// A four-character code rendered back as its characters, so an error names what the file said.
fn four_cc_name(code: u32) -> String {
    code.to_le_bytes()
        .iter()
        .map(|byte| {
            if byte.is_ascii_graphic() {
                char::from(*byte)
            } else {
                '?'
            }
        })
        .collect()
}

/// A `u32` as a `usize`, saturating. Every call site is a count or a dimension already bounded well
/// below either type's range; the saturation is there so this is total rather than a cast.
fn as_usize(value: u32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

/// Reads a little-endian `u32`, yielding zero when the offset is out of range.
///
/// Total rather than indexing: every call below is inside a length already checked, and a helper that
/// cannot panic means a future field added at a new offset cannot introduce one either.
fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    let mut field = [0u8; 4];
    if let Some(slice) = bytes.get(offset..offset.saturating_add(4)) {
        field.copy_from_slice(slice);
    }
    u32::from_le_bytes(field)
}

const MAGIC_BYTES: usize = 4;
const HEADER_BYTES: usize = 124;
const DX10_BYTES: usize = 20;

/// `"DDS "` in little-endian byte order.
const DDS_MAGIC: u32 = 0x2053_4444;
/// `"DX10"`, the code that says a `DDS_HEADER_DXT10` follows.
const FOURCC_DX10: u32 = 0x3031_5844;
/// `"DXT1"`.
const FOURCC_DXT1: u32 = 0x3154_5844;
/// `"ATI2"`, what ATI's tools called BC5.
const FOURCC_ATI2: u32 = 0x3249_5441;
/// `"BC5U"`, what everyone else called it.
const FOURCC_BC5U: u32 = 0x5535_4342;

const DDSD_CAPS: u32 = 0x1;
const DDSD_HEIGHT: u32 = 0x2;
const DDSD_WIDTH: u32 = 0x4;
const DDSD_PIXELFORMAT: u32 = 0x1000;
const DDSD_MIPMAPCOUNT: u32 = 0x2_0000;
const DDSD_LINEARSIZE: u32 = 0x8_0000;

const DDPF_FOURCC: u32 = 0x4;

const DDSCAPS_COMPLEX: u32 = 0x8;
const DDSCAPS_TEXTURE: u32 = 0x1000;
const DDSCAPS_MIPMAP: u32 = 0x40_0000;

const DDSCAPS2_CUBEMAP: u32 = 0x200;
const DDSCAPS2_VOLUME: u32 = 0x20_0000;

const RESOURCE_DIMENSION_TEXTURE2D: u32 = 3;
const RESOURCE_DIMENSION_TEXTURE3D: u32 = 4;
const DDS_RESOURCE_MISC_TEXTURECUBE: u32 = 0x4;

const DXGI_BC1_UNORM: u32 = 71;
const DXGI_BC1_UNORM_SRGB: u32 = 72;
const DXGI_BC5_UNORM: u32 = 83;
const DXGI_BC7_UNORM: u32 = 98;
const DXGI_BC7_UNORM_SRGB: u32 = 99;

/// A structured failure while reading a texture container.
#[derive(Debug)]
pub enum TextureError {
    /// The first four bytes were not `DDS `.
    NotDds,
    /// The file ended before a header this reader needs was complete.
    TruncatedHeader {
        /// Bytes the header needs.
        expected: usize,
        /// Bytes the file has.
        actual: usize,
    },
    /// A header declared a size the specification fixes, so its field offsets are unknown.
    UnexpectedHeaderSize {
        /// Which header.
        what: &'static str,
        /// The size it declared.
        size: u32,
    },
    /// The surface is stored as uncompressed channels rather than as blocks.
    Uncompressed {
        /// The pixel-format flags found.
        flags: u32,
    },
    /// A legacy four-character code this reader does not map.
    UnsupportedFourCc {
        /// The code, as its characters.
        code: String,
    },
    /// A `DXGI_FORMAT` this engine does not use.
    UnsupportedDxgiFormat {
        /// The number found.
        format: u32,
        /// What that number is called.
        name: &'static str,
    },
    /// The surface is a cube map, which nothing here samples.
    Cubemap,
    /// The surface is a volume texture.
    Volume {
        /// Declared depth.
        depth: u32,
    },
    /// The surface carries several array layers. The renderer builds its arrays from separate files.
    ArrayTexture {
        /// Layers declared.
        layers: u32,
    },
    /// A mip level's payload was not present in full.
    TruncatedPayload {
        /// Which level.
        level: u32,
        /// Bytes the level needs.
        expected: usize,
        /// Bytes remaining in the file.
        actual: usize,
    },
    /// A level's byte length disagreed with the size its position in the chain implies.
    LevelSizeMismatch {
        /// Which level.
        level: u32,
        /// Bytes the level's dimensions imply.
        expected: usize,
        /// Bytes supplied.
        actual: usize,
    },
    /// A texture declared a zero width or height.
    EmptyDimension {
        /// Declared width.
        width: u32,
        /// Declared height.
        height: u32,
    },
    /// A texture carried no levels at all.
    NoLevels,
    /// An explicit [`TextureLimits`] bound was exceeded.
    LimitExceeded {
        /// Which bound was crossed.
        what: &'static str,
        /// Observed value.
        actual: usize,
        /// Configured maximum.
        maximum: usize,
    },
}

impl Display for TextureError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotDds => formatter.write_str("not a DDS container: the magic is not `DDS `"),
            Self::TruncatedHeader { expected, actual } => write!(
                formatter,
                "a {expected}-byte DDS header needs more than the {actual} bytes present"
            ),
            Self::UnexpectedHeaderSize { what, size } => {
                write!(formatter, "{what} declares an unexpected size {size}")
            }
            Self::Uncompressed { flags } => write!(
                formatter,
                "the surface is uncompressed (pixel format flags {flags:#x}); convert it to BC1, BC5 or BC7 first"
            ),
            Self::UnsupportedFourCc { code } => write!(
                formatter,
                "unsupported legacy pixel format `{code}`; only DXT1 and ATI2/BC5U are read without a DX10 header"
            ),
            Self::UnsupportedDxgiFormat { format, name } => write!(
                formatter,
                "unsupported DXGI format {format} ({name}); this engine reads BC1, BC5 and BC7"
            ),
            Self::Cubemap => formatter.write_str("the surface is a cube map"),
            Self::Volume { depth } => {
                write!(
                    formatter,
                    "the surface is a volume texture of depth {depth}"
                )
            }
            Self::ArrayTexture { layers } => write!(
                formatter,
                "the surface declares {layers} array layers; array slices come from separate files"
            ),
            Self::TruncatedPayload {
                level,
                expected,
                actual,
            } => write!(
                formatter,
                "mip level {level} needs {expected} bytes and {actual} remain"
            ),
            Self::LevelSizeMismatch {
                level,
                expected,
                actual,
            } => write!(
                formatter,
                "mip level {level} is {actual} bytes where its dimensions imply {expected}"
            ),
            Self::EmptyDimension { width, height } => {
                write!(formatter, "a texture cannot be {width}x{height}")
            }
            Self::NoLevels => formatter.write_str("the texture carries no mip levels"),
            Self::LimitExceeded {
                what,
                actual,
                maximum,
            } => write!(formatter, "{what} {actual} exceeds maximum {maximum}"),
        }
    }
}

impl Error for TextureError {}

#[cfg(test)]
mod tests {
    use super::{
        BLOCK_EXTENT, BlockFormat, TextureAsset, TextureError, TextureLimits, decode_dds,
        level_size,
    };
    use crate::image::ColourSpace;

    /// A BC7 mode-6 block encoding opaque white, as `crate::bc` reads it.
    ///
    /// Mode 6 is signalled by the first set bit sitting at position 6, then two RGBA endpoints of seven
    /// bits each with a trailing parity bit, then the indices. All-ones endpoints with all-zero indices
    /// select endpoint 0 everywhere, which is `(255, 255, 255, 255)`.
    fn white_bc7_block() -> Vec<u8> {
        let mut block = vec![0u8; 16];
        // Bit 6 is the mode, bit 7 starts the endpoints.
        block[0] = 0b1100_0000;
        // Bits 8..=63: the rest of the endpoint bits.
        for byte in &mut block[1..8] {
            *byte = u8::MAX;
        }
        // Bit 64 is endpoint 1's parity bit; bit 65 onward are the indices, all zero.
        block[8] = 0b0000_0001;
        block
    }

    /// Overwrites a little-endian `u32` field, so a test can state which header field it corrupts.
    fn patch(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn asset(format: BlockFormat, width: u32, height: u32) -> TextureAsset {
        let levels = (0..crate::image::mip_level_count(width, height))
            .map(|level| level_size(width, height, level))
            .map(|(w, h)| {
                let blocks = w.div_ceil(BLOCK_EXTENT).max(1) * h.div_ceil(BLOCK_EXTENT).max(1);
                vec![0u8; (blocks * format.block_bytes()) as usize]
            })
            .collect();
        TextureAsset::new(width, height, format, levels, TextureLimits::default())
            .expect("a derived chain is self-consistent")
    }

    #[test]
    fn a_written_container_reads_back_identically() {
        // The round trip is the whole claim of the container half of this module: every field the reader
        // needs is one the writer set, at the offset the specification puts it.
        for format in [
            BlockFormat::Bc1RgbaUnorm,
            BlockFormat::Bc1RgbaUnormSrgb,
            BlockFormat::Bc5Unorm,
            BlockFormat::Bc7Unorm,
            BlockFormat::Bc7UnormSrgb,
        ] {
            let original = asset(format, 16, 8);
            let read = decode_dds(&original.encode(), TextureLimits::default())
                .unwrap_or_else(|error| panic!("{}: {error}", format.name()));
            assert_eq!(read, original, "{}", format.name());
        }
    }

    #[test]
    fn the_srgb_distinction_survives_the_container() {
        // The one property of a texture that cannot be recovered by looking at the pixels, and the
        // reason every write uses a DX10 header rather than a four-character code.
        let colour = decode_dds(
            &asset(BlockFormat::Bc7UnormSrgb, 4, 4).encode(),
            TextureLimits::default(),
        )
        .expect("read");
        let data = decode_dds(
            &asset(BlockFormat::Bc7Unorm, 4, 4).encode(),
            TextureLimits::default(),
        )
        .expect("read");
        assert_eq!(colour.format().colour_space(), ColourSpace::Srgb);
        assert_eq!(data.format().colour_space(), ColourSpace::Linear);
        assert_ne!(colour.format(), data.format());
    }

    #[test]
    fn bc5_has_no_srgb_variant_because_two_channels_are_not_a_colour() {
        // Stated as a test because the tempting mistake is to give every format both spaces for
        // symmetry, and an sRGB normal map is exactly the failure `crate::image` documents.
        assert_eq!(
            BlockFormat::Bc5Unorm.with_colour_space(ColourSpace::Srgb),
            None
        );
        assert_eq!(
            BlockFormat::Bc7Unorm.with_colour_space(ColourSpace::Srgb),
            Some(BlockFormat::Bc7UnormSrgb)
        );
        assert_eq!(
            BlockFormat::Bc1RgbaUnormSrgb.with_colour_space(ColourSpace::Linear),
            Some(BlockFormat::Bc1RgbaUnorm)
        );
    }

    #[test]
    fn a_mip_chain_halves_each_axis_independently_and_stops_at_one() {
        // A 5x3 base, because the failure worth catching is a chain that rounds a dimension up: DDS and
        // every graphics API floor it, and a reader that disagreed would read every level after the
        // first at the wrong offset.
        let texture = asset(BlockFormat::Bc7Unorm, 5, 3);
        assert_eq!(texture.level_size(0), (5, 3));
        assert_eq!(texture.level_size(1), (2, 1));
        assert_eq!(texture.level_size(2), (1, 1));
        assert_eq!(texture.level_count(), 3);
        // Every level costs a whole block however small it is, which is why the tail is not free.
        assert_eq!(texture.byte_count(), (2 + 1 + 1) * 16);
    }

    #[test]
    fn reads_the_legacy_four_character_codes_other_tools_write() {
        // A DDS from a tool that predates the DX10 header. Assembled here rather than by `encode`, which
        // deliberately never produces this form -- so this is the one path a round trip cannot cover.
        let written = asset(BlockFormat::Bc5Unorm, 8, 8).encode();
        let mut legacy = written[..128].to_vec();
        patch(&mut legacy, 84, 0x3249_5441); // `ATI2` in place of `DX10`
        legacy.extend_from_slice(&written[148..]); // and no DX10 header between them
        let read = decode_dds(&legacy, TextureLimits::default()).expect("legacy BC5");
        assert_eq!(read.format(), BlockFormat::Bc5Unorm);
        assert_eq!((read.width(), read.height()), (8, 8));
    }

    #[test]
    fn refuses_what_it_cannot_sample_rather_than_reading_it_wrong() {
        let limits = TextureLimits::default();
        assert!(matches!(
            decode_dds(&[b'X'; 256], limits),
            Err(TextureError::NotDds)
        ));
        assert!(matches!(
            decode_dds(b"DDS short", limits),
            Err(TextureError::TruncatedHeader { .. })
        ));

        // A cube map, via the legacy caps bit.
        let mut cube = asset(BlockFormat::Bc7Unorm, 4, 4).encode();
        patch(&mut cube, 112, 0x200);
        assert!(matches!(
            decode_dds(&cube, limits),
            Err(TextureError::Cubemap)
        ));

        // A volume texture, via the DX10 resource dimension.
        let mut volume = asset(BlockFormat::Bc7Unorm, 4, 4).encode();
        patch(&mut volume, 132, 4);
        assert!(matches!(
            decode_dds(&volume, limits),
            Err(TextureError::Volume { .. })
        ));

        // An array texture, which the renderer composes from separate files instead.
        let mut array = asset(BlockFormat::Bc7Unorm, 4, 4).encode();
        patch(&mut array, 140, 6);
        assert!(matches!(
            decode_dds(&array, limits),
            Err(TextureError::ArrayTexture { layers: 6 })
        ));

        // An uncompressed surface: a real DDS, and not one there is any point uploading as it stands.
        let mut raw = asset(BlockFormat::Bc7Unorm, 4, 4).encode();
        patch(&mut raw, 80, 0x41); // DDPF_RGB | DDPF_ALPHAPIXELS, and no four-character code
        assert!(matches!(
            decode_dds(&raw, limits),
            Err(TextureError::Uncompressed { .. })
        ));

        // BC3, which is a real DDS this engine deliberately does not read -- and the error names it, so
        // a content author knows the converter preset was wrong rather than the file broken.
        let mut bc3 = asset(BlockFormat::Bc7Unorm, 4, 4).encode();
        patch(&mut bc3, 128, 77);
        assert!(matches!(
            decode_dds(&bc3, limits),
            Err(TextureError::UnsupportedDxgiFormat { format: 77, name }) if name.contains("BC3")
        ));
    }

    #[test]
    fn refuses_a_payload_the_header_over_declares_before_allocating_it() {
        // The bound that matters for hostile input: the level sizes come from the dimensions rather than
        // from `dwPitchOrLinearSize`, so a file declaring an 8192-square BC7 texture while carrying
        // sixteen bytes must fail on what is present rather than reserve what it asked for.
        let mut bytes = asset(BlockFormat::Bc7Unorm, 4, 4).encode();
        patch(&mut bytes, 12, 8_192); // height
        patch(&mut bytes, 16, 8_192); // width
        assert!(matches!(
            decode_dds(&bytes, TextureLimits::default()),
            Err(TextureError::TruncatedPayload { level: 0, .. })
        ));
    }

    #[test]
    fn refuses_a_texture_past_the_declared_bounds() {
        let bytes = asset(BlockFormat::Bc7Unorm, 64, 64).encode();
        assert!(matches!(
            decode_dds(
                &bytes,
                TextureLimits {
                    maximum_dimension: 32,
                    ..TextureLimits::default()
                }
            ),
            Err(TextureError::LimitExceeded {
                what: "texture dimension",
                ..
            })
        ));
        assert!(matches!(
            decode_dds(
                &bytes,
                TextureLimits {
                    maximum_levels: 3,
                    ..TextureLimits::default()
                }
            ),
            Err(TextureError::LimitExceeded {
                what: "mip level count",
                ..
            })
        ));
        assert!(matches!(
            decode_dds(
                &bytes,
                TextureLimits {
                    maximum_bytes: 64,
                    ..TextureLimits::default()
                }
            ),
            Err(TextureError::LimitExceeded {
                what: "texture bytes",
                ..
            })
        ));
    }

    #[test]
    fn a_level_whose_length_disagrees_with_its_dimensions_is_refused() {
        // The invariant the renderer relies on: it computes a level's row pitch from the dimensions, so
        // a short level would have it reading past the buffer or uploading garbage.
        let error = TextureAsset::new(
            8,
            8,
            BlockFormat::Bc7Unorm,
            vec![vec![0u8; 4 * 16], vec![0u8; 99]],
            TextureLimits::default(),
        )
        .expect_err("level 1 of an 8x8 BC7 is one block");
        assert!(
            matches!(
                error,
                TextureError::LevelSizeMismatch {
                    level: 1,
                    expected: 16,
                    actual: 99
                }
            ),
            "got {error:?}"
        );
    }

    #[test]
    fn a_decoded_level_is_rgba8_at_that_levels_size() {
        // The bridge to the software path: whatever the block layout, a decoded level is four bytes per
        // texel at that level's own dimensions.
        let texture = TextureAsset::new(
            4,
            4,
            BlockFormat::Bc7Unorm,
            vec![white_bc7_block()],
            TextureLimits::default(),
        )
        .expect("one 4x4 block");
        let decoded = texture.decode();
        assert_eq!(decoded.len(), 4 * 4 * 4);
        assert!(
            decoded
                .chunks_exact(4)
                .all(|texel| texel == [255, 255, 255, 255]),
            "a mode-6 block of maximum endpoints is opaque white, got {:?}",
            &decoded[..4]
        );
        assert_eq!(texture.decode_level(1), None, "there is no level 1");
    }
}
