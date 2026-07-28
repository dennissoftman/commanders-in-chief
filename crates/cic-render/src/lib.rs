//! Terrain rendering, GPU-independent bookkeeping, and the WGSL shader set.
//!
//! # What is here
//!
//! - **The deferred chain** ([`deferred`]): four depth-only shadow cascades, a G-buffer, ambient
//!   occlusion with a bilateral blur, lighting that reconstructs world position from depth, a
//!   tone-mapping composite, and an optional antialias resolve.
//! - **Per-pass GPU timing** ([`timing`]), because every performance question here is
//!   workload-dependent and none can be settled by argument. A total says something is slow; a
//!   breakdown says which pass. Optional, since `TIMESTAMP_QUERY` is.
//! - **Display settings** ([`display`]): the resolution the chain renders at and how it resolves.
//!   Multisampling is declined rather than pending — see
//!   [ADR 0005](../../../docs/adr/0005-antialiasing-strategy.md) — so a resolution scale is the primary
//!   control, a post pass is the floor beneath it, and a temporal accumulation is the tier above. This
//!   module also owns the jitter sequence, because a sub-pixel sample position is arithmetic and the
//!   subtle failures live in arithmetic.
//! - **Terrain** ([`terrain`]), rendered from a [`cic_assets::Terrain`] with heights and layer weights
//!   held in *writable* GPU textures rather than a baked mesh, and per-layer albedo tiled in world
//!   space. See that module for why the writable-texture choice is load-bearing rather than incidental.
//! - **Models** ([`model`]), instanced, with per-instance transform, tint and sway, and one draw call per
//!   model per material path however many materials it has. Base colour, normal and metallic-roughness
//!   maps, and a second index range for materials that cut their own silhouette — which is how foliage is
//!   authored, and which has to reach every shadow cascade rather than only the lit frame.
//! - **Scenery sway** ([`scenery`]), the wind model a plant's vertices are displaced by. Written from
//!   scratch with every constant derived in the file; see [LICENSING.md](../../../LICENSING.md) for why
//!   that is stated rather than assumed.
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
//! - **The WGSL shader set** ([`shader`]), assembled from composable chunks because WGSL has no include
//!   mechanism — without composition every pass needing the cascade selection had to share one file with
//!   it, which is how a single shader reached 620 lines. Every composed program is parsed and validated
//!   at test time by the same front end the GPU backend uses. A shader is code that `cargo build` does
//!   not compile, so without this a copy error or a syntax regression would produce a clean build and a
//!   blank frame.
//! - **Virtual-page residency bookkeeping** ([`terrain_virtual`]), which decides which terrain pages
//!   to stage and evict for a given view. That is arithmetic, and the subtle bugs live in it, so it
//!   is kept separate from device calls — and [`terrain_page`] is the device side that consumes it: the
//!   physical pages, the tables that index them, and the compute pass that composes the layer blend once
//!   per page instead of once per fragment per frame.
//! - **Texture resources** ([`resource`]), which deduplicate decoded images by content hash under
//!   explicit byte budgets.
//!
//! # What is next
//!
//! Mip chains for the virtual-texture pages, without which a page aliases at a shallow angle where the
//! direct blend does not; and terrain level of detail. See `docs/milestones/m3-renderer.md`.

pub mod culling;
pub mod deferred;
pub mod detail;
pub mod display;
pub mod environment;
pub mod gpu;
pub mod model;
pub mod presentation;
pub mod regression;
pub mod resource;
pub mod scene;
pub mod scenery;
pub mod shader;
pub mod shadow;
pub mod terrain;
pub mod terrain_page;
pub mod terrain_virtual;
pub mod texture;
pub mod timing;
pub mod view;
pub mod water;

use std::error::Error;
use std::fmt::{self, Display, Formatter};

pub use culling::{CHUNK_CELLS, ChunkGrid, Frustum};
pub use deferred::{DeferredFrame, DeferredRenderer, DeferredTargets, occlusion_size};
pub use display::{Antialiasing, DisplaySettings};
pub use environment::{Clouds, Environment, Fog, Weather};
pub use gpu::{Capture, CaptureTarget, GpuContext};
pub use model::{ModelBatch, ModelInstance};
pub use presentation::{Action, InputState, SurfaceRenderer, TerrainGround};
pub use regression::{Comparison, Tolerance};
pub use resource::{TextureId, TextureResourceManager};
pub use scene::{TerrainFrame, capture_terrain, render_terrain_into};
pub use scenery::{SwayProfile, sway_phase};
pub use shader::{PROGRAMS, Program, compose};
pub use shadow::{CASCADE_COUNT, Cascade, fit_cascades};
pub use terrain::{Animation, DirectionalLight, LayerColour, LayerMaterial, TerrainRenderer};
pub use terrain_page::TerrainPageCache;
pub use texture::{ColourSpace, TextureArray, TextureImage};
pub use timing::{FrameTimings, PassTimer, TimedPass};
pub use view::{Projection, view_projection};
pub use water::{WaterBody, WaterMaterial, WaterSurface};

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
