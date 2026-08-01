//! Headless GPU device acquisition and framebuffer readback.
//!
//! Headless comes first, before any window. A capture is the only rendering verification that works
//! in CI, and a renderer that can only draw into a window is one whose output nothing can check.
//! Windowed presentation is the same passes pointed at a surface instead.

use std::sync::mpsc;
use std::time::Duration;

use crate::RenderError;

/// Bytes per pixel in a captured framebuffer, which is always `Rgba8UnormSrgb`.
const BYTES_PER_PIXEL: u32 = 4;

/// Largest capture dimension accepted, so a mistaken size cannot ask for a terabyte.
const MAX_CAPTURE_DIMENSION: u32 = 8_192;

/// Largest readback buffer accepted.
const MAX_CAPTURE_BUFFER_BYTES: u64 = 1_024 * 1_024 * 1_024;

/// Colour format every capture uses.
pub const CAPTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Depth format every pass uses.
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// A device, its queue, and the adapter they came from.
///
/// The adapter is retained deliberately. Surface capabilities are queried *through* an adapter, and
/// that adapter must be the one whose instance owns the surface — querying with an adapter from a
/// different instance is not a wrong answer but a hard failure inside the graphics layer.
#[derive(Debug)]
pub struct GpuContext {
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter: wgpu::Adapter,
    adapter_info: wgpu::AdapterInfo,
}

/// Optional features requested when the adapter offers them, and silently skipped when it does not.
///
/// Both entries are *optional* on purpose, and for the same reason: asking for a feature the adapter
/// lacks fails device creation outright, so the request is intersected with what is available rather
/// than assumed. A renderer that refused to start on a software adapter would be a renderer CI cannot
/// run.
///
/// `TIMESTAMP_QUERY` is what [`crate::timing`] needs to attribute frame time to individual passes; the
/// answer to its absence is a renderer that draws without timing.
///
/// `TEXTURE_COMPRESSION_BC` is what [`crate::texture::TextureArray::new_blocks`] needs to hand a `.dds`
/// straight to the texture unit. Every desktop GPU has it, and so does Mesa's `llvmpipe`, which
/// decompresses in software — so the CI runner takes the compressed path too. The answer to its absence is
/// the decode in [`cic_assets::bc`] and the uncompressed upload path: the same picture, built the slow way.
/// Ask [`GpuContext::supports_block_compression`] rather than assuming either, because *which* adapters
/// have it is not a thing to hold in your head.
const OPTIONAL_FEATURES: wgpu::Features =
    wgpu::Features::TIMESTAMP_QUERY.union(wgpu::Features::TEXTURE_COMPRESSION_BC);

/// Builds the instance both entry points share, honouring `WGPU_BACKEND` and the other `WGPU_*`
/// variables.
///
/// `wgpu::Instance::default()` reads none of them, and the difference matters because a committed
/// reference image belongs to one *backend* as much as to one adapter — see [`crate::regression`],
/// where the backend is half of the set's name. This crate compiles `dx12`, `metal` and `vulkan`, so on
/// Windows one card is reachable two ways and which of them renders a regenerated reference set was
/// wgpu's preference order rather than anybody's decision. Being able to pin it is also what makes the
/// no-adapter path testable at all, since forcing a backend that is not compiled in is the only way to
/// reach it on a machine that has a GPU. Setting no variable leaves every field at its default, so this
/// changes nothing unless something asks it to.
fn instance() -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env())
}

/// Bind groups the renderer's deepest pipeline needs bound at once.
///
/// The water pass is that pipeline: the scene and its G-buffer, its own uniform, the shadow cascades,
/// the environment, and — once a reflection provider samples the lit scene — a fifth group for what it
/// reads. `wgpu::Limits::default()` stops at four, which is one short, and four is not an arbitrary
/// default: it is Vulkan's own guaranteed minimum for `maxBoundDescriptorSets`.
///
/// So this is a real requirement rather than a preference, and it is stated as a constant so a device
/// that cannot meet it fails at creation with a reason rather than at pipeline creation with a
/// validation message that names a layout instead of a cause.
pub(crate) const REQUIRED_BIND_GROUPS: u32 = 5;

