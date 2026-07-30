//! Colour texture arrays: one common size, a full mip chain, and the upload.
//!
//! # Why an array rather than one texture per material
//!
//! Both things that need texturing here draw in a single call over many surfaces. A terrain blends up
//! to eight layers in one fragment, and a model concatenates its primitives so the whole thing is one
//! instanced draw. Neither can bind a texture per surface without giving that up — so the surfaces
//! index *into* one bound resource instead, and a `D2Array` is the baseline way to do that.
//!
//! The array's constraint is that every slice shares one size. Images that arrive at different sizes
//! are resampled to the largest, which costs memory for the smaller ones and loses nothing. The
//! alternative — packing everything into one atlas — keeps each image at its authored size but breaks
//! wrapped coordinates and bleeds across rect edges at every mip level below the gutter. Terrain
//! detail textures are tiled by definition, so that alternative was not available for half the callers
//! and was not worth having in two forms.
//!
//! # Two ways in: uncompressed with mips generated here, or block-compressed with mips already in it
//!
//! [`TextureArray::new_in`] takes RGBA8 images and builds their mip chains on the CPU at upload. That is
//! not an optimisation but a correctness requirement: a strategic-zoom camera minifies a detail texture
//! by two orders of magnitude, and an unmipped sample of that is a field of aliasing that moves when the
//! camera does.
//!
//! [`TextureArray::new_blocks`] takes textures that arrive already compressed and already mipped, and
//! copies them. ADR 0004 predicted this: it recorded that if per-slice CPU mip generation ever cost too
//! much, the answer was precomputed mips in the asset. ADR 2001 took it, so the two paths now coexist and
//! neither is a fallback for the other — an authored `.dds` uses the second, and a procedural or
//! placeholder image uses the first.
//!
//! What they share is [`cic_assets::image`], which owns the averaging both depend on. That is deliberate:
//! the mip chain baked into a `.dds` by the converter and the one this module builds have to be the same
//! arithmetic, or a converted texture would visibly differ from an unconverted one as the camera pulls
//! back.
//!
//! # Why the colour space is a parameter
//!
//! A normal map, a roughness map and a metallic map are not colours. Their bytes are measurements
//! stored linearly, and running them through the sRGB decode *undoes an encoding that was never
//! applied*: a flat normal's 128 becomes 0.216 instead of 0.502, so the surface tilts everywhere by the
//! same wrong amount and reads as a lighting bug rather than a texture one. Roughness 128 becomes 0.216
//! too, which is a different material.
//!
//! So [`ColourSpace`] governs two things together, and they must agree: the texture format the hardware
//! reads through, and whether the averaging round-trips through the transfer function. A format without
//! the matching averaging is the more insidious half of the mistake, because it is invisible at the base
//! mip level and only appears as the camera pulls back. For a block-compressed texture the space is not a
//! parameter at all — it is part of the format the file declares, which is why `BC7_UNORM` and
//! `BC7_UNORM_SRGB` are different formats.

use cic_assets::image::{decode_in, encode_in, halve, mip_level_count, reduce, resample};
use cic_assets::texture::{BLOCK_EXTENT, BlockFormat, TextureAsset};

use crate::RenderError;

// The colour-space vocabulary is shared with the asset side, which owns the averaging rules it selects
// between. Re-exported so `crate::ColourSpace` keeps naming one type rather than two that must agree.
pub use cic_assets::image::ColourSpace;

/// The texture format an uncompressed array in this space uploads in.
///
/// A free function rather than a method, because [`ColourSpace`] belongs to `cic-assets`, which has no
/// business naming a `wgpu` type. The pairing it expresses is still one decision: see the module note on
/// why the format and the averaging cannot be chosen separately.
#[must_use]
pub const fn array_format(space: ColourSpace) -> wgpu::TextureFormat {
    match space {
        ColourSpace::Srgb => ARRAY_FORMAT,
        ColourSpace::Linear => LINEAR_ARRAY_FORMAT,
    }
}

