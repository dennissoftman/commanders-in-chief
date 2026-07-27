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
//! # Mip levels are generated here, on the CPU
//!
//! Not an optimisation — a correctness requirement. A strategic-zoom camera minifies a detail texture
//! by two orders of magnitude, and an unmipped sample of that is a field of aliasing that moves when
//! the camera does. Generating them here rather than with a GPU blit chain keeps this a pure function
//! over bytes, which is testable without a device.
//!
//! Both the resample and the mip reduction average in **linear** light, not in stored sRGB. Averaging
//! encoded values is brighter than the correct result — the encode curve is concave, so the mean of
//! two encoded values sits above the encoding of their mean — and on a high-contrast texture that
//! reads as a surface that pales as it recedes.

use crate::RenderError;

/// Format every colour array uses. sRGB, because albedo is authored in sRGB and the hardware's
/// conversion on read is free.
pub const ARRAY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

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
        let (width, height) = (width.max(1), height.max(1));
        if width == self.width && height == self.height {
            return self.clone();
        }
        Self {
            width,
            height,
            rgba: encode(&resample(
                &decode(&self.rgba),
                self.width,
                self.height,
                width,
                height,
            )),
        }
    }
}

/// A colour texture array on the GPU, with every mip level filled.
#[derive(Debug)]
pub struct TextureArray {
    view: wgpu::TextureView,
    width: u32,
    height: u32,
    layers: u32,
    mip_levels: u32,
}

impl TextureArray {
    /// Uploads images as the slices of one array, resampling each to the largest size present.
    ///
    /// An empty slice list yields a single opaque-white slice rather than an error, because that is
    /// exactly what a caller with no textures needs: white multiplies its material colour through
    /// unchanged, so one code path covers both cases.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::TextureTooLarge`] when the slices and their mip levels exceed
    /// [`MAX_ARRAY_BYTES`], or [`RenderError::InvalidTexture`] when the layer count does not fit an
    /// array texture.
    pub fn new(
        context: &crate::GpuContext,
        label: &str,
        images: &[TextureImage],
    ) -> Result<Self, RenderError> {
        let white = [TextureImage::solid(1, 1, [u8::MAX; 4])];
        let images = if images.is_empty() {
            &white[..]
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
            format: ARRAY_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let queue = context.queue();
        for (index, image) in images.iter().enumerate() {
            let slice = image.resampled(width, height);
            let layer = u32::try_from(index).map_err(|_| RenderError::InvalidTexture)?;
            let mut level_width = width;
            let mut level_height = height;
            let mut linear = decode(slice.rgba());
            for level in 0..mip_levels {
                if level > 0 {
                    let (next_width, next_height) = (halve(level_width), halve(level_height));
                    linear = reduce(&linear, level_width, level_height);
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
                    &encode(&linear),
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
        })
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

/// Number of mip levels down to a 1x1 tail.
#[must_use]
pub const fn mip_level_count(width: u32, height: u32) -> u32 {
    let largest = if width > height { width } else { height };
    // `ilog2` of the largest dimension, plus the base level. `max(1)` guards the zero case, which
    // `TextureImage` already refuses but this function does not require.
    if largest <= 1 { 1 } else { largest.ilog2() + 1 }
}

const fn halve(value: u32) -> u32 {
    if value > 1 { value / 2 } else { 1 }
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

/// Expands stored bytes to linear-light floats, leaving alpha alone.
///
/// Alpha is not gamma encoded — it is a coverage fraction, and running it through the colour transfer
/// function would make every partially transparent edge the wrong opacity.
fn decode(rgba: &[u8]) -> Vec<f32> {
    rgba.iter()
        .enumerate()
        .map(|(index, value)| {
            if index % 4 == 3 {
                f32::from(*value) / 255.0
            } else {
                srgb_to_linear(*value)
            }
        })
        .collect()
}

/// Re-encodes linear-light floats as stored bytes.
fn encode(linear: &[f32]) -> Vec<u8> {
    linear
        .iter()
        .enumerate()
        .map(|(index, value)| {
            if index % 4 == 3 {
                quantize(*value)
            } else {
                quantize(linear_to_srgb(*value))
            }
        })
        .collect()
}

fn srgb_to_linear(value: u8) -> f32 {
    let value = f32::from(value) / 255.0;
    if value <= 0.040_45 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(value: f32) -> f32 {
    if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

/// Rounds a `0..=1` value to a byte.
fn quantize(value: f32) -> u8 {
    // Clamped into `0..=255` immediately before the cast, so neither bound can be crossed.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    {
        (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
    }
}

/// Bilinearly resamples linear-light RGBA.
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
fn resample(
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

/// Reduces linear-light RGBA to the next mip level by averaging each 2x2 block.
///
/// An odd dimension halves downward and the trailing row or column is folded into its neighbour by
/// the clamp, which is what keeps a 3-wide level from losing its last column entirely.
fn reduce(source: &[f32], width: u32, height: u32) -> Vec<f32> {
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
    use super::{
        MAX_ARRAY_DIMENSION, TextureImage, decode, encode, mip_level_count, reduce, resample,
        total_bytes,
    };
    use crate::RenderError;

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
    fn a_mip_chain_reaches_one_by_one() {
        assert_eq!(mip_level_count(1, 1), 1);
        assert_eq!(mip_level_count(2, 1), 2);
        assert_eq!(mip_level_count(256, 64), 9);
        // A non-power-of-two largest dimension still terminates, one level past its floor log.
        assert_eq!(mip_level_count(300, 7), 9);
    }

    #[test]
    fn reduction_averages_in_linear_light_not_in_stored_bytes() {
        // The whole reason the mip generator round-trips through linear. Half black and half white,
        // reduced to one texel: the correct answer is mid *grey in linear terms*, which encodes to
        // about 188 — far from the 128 a naive average of the stored bytes produces.
        let black_and_white: Vec<u8> = [[0, 0, 0, 255], [255, 255, 255, 255]]
            .into_iter()
            .cycle()
            .take(4)
            .flatten()
            .collect();
        let reduced = encode(&reduce(&decode(&black_and_white), 2, 2));
        assert_eq!(reduced.len(), 4);
        assert!(
            (185..=190).contains(&reduced[0]),
            "expected a linear-light average near 188, got {}",
            reduced[0]
        );
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