/// Refuses a device that cannot bind what the deepest pipeline needs.
///
/// Checked against the *device* rather than the adapter, because the device is what a pipeline is
/// created on and the two can differ — the descriptor asks for what the adapter offers, and a backend
/// is free to hand back less.
fn check_bind_groups(device: &wgpu::Device) -> Result<(), RenderError> {
    let offered = device.limits().max_bind_groups;
    if offered < REQUIRED_BIND_GROUPS {
        return Err(RenderError::BindGroupLimit(offered));
    }
    Ok(())
}

/// Builds a device descriptor asking for whichever optional features this adapter has, and for as many
/// bind groups as it will give.
///
/// Taken from the adapter rather than written as a number, which is what makes the raise cost nothing:
/// the device asks for exactly what the hardware already reports, so no adapter that worked before
/// stops working. Every desktop backend answers eight here — the figure wgpu itself caps at — and the
/// two this project is tested on, an RTX 4080 and Mesa's lavapipe, both do.
///
/// Every other limit stays at its default deliberately. Raising one because a pass needs it is a
/// different thing from raising all of them because they were there for the taking, and the second is
/// how a renderer quietly acquires a hardware requirement nobody decided on.
fn device_descriptor<'a>(adapter: &wgpu::Adapter, label: &'a str) -> wgpu::DeviceDescriptor<'a> {
    wgpu::DeviceDescriptor {
        label: Some(label),
        required_features: adapter.features() & OPTIONAL_FEATURES,
        required_limits: wgpu::Limits {
            max_bind_groups: adapter.limits().max_bind_groups,
            ..wgpu::Limits::default()
        },
        ..Default::default()
    }
}

impl GpuContext {
    /// Requests an adapter and a device, with optional features taken where offered.
    ///
    /// A fallback (software) adapter is tried when no native adapter answers, because CI runners
    /// frequently have no GPU and a renderer that cannot be tested there is untested.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::RequestAdapter`] or [`RenderError::RequestDevice`] when neither a
    /// native nor a fallback device can be created.
    pub async fn new() -> Result<Self, RenderError> {
        let instance = instance();
        let mut options = wgpu::RequestAdapterOptions::default();
        let adapter = if let Ok(adapter) = instance.request_adapter(&options).await {
            adapter
        } else {
            options.force_fallback_adapter = true;
            instance
                .request_adapter(&options)
                .await
                .map_err(RenderError::RequestAdapter)?
        };
        let adapter_info = adapter.get_info();
        let (device, queue) = adapter
            .request_device(&device_descriptor(&adapter, "cic-render device"))
            .await
            .map_err(RenderError::RequestDevice)?;
        check_bind_groups(&device)?;
        Ok(Self {
            device,
            queue,
            adapter,
            adapter_info,
        })
    }

