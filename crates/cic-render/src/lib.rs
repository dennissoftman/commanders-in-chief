//! Terrain rendering, GPU-independent bookkeeping, and the WGSL shader set.
//!
//! # What is here
//!
//! - **A forward terrain pass** ([`terrain`]), which renders a [`cic_assets::Terrain`] with heights
//!   and layer weights held in *writable* GPU textures rather than a baked mesh. See that module for
//!   why that choice is load-bearing rather than incidental.
//! - **Headless rendering and capture** ([`gpu`]). Headless comes before any window, because a
//!   capture is the only rendering verification that runs in CI.
//! - **View and projection** ([`view`]), kept out of `cic-camera` because a projection depends on
//!   the viewport and the API's clip-space convention.
//! - **The WGSL shader set.** Every shader is parsed and validated at test time by the same front
//!   end the GPU backend uses. A shader is code that `cargo build` does not compile, so without this
//!   a copy error or a syntax regression would produce a clean build and a blank frame.
//! - **Virtual-page residency bookkeeping** ([`terrain_virtual`]), which decides which terrain pages
//!   to stage and evict for a given view. That is arithmetic, and the subtle bugs live in it, so it
//!   is kept separate from device calls.
//! - **Texture resources** ([`resource`]), which deduplicate decoded images by content hash under
//!   explicit byte budgets.
//!
//! # What is next
//!
//! The deferred chain — G-buffer, cascaded shadows, ambient occlusion — plus models, water, and
//! windowed presentation. The shaders for most of that are already here and validated; what they
//! need is the pipeline scaffolding around them. See `docs/milestones/m3-renderer.md`.

pub mod detail;
pub mod gpu;
pub mod resource;
pub mod scene;
pub mod terrain;
pub mod terrain_virtual;
pub mod view;

use std::error::Error;
use std::fmt::{self, Display, Formatter};

pub use gpu::{Capture, CaptureTarget, GpuContext};
pub use resource::{TextureId, TextureResourceManager};
pub use scene::{TerrainFrame, capture_terrain, render_terrain_into};
pub use terrain::{DirectionalLight, LayerColour, TerrainRenderer};
pub use view::{Projection, view_projection};

/// Every WGSL shader in the set, as `(name, source)`.
///
/// Compiled in rather than loaded from disk: a shader is code, and shipping it as a loose file
/// invites a mismatch between the binary and the file next to it.
pub const SHADERS: &[(&str, &str)] = &[
    ("boundary_viewer", include_str!("boundary_viewer.wgsl")),
    ("model", include_str!("model.wgsl")),
    ("road_viewer", include_str!("road_viewer.wgsl")),
    ("scene_shadow", include_str!("scene_shadow.wgsl")),
    ("shader", include_str!("shader.wgsl")),
    ("terrain", include_str!("terrain.wgsl")),
    ("terrain_ao", include_str!("terrain_ao.wgsl")),
    ("terrain_deferred", include_str!("terrain_deferred.wgsl")),
    ("terrain_shadow", include_str!("terrain_shadow.wgsl")),
    ("terrain_viewer", include_str!("terrain_viewer.wgsl")),
    ("terrain_virtual", include_str!("terrain_virtual.wgsl")),
    ("ui", include_str!("ui.wgsl")),
    ("viewer", include_str!("viewer.wgsl")),
];

/// Returns one shader's source by name.
#[must_use]
pub fn shader(name: &str) -> Option<&'static str> {
    SHADERS
        .iter()
        .find_map(|(candidate, source)| (*candidate == name).then_some(*source))
}