/// The `wgpu` format a block layout uploads through.
#[must_use]
pub const fn block_array_format(format: BlockFormat) -> wgpu::TextureFormat {
    match format {
        BlockFormat::Bc1RgbaUnorm => wgpu::TextureFormat::Bc1RgbaUnorm,
        BlockFormat::Bc1RgbaUnormSrgb => wgpu::TextureFormat::Bc1RgbaUnormSrgb,
        BlockFormat::Bc5Unorm => wgpu::TextureFormat::Bc5RgUnorm,
        BlockFormat::Bc7Unorm => wgpu::TextureFormat::Bc7RgbaUnorm,
        BlockFormat::Bc7UnormSrgb => wgpu::TextureFormat::Bc7RgbaUnormSrgb,
    }
}

/// Format a colour array uses. sRGB, because albedo is authored in sRGB and the hardware's
/// conversion on read is free.
pub const ARRAY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Format a linear-data array uses: the same eight bits per channel with no transfer function.
pub const LINEAR_ARRAY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Largest dimension a single array slice may have, matching the baseline device limit for a 2D
/// texture. Anything above it is refused here rather than at pipeline creation, where the failure
/// would name a texture the caller never asked about.
pub const MAX_ARRAY_DIMENSION: u32 = 8_192;

/// Largest total upload for one array, counting every slice and every mip level.
pub const MAX_ARRAY_BYTES: usize = 512 * 1_024 * 1_024;

/// One bounded straight-alpha RGBA8 image.
///
/// The single CPU-side image type in the renderer: [`crate::resource::TextureResourceManager`] caches
/// these, and [`TextureArray`] uploads them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureImage {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl TextureImage {
    /// Wraps decoded RGBA bytes, checking them against the array bounds.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::InvalidTexture`] when a dimension is zero or the byte length disagrees
    /// with the dimensions, or [`RenderError::TextureTooLarge`] when a dimension exceeds
    /// [`MAX_ARRAY_DIMENSION`].
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, RenderError> {
        if width > MAX_ARRAY_DIMENSION || height > MAX_ARRAY_DIMENSION {
            return Err(RenderError::TextureTooLarge);
        }
        if width == 0 || height == 0 || rgba.len() != pixel_bytes(width, height)? {
            return Err(RenderError::InvalidTexture);
        }
        Ok(Self {
            width,
            height,
            rgba,
        })
    }

    /// A single-colour image, for a slice a material did not supply.
    #[must_use]
    pub fn solid(width: u32, height: u32, colour: [u8; 4]) -> Self {
        let count = (width.max(1) as usize) * (height.max(1) as usize);
        Self {
            width: width.max(1),
            height: height.max(1),
            rgba: colour.repeat(count),
        }
    }

    /// Returns the image width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the image height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Returns the straight-alpha RGBA bytes, row-major from the top-left.
    #[must_use]
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// Returns this image resampled to another size, bilinearly and in linear light.
    ///
    /// A no-op clone when the size already matches, which is the common case: a model whose textures
    /// were authored to one budget never resamples at all.
    #[must_use]
    pub fn resampled(&self, width: u32, height: u32) -> Self {
        self.resampled_in(width, height, ColourSpace::Srgb)
    }

    /// Returns this image resampled to another size, averaging in the space its bytes are in.
    ///
    /// See [`ColourSpace`] for why the space has to be stated: interpolating a normal map through the
    /// sRGB transfer function decodes an encoding that was never applied, which tilts the surface
    /// everywhere by the same wrong amount.
    #[must_use]
    pub fn resampled_in(&self, width: u32, height: u32, space: ColourSpace) -> Self {
        let (width, height) = (width.max(1), height.max(1));
        if width == self.width && height == self.height {
            return self.clone();
        }
        Self {
            width,
            height,
            rgba: encode_in(
                &resample(
                    &decode_in(&self.rgba, space),
                    self.width,
                    self.height,
                    width,
                    height,
                ),
                space,
            ),
        }
    }
}