    /// Requests a device for a window, returning the surface alongside it.
    ///
    /// The surface is created *before* the adapter, and passed as `compatible_surface`, because an
    /// adapter chosen without one is not guaranteed to be able to present to it. On a machine with
    /// both an integrated and a discrete GPU that is not hypothetical: the surface may belong to one
    /// and the chosen adapter to the other.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::CreateSurface`] when the window cannot provide a surface, or the same
    /// adapter and device errors as [`Self::new`].
    pub async fn for_window(
        target: impl Into<wgpu::SurfaceTarget<'static>>,
    ) -> Result<(Self, wgpu::Surface<'static>), RenderError> {
        let instance = instance();
        let surface = instance
            .create_surface(target)
            .map_err(|error| RenderError::CreateSurface(error.to_string()))?;
        let mut options = wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        };
        let adapter = if let Ok(adapter) = instance.request_adapter(&options).await {
            adapter
        } else {
            options.force_fallback_adapter = true;
            instance
                .request_adapter(&options)
                .await
                .map_err(RenderError::RequestAdapter)?
        };
        let adapter_info = adapter.get_info();
        let (device, queue) = adapter
            .request_device(&device_descriptor(
                &adapter,
                "cic-render presentation device",
            ))
            .await
            .map_err(RenderError::RequestDevice)?;
        check_bind_groups(&device)?;
        Ok((
            Self {
                device,
                queue,
                adapter,
                adapter_info,
            },
            surface,
        ))
    }

    /// Returns the device.
    #[must_use]
    pub const fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// Returns the queue.
    #[must_use]
    pub const fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Returns the adapter, for querying surface capabilities.
    ///
    /// It must be this one: a surface's capabilities can only be queried through an adapter belonging
    /// to the same instance that created the surface.
    #[must_use]
    pub const fn adapter(&self) -> &wgpu::Adapter {
        &self.adapter
    }

    /// Returns which adapter answered, for diagnostics and for capture comparisons that may differ
    /// between a software rasteriser and real hardware.
    #[must_use]
    pub const fn adapter_info(&self) -> &wgpu::AdapterInfo {
        &self.adapter_info
    }

    /// Whether this device can attribute time to individual passes.
    ///
    /// False is a normal answer, not a fault: `TIMESTAMP_QUERY` is optional, and a device without it
    /// renders identically and reports no timings. Callers should treat it the way the render tests treat
    /// a missing adapter — skip the measurement, do not fail.
    #[must_use]
    pub fn supports_timing(&self) -> bool {
        self.device
            .features()
            .contains(wgpu::Features::TIMESTAMP_QUERY)
    }

    /// Whether this device can sample block-compressed textures.
    ///
    /// False is a normal answer, not a fault, and it changes what a caller *does* rather than whether it
    /// succeeds: a `.dds` still loads, but its blocks are decoded on the CPU and uploaded as RGBA8 instead
    /// of being copied. The picture is the same to within a bit; the memory and the load time are not.
    ///
    /// Both paths are live and both need to work, which is why this is asked rather than assumed. It is
    /// *not* a proxy for "is this a real GPU": Mesa's `llvmpipe` reports true and decompresses in software,
    /// so the CI runner exercises the compressed path — and its decompression is not bit-identical to a
    /// hardware one, which is a property of the format rather than a fault. See
    /// [`crate::texture::TextureArray::new_blocks`].
    #[must_use]
    pub fn supports_block_compression(&self) -> bool {
        self.device
            .features()
            .contains(wgpu::Features::TEXTURE_COMPRESSION_BC)
    }
}

/// A colour and depth target pair that can be read back.
#[derive(Debug)]
pub struct CaptureTarget {
    colour: wgpu::Texture,
    colour_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    readback: wgpu::Buffer,
    width: u32,
    height: u32,
    unpadded_row: u32,
    padded_row: u32,
}