/// A failure in a renderer operation.
#[derive(Debug)]
pub enum RenderError {
    /// A texture's dimensions, byte length, or declared size were inconsistent or out of range.
    InvalidTexture,
    /// A texture, or the texture set as a whole, exceeded its explicit byte budget.
    TextureTooLarge,
    /// No adapter, native or fallback, could be acquired.
    RequestAdapter(wgpu::RequestAdapterError),
    /// An adapter was found but no device could be created from it.
    RequestDevice(wgpu::RequestDeviceError),
    /// Waiting for submitted work to complete failed.
    Poll(wgpu::PollError),
    /// Mapping the readback buffer failed.
    MapBuffer(wgpu::BufferAsyncError),
    /// Taking a mapped range of the readback buffer failed.
    MapRange(wgpu::MapRangeError),
    /// The map callback did not fire within its timeout.
    MapCallbackTimeout,
    /// A capture was requested with a zero width or height.
    EmptyCapture,
    /// A capture's dimensions or buffer size exceeded the renderer's explicit bounds.
    CaptureTooLarge,
    /// Encoding a capture as a PNG failed.
    EncodePng(String),
    /// A terrain declared more layers than the forward pass blends.
    TooManyLayers {
        /// Layers the terrain declared.
        actual: usize,
        /// Layers the pass supports.
        maximum: usize,
    },
    /// A layer index was outside the terrain's layer set.
    LayerOutOfRange {
        /// The requested layer.
        layer: u32,
        /// Layers the terrain declares.
        layers: u32,
    },
    /// A write region left the terrain, was empty, or disagreed with the supplied data length.
    RegionOutOfRange {
        /// Requested origin.
        origin: [u32; 2],
        /// Requested size.
        size: [u32; 2],
        /// The terrain's sample dimensions.
        terrain: [u32; 2],
    },
}

impl Display for RenderError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTexture => {
                formatter.write_str("texture dimensions or byte length are invalid")
            }
            Self::TextureTooLarge => {
                formatter.write_str("texture exceeds its explicit byte budget")
            }
            Self::RequestAdapter(error) => write!(formatter, "no usable adapter: {error}"),
            Self::RequestDevice(error) => write!(formatter, "no usable device: {error}"),
            Self::Poll(error) => write!(formatter, "waiting for the queue failed: {error}"),
            Self::MapBuffer(error) => write!(formatter, "mapping the readback failed: {error}"),
            Self::MapRange(error) => {
                write!(formatter, "taking the mapped range failed: {error}")
            }
            Self::MapCallbackTimeout => {
                formatter.write_str("the buffer map callback did not fire in time")
            }
            Self::EmptyCapture => formatter.write_str("a capture cannot be zero-sized"),
            Self::CaptureTooLarge => {
                formatter.write_str("capture size exceeds the renderer's explicit bounds")
            }
            Self::EncodePng(message) => write!(formatter, "encoding a PNG failed: {message}"),
            Self::TooManyLayers { actual, maximum } => write!(
                formatter,
                "terrain declares {actual} layers, but the forward pass blends at most {maximum}"
            ),
            Self::LayerOutOfRange { layer, layers } => write!(
                formatter,
                "layer {layer} is outside the {layers} the terrain declares"
            ),
            Self::RegionOutOfRange {
                origin,
                size,
                terrain,
            } => write!(
                formatter,
                "region {size:?} at {origin:?} does not fit a {terrain:?} terrain, \
                 or its data length disagrees"
            ),
        }
    }
}

impl Error for RenderError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::RequestAdapter(error) => Some(error),
            Self::RequestDevice(error) => Some(error),
            Self::Poll(error) => Some(error),
            Self::MapBuffer(error) => Some(error),
            Self::MapRange(error) => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod shader_tests {
    use super::{SHADERS, shader};

    #[test]
    fn every_shader_parses_and_validates() {
        // The point of this test: the shader set is the most valuable thing carried across, and a
        // silent copy error would otherwise surface as a blank frame much later. `naga` is the same
        // WGSL front end `wgpu` uses, so passing here means the shader compiles for real.
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        for (name, source) in SHADERS {
            let module = naga::front::wgsl::parse_str(source)
                .unwrap_or_else(|error| panic!("{name}.wgsl failed to parse: {error:?}"));
            validator
                .validate(&module)
                .unwrap_or_else(|error| panic!("{name}.wgsl failed to validate: {error:?}"));
        }
    }

    #[test]
    fn the_shader_set_is_complete_and_addressable() {
        assert_eq!(SHADERS.len(), 13, "the seeded, licence-clean shader set");
        for (name, source) in SHADERS {
            assert!(!source.trim().is_empty(), "{name}.wgsl is empty");
            assert_eq!(shader(name), Some(*source));
        }
        assert_eq!(shader("no_such_shader"), None);
    }

    #[test]
    fn no_shader_carries_an_inherited_licence_header() {
        // The licence is an open decision, so no file may assert one. This guards against a
        // header being reintroduced by a copy-paste before that decision is made.
        for (name, source) in SHADERS {
            for marker in ["SPDX", "GPL", "Copyright (C)", "License"] {
                assert!(
                    !source.contains(marker),
                    "{name}.wgsl still mentions {marker}"
                );
            }
        }
    }
}
