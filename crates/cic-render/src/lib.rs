//! Terrain rendering, GPU-independent bookkeeping, and the WGSL shader set.
//!
//! # What is here
//!
//! - **The deferred chain** ([`deferred`]): four depth-only shadow cascades, a G-buffer, ambient
//!   occlusion with a bilateral blur, lighting that reconstructs world position from depth, and a
//!   tone-mapping composite.
//! - **Terrain** ([`terrain`]), rendered from a [`cic_assets::Terrain`] with heights and layer weights
//!   held in *writable* GPU textures rather than a baked mesh, and per-layer albedo tiled in world
//!   space. See that module for why the writable-texture choice is load-bearing rather than incidental.
//! - **Models** ([`model`]), instanced, with per-instance transform and tint and one draw call per
//!   model however many materials it has.
//! - **Water** ([`water`]), a bounded plane with procedural waves, blended into the scene before tone
//!   mapping. Its shoreline comes from the depth buffer rather than from an authored outline, so a
//!   rectangle plus a heightfield already produce an arbitrarily shaped shore.
//! - **Colour texture arrays** ([`texture`]), which resample to a common slice size and generate their
//!   mip chain on the CPU in linear light. Both terrain layers and model materials index into one.
//! - **Headless rendering and capture** ([`gpu`]). Headless comes before any window, because a
//!   capture is the only rendering verification that runs in CI.
//! - **Windowed presentation** ([`presentation`]): the same chain pointed at a swapchain, plus an
//!   input-to-intent mapping that is testable without a window.
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
//! Antialiasing, a real virtual-texture cache behind the residency bookkeeping, and the committed
//! reference captures that close the milestone. See `docs/milestones/m3-renderer.md`.

pub mod deferred;
pub mod detail;
pub mod gpu;
pub mod model;
pub mod presentation;
pub mod regression;
pub mod resource;
pub mod scene;
pub mod shadow;
pub mod terrain;
pub mod terrain_virtual;
pub mod texture;
pub mod view;
pub mod water;

use std::error::Error;
use std::fmt::{self, Display, Formatter};

pub use deferred::{DeferredFrame, DeferredRenderer, DeferredTargets};
pub use gpu::{Capture, CaptureTarget, GpuContext};
pub use model::{ModelBatch, ModelInstance};
pub use presentation::{Action, InputState, SurfaceRenderer, TerrainGround};
pub use regression::{Comparison, Tolerance};
pub use resource::{TextureId, TextureResourceManager};
pub use scene::{TerrainFrame, capture_terrain, render_terrain_into};
pub use shadow::{CASCADE_COUNT, Cascade, fit_cascades};
pub use terrain::{DirectionalLight, LayerColour, LayerMaterial, TerrainRenderer};
pub use texture::{TextureArray, TextureImage};
pub use view::{Projection, view_projection};
pub use water::{WaterBody, WaterMaterial, WaterSurface};

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
    ("terrain_forward", include_str!("terrain_forward.wgsl")),
    ("model_gbuffer", include_str!("model_gbuffer.wgsl")),
    ("terrain_gbuffer", include_str!("terrain_gbuffer.wgsl")),
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
    /// The camera's view-projection could not be inverted, so no world position could be
    /// reconstructed from depth.
    SingularCamera,
    /// A window could not provide a presentable surface.
    CreateSurface(String),
    /// Acquiring the next surface frame failed for a reason a redraw will not fix.
    SurfaceLost(String),
    /// The surface reported no format this renderer can present to.
    NoSurfaceFormat,
    /// A water body's rectangle was empty or inverted, or one of its material figures was
    /// non-finite, non-positive, or outside its range.
    InvalidWater,
    /// A model had no geometry to upload.
    EmptyModel,
    /// A model vertex, index, or instance count exceeded the addressable range.
    ModelTooLarge,
    /// A capture's dimensions or buffer size exceeded the renderer's explicit bounds.
    CaptureTooLarge,
    /// Encoding a capture as a PNG failed.
    EncodePng(String),
    /// A reference image was not a readable 8-bit RGBA PNG.
    DecodePng(String),
    /// A reference image was rendered at a different size than the capture compared against it.
    ReferenceSizeMismatch {
        /// The reference's dimensions.
        reference: [u32; 2],
        /// The capture's dimensions.
        capture: [u32; 2],
    },
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
            Self::SingularCamera => {
                formatter.write_str("the camera view-projection is singular and cannot be inverted")
            }
            Self::CreateSurface(message) => {
                write!(formatter, "could not create a surface: {message}")
            }
            Self::SurfaceLost(message) => {
                write!(formatter, "the surface was lost: {message}")
            }
            Self::NoSurfaceFormat => {
                formatter.write_str("the surface offers no format this renderer can present to")
            }
            Self::InvalidWater => formatter
                .write_str("the water body's rectangle or one of its material figures is invalid"),
            Self::EmptyModel => formatter.write_str("the model has no geometry"),
            Self::ModelTooLarge => {
                formatter.write_str("the model exceeds the addressable vertex range")
            }
            Self::CaptureTooLarge => {
                formatter.write_str("capture size exceeds the renderer's explicit bounds")
            }
            Self::EncodePng(message) => write!(formatter, "encoding a PNG failed: {message}"),
            Self::DecodePng(message) => {
                write!(formatter, "decoding a reference PNG failed: {message}")
            }
            Self::ReferenceSizeMismatch { reference, capture } => write!(
                formatter,
                "the reference is {reference:?} but the capture is {capture:?}, \
                 so the two cannot be compared"
            ),
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
        assert_eq!(
            SHADERS.len(),
            16,
            "13 seeded shaders plus the forward, terrain G-buffer, and model passes"
        );
        for (name, source) in SHADERS {
            assert!(!source.trim().is_empty(), "{name}.wgsl is empty");
            assert_eq!(shader(name), Some(*source));
        }
        assert_eq!(shader("no_such_shader"), None);
    }

    #[test]
    fn no_shader_carries_an_inherited_licence_header() {
        // The licence is declared once, in the workspace manifest, so no shader asserts one of
        // its own. That makes any licence header in this directory an *inherited* one, which is
        // exactly the copy-paste this guards against: the predecessor's shaders carried GPL
        // headers, and a pasted region would bring the obligation back with it. See LICENSING.md.
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