impl CaptureTarget {
    /// Allocates a colour target, a depth target, and a readback buffer.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::EmptyCapture`] for a zero dimension or [`RenderError::CaptureTooLarge`]
    /// when the requested size exceeds the bounds above.
    pub fn new(context: &GpuContext, width: u32, height: u32) -> Result<Self, RenderError> {
        let (unpadded_row, padded_row, buffer_size) = capture_layout(width, height)?;
        let device = context.device();
        let extent = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let colour = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cic-render capture colour"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: CAPTURE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cic-render capture depth"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cic-render capture readback"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Ok(Self {
            colour_view: colour.create_view(&wgpu::TextureViewDescriptor::default()),
            colour,
            depth_view: depth.create_view(&wgpu::TextureViewDescriptor::default()),
            readback,
            width,
            height,
            unpadded_row,
            padded_row,
        })
    }

    /// Returns the colour attachment view.
    #[must_use]
    pub const fn colour_view(&self) -> &wgpu::TextureView {
        &self.colour_view
    }

    /// Returns the depth attachment view.
    #[must_use]
    pub const fn depth_view(&self) -> &wgpu::TextureView {
        &self.depth_view
    }

    /// Returns the capture width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the capture height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Copies the colour target into the readback buffer, submits, waits, and unpads the result.
    ///
    /// # Errors
    ///
    /// Returns a structured [`RenderError`] when submission, polling, mapping, or the map callback
    /// fails or times out.
    pub fn resolve(
        &self,
        context: &GpuContext,
        mut encoder: wgpu::CommandEncoder,
    ) -> Result<Capture, RenderError> {
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.colour,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );

        let submission = context.queue().submit([encoder.finish()]);
        let slice = self.readback.slice(..);
        let (sender, receiver) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        context
            .device()
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(Duration::from_secs(30)),
            })
            .map_err(RenderError::Poll)?;
        receiver
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| RenderError::MapCallbackTimeout)?
            .map_err(RenderError::MapBuffer)?;

        let mapped = slice.get_mapped_range().map_err(RenderError::MapRange)?;
        let unpadded =
            usize::try_from(self.unpadded_row).map_err(|_| RenderError::CaptureTooLarge)?;
        let padded = usize::try_from(self.padded_row).map_err(|_| RenderError::CaptureTooLarge)?;
        let mut rgba = Vec::with_capacity(unpadded * self.height as usize);
        // The copy pads each row to the alignment the API requires; the caller wants tight rows.
        for row in mapped.chunks_exact(padded) {
            rgba.extend_from_slice(&row[..unpadded]);
        }
        drop(mapped);
        self.readback.unmap();

        Ok(Capture {
            width: self.width,
            height: self.height,
            rgba,
        })
    }
}

/// A resolved framebuffer read back to host memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capture {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl Capture {
    /// Returns the capture width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the capture height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Returns tightly packed RGBA bytes, row-major from the top-left.
    #[must_use]
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// Returns the pixel at a position, or `None` when out of range.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let offset = (y as usize * self.width as usize + x as usize) * 4;
        let bytes = self.rgba.get(offset..offset + 4)?;
        Some([bytes[0], bytes[1], bytes[2], bytes[3]])
    }

    /// Returns how many distinct colours the capture contains.
    ///
    /// A cheap sanity signal that beats "the buffer is non-zero": a solid clear colour, an
    /// all-black frame, and a two-tone silhouette are all distinguishable by this number, so a test
    /// can assert that shading actually varied without committing a reference image.
    #[must_use]
    pub fn distinct_colours(&self) -> usize {
        let mut seen = std::collections::BTreeSet::new();
        for pixel in self.rgba.chunks_exact(4) {
            seen.insert([pixel[0], pixel[1], pixel[2], pixel[3]]);
        }
        seen.len()
    }

    /// Returns the lowest and highest perceptual luminance in the capture, each in `0..=1`.
    ///
    /// A far better shading signal than a colour count, which mostly reports how varied the *scene*
    /// was. A lit surface with real relief spans a wide luminance range; a flat or unlit one does
    /// not, whatever its palette.
    #[must_use]
    pub fn luminance_range(&self) -> (f32, f32) {
        let mut lowest = f32::INFINITY;
        let mut highest = f32::NEG_INFINITY;
        for pixel in self.rgba.chunks_exact(4) {
            let value = luminance(pixel);
            lowest = lowest.min(value);
            highest = highest.max(value);
        }
        if lowest.is_finite() {
            (lowest, highest)
        } else {
            (0.0, 0.0)
        }
    }

    /// Returns the standard deviation of perceptual luminance across the capture.
    #[must_use]
    pub fn luminance_deviation(&self) -> f32 {
        let total = self.rgba.len() / 4;
        if total == 0 {
            return 0.0;
        }
        // Pixel counts are bounded by the capture limits, far inside exact f32 range.
        #[allow(clippy::cast_precision_loss)]
        let count = total as f32;
        let mean = self.rgba.chunks_exact(4).map(luminance).sum::<f32>() / count;
        let variance = self
            .rgba
            .chunks_exact(4)
            .map(|pixel| (luminance(pixel) - mean).powi(2))
            .sum::<f32>()
            / count;
        variance.sqrt()
    }

    /// Returns the fraction of pixels that differ from a given colour.
    #[must_use]
    pub fn fraction_differing_from(&self, colour: [u8; 4]) -> f32 {
        let total = self.rgba.len() / 4;
        if total == 0 {
            return 0.0;
        }
        let differing = self
            .rgba
            .chunks_exact(4)
            .filter(|pixel| pixel[0..4] != colour[..])
            .count();
        // Both counts are pixel counts bounded by the capture limits, far inside exact f32 range.
        #[allow(clippy::cast_precision_loss)]
        {
            differing as f32 / total as f32
        }
    }

    /// Encodes the capture as a PNG.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::EncodePng`] when encoding fails.
    pub fn png(&self) -> Result<Vec<u8>, RenderError> {
        let mut output = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut output, self.width, self.height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder
                .write_header()
                .map_err(|error| RenderError::EncodePng(error.to_string()))?;
            writer
                .write_image_data(&self.rgba)
                .map_err(|error| RenderError::EncodePng(error.to_string()))?;
            writer
                .finish()
                .map_err(|error| RenderError::EncodePng(error.to_string()))?;
        }
        Ok(output)
    }
}