/// A texture array on the GPU, with every mip level filled.
#[derive(Debug)]
pub struct TextureArray {
    view: wgpu::TextureView,
    width: u32,
    height: u32,
    layers: u32,
    mip_levels: u32,
    space: ColourSpace,
    blocks: Option<BlockFormat>,
}

impl TextureArray {
    /// Uploads already-compressed, already-mipped textures as the slices of one array.
    ///
    /// The fast path, and a copy rather than a conversion: the blocks the converter wrote reach the
    /// texture unit exactly as they are, which is the whole reason for the format. Nothing here decodes,
    /// resamples or reduces anything.
    ///
    /// # Why every slice must match
    ///
    /// [`Self::new_in`] resamples a small slice up to the array's size. That option does not exist here:
    /// resampling compressed blocks means decoding them, resampling, and *re-encoding* — which is the
    /// expensive half of an offline tool, at load time, to produce a worse image than converting the
    /// texture at the right size would have. So a disagreement is refused, and the caller either fixes
    /// the asset or falls back to the uncompressed path. See `crate::model`, which does the latter per
    /// slot.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::BlockCompressionUnsupported`] when the device lacks
    /// `TEXTURE_COMPRESSION_BC` — ask [`crate::GpuContext::supports_block_compression`] first and take
    /// the uncompressed path instead. Returns [`RenderError::MismatchedTextureSlices`] when the slices
    /// disagree on size, format or mip count, [`RenderError::InvalidTexture`] for an empty slice list or
    /// a layer count that does not fit an array texture, and [`RenderError::TextureTooLarge`] when the
    /// upload exceeds [`MAX_ARRAY_BYTES`].
    pub fn new_blocks(
        context: &crate::GpuContext,
        label: &str,
        slices: &[&TextureAsset],
    ) -> Result<Self, RenderError> {
        if !context.supports_block_compression() {
            return Err(RenderError::BlockCompressionUnsupported);
        }
        let Some(first) = slices.first() else {
            return Err(RenderError::InvalidTexture);
        };
        let (width, height) = (first.width(), first.height());
        let format = first.format();
        let mip_levels = first.level_count();
        for slice in slices {
            if slice.width() != width
                || slice.height() != height
                || slice.format() != format
                || slice.level_count() != mip_levels
            {
                return Err(RenderError::MismatchedTextureSlices {
                    expected: [width, height],
                    found: [slice.width(), slice.height()],
                });
            }
        }
        if width > MAX_ARRAY_DIMENSION || height > MAX_ARRAY_DIMENSION {
            return Err(RenderError::TextureTooLarge);
        }
        let layers = u32::try_from(slices.len()).map_err(|_| RenderError::InvalidTexture)?;
        let total = slices
            .iter()
            .try_fold(0usize, |total, slice| total.checked_add(slice.byte_count()))
            .ok_or(RenderError::TextureTooLarge)?;
        if total > MAX_ARRAY_BYTES {
            return Err(RenderError::TextureTooLarge);
        }

        let texture = context.device().create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: layers,
            },
            mip_level_count: mip_levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: block_array_format(format),
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let queue = context.queue();
        for (index, slice) in slices.iter().enumerate() {
            let layer = u32::try_from(index).map_err(|_| RenderError::InvalidTexture)?;
            for level in 0..mip_levels {
                let payload = slice.level(level).ok_or(RenderError::InvalidTexture)?;
                let (level_width, level_height) = slice.level_size(level);
                // A compressed row pitch is counted in *blocks*, not texels, and a level narrower than
                // one block still occupies a whole one -- which is every level below 4x4 in the chain.
                let blocks_across = level_width.div_ceil(BLOCK_EXTENT).max(1);
                let blocks_down = level_height.div_ceil(BLOCK_EXTENT).max(1);
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: level,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: layer,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    payload,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(blocks_across * format.block_bytes()),
                        rows_per_image: Some(blocks_down),
                    },
                    // The copy extent is in texels, and it is the level's *physical* size — rounded up to
                    // whole blocks — not its logical one. A copy into a compressed texture must be
                    // block-aligned in both axes with no exception for the last partial block, so a 2x2
                    // mip level is copied as the 4x4 block that holds it. Every mip chain ends in three or
                    // four such levels, so this is the ordinary path and not an edge case; passing the
                    // logical size instead is rejected outright by validation rather than silently
                    // misplacing anything, which is the good kind of mistake to make.
                    wgpu::Extent3d {
                        width: level_width,
                        height: level_height,
                        depth_or_array_layers: 1,
                    }
                    .physical_size(block_array_format(format)),
                );
            }
        }

        Ok(Self {
            view: texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some(label),
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            }),
            width,
            height,
            layers,
            mip_levels,
            space: format.colour_space(),
            blocks: Some(format),
        })
    }

    /// Uploads sRGB colour images as the slices of one array. See [`Self::new_in`].
    ///
    /// # Errors
    ///
    /// As [`Self::new_in`].
    pub fn new(
        context: &crate::GpuContext,
        label: &str,
        images: &[TextureImage],
    ) -> Result<Self, RenderError> {
        Self::new_in(context, label, images, ColourSpace::Srgb, [u8::MAX; 4])
    }

    /// Uploads images as the slices of one array, resampling each to the largest size present.
    ///
    /// An empty slice list yields a single slice of `fallback` rather than an error, because that is
    /// exactly what a caller with no textures needs: one code path covers both cases as long as the
    /// fallback is the identity for whatever the array means. For colour that is opaque white, which
    /// multiplies a material colour through unchanged; for a normal map it is the encoded flat normal,
    /// which perturbs nothing. A caller supplying the wrong one gets a *plausible* image rather than an
    /// error, which is why it is a parameter rather than a constant here.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::TextureTooLarge`] when the slices and their mip levels exceed
    /// [`MAX_ARRAY_BYTES`], or [`RenderError::InvalidTexture`] when the layer count does not fit an
    /// array texture.
    pub fn new_in(
        context: &crate::GpuContext,
        label: &str,
        images: &[TextureImage],
        space: ColourSpace,
        fallback: [u8; 4],
    ) -> Result<Self, RenderError> {
        let default = [TextureImage::solid(1, 1, fallback)];
        let images = if images.is_empty() {
            &default[..]
        } else {
            images
        };

        // The largest size present, so no slice is ever downsampled and detail is never discarded to
        // satisfy a slice that happened to be authored small.
        let width = images.iter().map(TextureImage::width).max().unwrap_or(1);
        let height = images.iter().map(TextureImage::height).max().unwrap_or(1);
        let layers = u32::try_from(images.len()).map_err(|_| RenderError::InvalidTexture)?;
        let mip_levels = mip_level_count(width, height);

        let total = total_bytes(width, height, layers, mip_levels)?;
        if total > MAX_ARRAY_BYTES {
            return Err(RenderError::TextureTooLarge);
        }

        let device = context.device();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: layers,
            },
            mip_level_count: mip_levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: array_format(space),
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let queue = context.queue();
        for (index, image) in images.iter().enumerate() {
            let slice = image.resampled_in(width, height, space);
            let layer = u32::try_from(index).map_err(|_| RenderError::InvalidTexture)?;
            let mut level_width = width;
            let mut level_height = height;
            let mut working = decode_in(slice.rgba(), space);
            for level in 0..mip_levels {
                if level > 0 {
                    let (next_width, next_height) = (halve(level_width), halve(level_height));
                    working = reduce(&working, level_width, level_height);
                    level_width = next_width;
                    level_height = next_height;
                }
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: level,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: layer,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    &encode_in(&working, space),
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(level_width * 4),
                        rows_per_image: Some(level_height),
                    },
                    wgpu::Extent3d {
                        width: level_width,
                        height: level_height,
                        depth_or_array_layers: 1,
                    },
                );
            }
        }

        Ok(Self {
            view: texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some(label),
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            }),
            width,
            height,
            layers,
            mip_levels,
            space,
            blocks: None,
        })
    }

    /// Returns the space this array's bytes are in, and therefore the format it was uploaded as.
    #[must_use]
    pub const fn colour_space(&self) -> ColourSpace {
        self.space
    }

    /// Returns the block layout this array was uploaded as, or `None` when it holds plain RGBA8.
    ///
    /// Worth asking rather than inferring: the two paths produce arrays a shader binds identically, so
    /// the only thing that can tell them apart is the array itself. A test asserting that a model
    /// actually took the compressed path reads this.
    #[must_use]
    pub const fn block_format(&self) -> Option<BlockFormat> {
        self.blocks
    }

    /// Returns the array view, for binding.
    #[must_use]
    pub const fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Returns the slice size every layer shares.
    #[must_use]
    pub const fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Returns how many slices the array holds.
    #[must_use]
    pub const fn layer_count(&self) -> u32 {
        self.layers
    }

    /// Returns how many mip levels were generated.
    #[must_use]
    pub const fn mip_level_count(&self) -> u32 {
        self.mip_levels
    }
}

/// Builds the sampler colour arrays are read through: repeating, trilinear.
///
/// Repeating rather than clamped, because terrain detail textures tile by definition and a model's
/// coordinates outside `0..1` mean the author intended a repeat. Trilinear rather than anisotropic:
/// anisotropic filtering is an optional device capability, and a sampler that fails to create on a
/// software adapter would take the headless tests with it.
#[must_use]
pub fn array_sampler(device: &wgpu::Device, label: &str) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some(label),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        address_mode_w: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    })
}

fn pixel_bytes(width: u32, height: u32) -> Result<usize, RenderError> {
    usize::try_from(width)
        .ok()
        .zip(usize::try_from(height).ok())
        .and_then(|(width, height)| width.checked_mul(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(RenderError::TextureTooLarge)
}

/// Bytes an array occupies across every slice and every mip level.
fn total_bytes(
    width: u32,
    height: u32,
    layers: u32,
    mip_levels: u32,
) -> Result<usize, RenderError> {
    let mut total = 0usize;
    let (mut level_width, mut level_height) = (width, height);
    for _ in 0..mip_levels {
        let level = pixel_bytes(level_width, level_height)?;
        total = total
            .checked_add(level)
            .ok_or(RenderError::TextureTooLarge)?;
        level_width = halve(level_width);
        level_height = halve(level_height);
    }
    total
        .checked_mul(usize::try_from(layers).map_err(|_| RenderError::TextureTooLarge)?)
        .ok_or(RenderError::TextureTooLarge)
}

#[cfg(test)]
mod tests {
    use super::{MAX_ARRAY_DIMENSION, TextureImage, array_format, block_array_format, total_bytes};
    use crate::RenderError;
    use cic_assets::image::ColourSpace;
    use cic_assets::texture::BlockFormat;

    // The arithmetic these arrays rest on -- averaging in linear light, the half-texel resample
    // convention, where a mip chain ends -- is tested in `cic_assets::image`, which owns it so that the
    // offline converter and this uploader cannot drift apart. What is left here is what belongs to the
    // renderer: the bounds an image is checked against, the byte budget, and the format pairings.

    #[test]
    fn each_space_and_each_block_layout_declares_its_own_format() {
        // The format and the averaging have to agree, and this is the pair that makes them one decision
        // rather than two. For a block layout the space is already inside the format, which is the whole
        // reason `BC7_UNORM` and `BC7_UNORM_SRGB` are separate formats rather than one plus a flag.
        assert_eq!(
            array_format(ColourSpace::Srgb),
            wgpu::TextureFormat::Rgba8UnormSrgb
        );
        assert_eq!(
            array_format(ColourSpace::Linear),
            wgpu::TextureFormat::Rgba8Unorm
        );
        assert_eq!(
            block_array_format(BlockFormat::Bc7UnormSrgb),
            wgpu::TextureFormat::Bc7RgbaUnormSrgb
        );
        assert_eq!(
            block_array_format(BlockFormat::Bc7Unorm),
            wgpu::TextureFormat::Bc7RgbaUnorm
        );
        assert_eq!(
            block_array_format(BlockFormat::Bc5Unorm),
            wgpu::TextureFormat::Bc5RgUnorm
        );
        assert_eq!(
            block_array_format(BlockFormat::Bc1RgbaUnormSrgb),
            wgpu::TextureFormat::Bc1RgbaUnormSrgb
        );
        assert_eq!(
            block_array_format(BlockFormat::Bc1RgbaUnorm),
            wgpu::TextureFormat::Bc1RgbaUnorm
        );
    }

    #[test]
    fn an_image_checks_its_own_dimensions() {
        assert!(TextureImage::new(2, 2, vec![0; 16]).is_ok());
        assert!(matches!(
            TextureImage::new(2, 2, vec![0; 15]),
            Err(RenderError::InvalidTexture)
        ));
        assert!(matches!(
            TextureImage::new(0, 2, Vec::new()),
            Err(RenderError::InvalidTexture)
        ));
        assert!(matches!(
            TextureImage::new(MAX_ARRAY_DIMENSION + 1, 1, Vec::new()),
            Err(RenderError::TextureTooLarge)
        ));
    }

    #[test]
    fn resampling_to_the_same_size_is_the_identity() {
        // The half-texel convention is what makes this true. Mapping destination *corners* to source
        // corners instead shifts everything half a pixel, which reads as a texture that crawls when a
        // model is rebuilt at a different size.
        let image = TextureImage::new(4, 4, (0u8..64).collect()).expect("valid image");
        assert_eq!(image.resampled(4, 4), image);
    }

    #[test]
    fn an_upsample_interpolates_rather_than_repeating() {
        // Two texels, black then white, doubled. A nearest-neighbour implementation would give two
        // blacks and two whites; bilinear must produce intermediate values in the middle.
        let image =
            TextureImage::new(2, 1, vec![0, 0, 0, 255, 255, 255, 255, 255]).expect("valid image");
        let wide = image.resampled(4, 1);
        let rgba = wide.rgba();
        assert_eq!(rgba[0], 0, "the left edge stays black");
        assert_eq!(rgba[12], 255, "the right edge stays white");
        assert!(
            rgba[4] > 0 && rgba[8] < 255 && rgba[4] < rgba[8],
            "the interior must ramp, got {rgba:?}"
        );
    }

    #[test]
    fn a_linear_resample_interpolates_without_a_transfer_function() {
        // Two texels, 0 then 200, doubled. Linearly the interior samples land at a quarter and three
        // quarters of the way, so the near-left one is 50 -- through the sRGB path it would be 111.
        let image = TextureImage::new(2, 1, vec![0, 0, 0, 255, 200, 200, 200, 255]).expect("valid");
        let wide = image.resampled_in(4, 1, ColourSpace::Linear);
        assert_eq!(wide.rgba()[4], 50, "got {:?}", wide.rgba());
    }

    #[test]
    fn a_solid_image_is_uniform() {
        let image = TextureImage::solid(3, 2, [10, 20, 30, 40]);
        assert_eq!(image.width(), 3);
        assert_eq!(image.height(), 2);
        assert_eq!(image.rgba().len(), 24);
        assert!(image.rgba().chunks_exact(4).all(|p| p == [10, 20, 30, 40]));
    }

    #[test]
    fn the_byte_total_counts_every_level_of_every_slice() {
        // A 2x2 array of two slices: 16 bytes at level 0 plus 4 at level 1, doubled.
        assert_eq!(total_bytes(2, 2, 2, 2).expect("fits"), (16 + 4) * 2);
        // The tail of a mip chain is not free, and a budget check that ignored it would under-count
        // by a third on a square texture.
        assert_eq!(total_bytes(4, 4, 1, 3).expect("fits"), 64 + 16 + 4);
    }
}