/// Rec. 709 relative luminance of an 8-bit pixel, normalized to `0..=1`.
fn luminance(pixel: &[u8]) -> f32 {
    let channel = |value: u8| f32::from(value) / 255.0;
    0.2126 * channel(pixel[0]) + 0.7152 * channel(pixel[1]) + 0.0722 * channel(pixel[2])
}

/// Computes the unpadded row length, the aligned row length, and the readback buffer size.
fn capture_layout(width: u32, height: u32) -> Result<(u32, u32, u64), RenderError> {
    if width == 0 || height == 0 {
        return Err(RenderError::EmptyCapture);
    }
    if width > MAX_CAPTURE_DIMENSION || height > MAX_CAPTURE_DIMENSION {
        return Err(RenderError::CaptureTooLarge);
    }
    let unpadded_row = width
        .checked_mul(BYTES_PER_PIXEL)
        .ok_or(RenderError::CaptureTooLarge)?;
    let padded_row = unpadded_row
        .checked_add(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT - 1)
        .map(|value| value / wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        .and_then(|value| value.checked_mul(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT))
        .ok_or(RenderError::CaptureTooLarge)?;
    let buffer_size = u64::from(padded_row)
        .checked_mul(u64::from(height))
        .ok_or(RenderError::CaptureTooLarge)?;
    if buffer_size > MAX_CAPTURE_BUFFER_BYTES {
        return Err(RenderError::CaptureTooLarge);
    }
    Ok((unpadded_row, padded_row, buffer_size))
}

#[cfg(test)]
mod tests {
    use super::capture_layout;
    use crate::RenderError;

    #[test]
    fn pads_rows_to_the_required_alignment() {
        // 64 pixels is 256 bytes, already aligned; 65 must round up to 512.
        let (unpadded, padded, size) = capture_layout(64, 8).expect("layout");
        assert_eq!(unpadded, 256);
        assert_eq!(padded, 256);
        assert_eq!(size, 2_048);

        let (unpadded, padded, size) = capture_layout(65, 8).expect("layout");
        assert_eq!(unpadded, 260);
        assert_eq!(padded, 512, "must round up to the copy alignment");
        assert_eq!(size, 4_096);
    }

    #[test]
    fn refuses_an_empty_or_oversized_capture() {
        assert!(matches!(
            capture_layout(0, 8),
            Err(RenderError::EmptyCapture)
        ));
        assert!(matches!(
            capture_layout(8, 0),
            Err(RenderError::EmptyCapture)
        ));
        assert!(matches!(
            capture_layout(100_000, 8),
            Err(RenderError::CaptureTooLarge)
        ));
    }
}
