//! The deferred chain: shadow cascades, G-buffer, ambient occlusion, lighting, composite.
//!
//! # Why deferred at all
//!
//! Both the shadow and the occlusion term are screen-space: each needs the whole depth buffer
//! resolved before any pixel can be lit. A forward pass cannot provide that, which is the entire
//! reason for the split — not a general preference for deferred rendering.
//!
//! # Pass order
//!
//! ```text
//! 1. shadow    x4   depth-only, terrain from each cascade's light view
//! 2. gbuffer        albedo, world normal + roughness, coverage, scene depth
//! 3. ao             occlusion from depth and normals
//! 4. ao blur        bilateral, so occlusion does not bleed across creases
//! 5. lighting       reconstruct world position from depth, apply light with shadow and AO -> HDR
//! 6. water          procedural waves, blended into the HDR target
//! 7. composite      tone map, downsample and sharpen -> the caller's target
//! 8. antialias      a luma edge-directed blend, or a temporal resolve, when enabled
//! ```
//!
//! Water sits *inside* the HDR target rather than being composited over the finished image, so its
//! glitter tone maps with everything else. Over the composite it would clip to white instead of
//! rolling off, which is the whole reason the accumulation target has range above one.
//!
//! # Two sizes, not one
//!
//! Passes 1 to 6 run at the *render* resolution, which is the output resolution times
//! [`DisplaySettings::resolution_scale`]. Passes 7 and 8 run at the output resolution; the composite's
//! filtered read of the HDR target is the downsample between them. Both sizes reach the shaders in the
//! scene uniform — `viewport` and `output` — and both are taken from [`DeferredTargets`], which is the
//! one place they are decided.
//!
//! Pass 8 exists only when antialiasing is enabled, and when it does the composite writes into an LDR
//! intermediate for it to read instead of into the caller's target. With antialiasing off there is no
//! intermediate and no extra pass, so the default path costs exactly what it did before either setting
//! existed.
//!
//! # The motion target, which is written whatever pass 8 does
//!
//! The G-buffer carries a fourth attachment: the texture-coordinate offset from each fragment to where the
//! same surface point sat in the previous frame. Only the temporal resolve reads it, and it is written
//! unconditionally — because the alternative is a second G-buffer pipeline per geometry kind, differing from
//! the first in one attachment, and four pipelines to keep in step rather than two. Two bytes a channel over
//! two channels is 4.6 MB at 1920x1200 and one more write per fragment, which the per-pass timer makes
//! visible rather than a matter of opinion.
//!
//! Nothing but the temporal resolve reads it, so with that resolve off the frame is unchanged — which is
//! what keeps every committed reference byte-identical across this addition.
//!
//! The G-buffer stores no world position. It is reconstructed from depth in step 5, for the reason
//! documented at `world_from_depth` in `scene.wgsl`: a half-float position target
//! quantises to whole world units past 1024, which striped self-shadowing across the whole terrain in
//! a way no bias setting could reach.

use cic_camera::CameraPose;

use std::cell::RefCell;
use std::ops::Range;

use crate::RenderError;
use crate::culling::{Frustum, contiguous_runs};
use crate::display::{DisplaySettings, jitter_offset};
use crate::environment::Environment;
use crate::gpu::{DEPTH_FORMAT, GpuContext};
use crate::model::{ModelBatch, buffer_layouts};
use crate::shadow::{CASCADE_COUNT, CASCADE_RESOLUTION, Cascade, fit_cascades};
use crate::terrain::{Animation, DirectionalLight, Motion, TerrainRenderer};
use crate::timing::{FrameTimings, PassTimer, TimedPass};
use crate::view::{Projection, invert, look_at, multiply, perspective};
use crate::water::WaterBody;

/// Albedo target format. sRGB because albedo is authored in sRGB and the conversion is free.
pub const ALBEDO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// World normal in `xyz` and roughness in `w`.
///
/// Half float rather than `Rgba8Unorm`: an 8-bit normal quantises to about half a degree, visible as
/// banding across the smooth gradients terrain is mostly made of.
pub const NORMAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Geometry coverage, with values above one carrying emissive strength — hence a float format rather
/// than unorm, which would clamp exactly the range that encodes emission.
pub const COVERAGE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;

/// Ambient occlusion, a single unsigned channel.
pub const AO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

/// Texture-coordinate motion since the previous frame, two signed channels.
///
/// Half float rather than `Rg8Snorm`: the values are *fractions of the screen*, so a pixel moving one pixel
/// at 1920 wide has a motion of 0.0005 — which eight bits over a range of `-1..=1` quantises to zero. The
/// slow, sub-pixel motion a temporal resolve exists to accumulate is exactly the range an integer format
/// cannot represent.
pub const MOTION_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;

/// The accumulated temporal history, at output size.
///
/// Not the output format, and the reason is precision rather than range. The history is a running average of
/// eight-bit inputs, so its *values* fit in eight bits but its increments do not: at a history weight of 0.9
/// each frame contributes a tenth of a step, which an eight-bit store rounds to either zero or one step and
/// so either freezes the accumulation or oscillates around it. A float history stores the average of the
/// samples rather than the nearest representable neighbour of it.
pub const HISTORY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Byte size of the temporal resolve's own uniform block: one vec4.
const TEMPORAL_UNIFORM_BYTES: usize = 16;

/// The size the occlusion *estimate* is computed at, for a given render size.
///
/// Half of it on each axis, rounding **up** so an odd render size keeps its last column and row rather
/// than dropping them. `terrain_ao.wgsl` derives the same figure with the same rounding to clamp its
/// upsample taps, and a test pins the pair — the two cannot be reconciled through the uniform, because
/// that shader reads the scene block through a deliberately truncated prefix of it.
///
/// Half rather than full because measurement said so: the estimate was 58% of a 1920x1200 frame and its
/// blur another 14%, which made it both the most expensive pass by a wide margin and the one a resolution
/// scale multiplies hardest. Occlusion is a low-frequency signal over a surface, so the estimate is what
/// is cheap to halve; the bilateral pass that resolves it back to full resolution is what keeps the
/// silhouettes.
#[must_use]
pub const fn occlusion_size(render_width: u32, render_height: u32) -> (u32, u32) {
    // `Ord::max` is not const on the pinned toolchain, hence the explicit floor of one.
    const fn halve(size: u32) -> u32 {
        let halved = size.div_ceil(2);
        if halved == 0 { 1 } else { halved }
    }
    (halve(render_width), halve(render_height))
}

/// Lighting accumulates before tone mapping, so it needs range above one.
pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Byte size of the `SceneCamera` uniform block: two matrices, two vectors, three lights, the five
/// atmosphere vectors — fog colour and density, fog falloff, cloud parameters, cloud drift, and the
/// surface weather the lighting pass applies to the G-buffer — and the output size the composite
/// resolves to.
const SCENE_UNIFORM_BYTES: usize = 64 + 64 + 16 + 16 + 3 * 48 + 5 * 16 + 16;

/// Byte size of the `ShadowCamera` uniform block: a matrix and a parameter vector per cascade.
const SHADOW_UNIFORM_BYTES: usize = CASCADE_COUNT * (64 + 16);

/// Byte size of one cascade's view uniform.
const CASCADE_UNIFORM_BYTES: usize = 64;

/// How far from the camera shadows are cast, in world units.
///
/// Separate from the projection's far plane on purpose: the far plane may be kilometres out for
/// distant terrain, while shadows only need to reach as far as they read. Stretching cascades to the
/// far plane would spend all their resolution on pixels where a shadow is a few texels wide.
pub const DEFAULT_SHADOW_DISTANCE: f32 = 1_600.0;

/// Everything one deferred frame needs.
///
/// # Why there is no viewport here
///
/// There was, and it was a hazard. The size the shaders reconstruct world positions against is a
/// property of the *targets* — they are the textures being loaded from — so taking it from the frame
/// meant two sources for one number with nothing checking they agreed. A frame one resize behind the
/// surface then moved every receiver and read as a shadowing fault rather than as a wrong figure, which
/// is why [`crate::presentation::SurfaceRenderer::render`] used to overwrite the caller's value.
///
/// [`DeferredRenderer`] now takes both the render and the output size from the [`DeferredTargets`] it
/// was built against, so the disagreement is not expressible. A size still reaches
/// [`DeferredFrame::new`], because a projection needs an aspect ratio — but that is all it is used for,
/// and a wrong one distorts the image rather than silently relocating the scene.
#[derive(Debug, Clone, Copy)]
pub struct DeferredFrame {
    /// Where the camera is.
    pub pose: CameraPose,
    /// How the viewport projects.
    pub projection: Projection,
    /// The primary light. It is the only shadowed one.
    pub light: DirectionalLight,
    /// How far shadows are cast.
    pub shadow_distance: f32,
    /// The air and the weather: fog, cloud shadows, and what the time of day implies.
    ///
    /// Defaults to a clear, fogless, cloudless environment, which is exactly the frame this renderer
    /// produced before one existed — that is what let every committed reference capture stay
    /// byte-identical when the atmosphere was added, and so what proved the plumbing had not quietly
    /// altered the lighting passing through it.
    pub environment: Environment,
    /// Scene time in seconds, which drives every animated surface.
    ///
    /// A frame *parameter* rather than a clock reading taken inside the renderer, and that is a
    /// deliberate constraint rather than a convenience. A capture of an animated scene is only a
    /// usable regression reference if the same inputs produce the same image, so the one thing that
    /// would make it irreproducible — the wall clock — is kept out of the renderer entirely. The
    /// caller running a window advances this; a test pins it.
    pub time: f32,
    /// Which sub-pixel jitter phase this frame is rendered at.
    ///
    /// A frame parameter for the same reason [`Self::time`] is one, and it is the reason a temporal capture
    /// is reproducible at all: the renderer holds no counter, so a test that renders phases 0 to 7 in order
    /// produces the same eight frames every run and on every machine. A caller running a window passes its
    /// own frame ordinal, which [`crate::display::jitter_offset`] wraps into the cycle.
    ///
    /// Ignored unless the display settings ask for a temporal resolve. Without one the projection is
    /// unjittered whatever this says, which is what keeps every committed reference valid.
    pub jitter: u32,
}

impl DeferredFrame {
    /// Builds a frame from a pose and an output size, with the default light and shadow distance, at
    /// time zero.
    ///
    /// The size is the caller's target, not the render resolution: it feeds the projection's aspect
    /// ratio, which a resolution scale does not change.
    #[must_use]
    pub fn new(pose: CameraPose, width: u32, height: u32) -> Self {
        let environment = Environment::default();
        Self {
            pose,
            projection: Projection::for_viewport(width, height),
            // Derived from the environment's hour rather than taken from a preset, so a caller who changes
            // the time of day gets a sun that moves with it instead of one that silently disagrees.
            // `Environment::sun_light` is calibrated against `daylight_with_occlusion`, which this replaces
            // as the default and which a test still pins it to.
            light: environment.sun_light(),
            shadow_distance: DEFAULT_SHADOW_DISTANCE,
            environment,
            time: 0.0,
            jitter: 0,
        }
    }

    /// Returns the frame with its environment replaced, and its light re-derived to match.
    ///
    /// The light comes along deliberately. An environment carrying a 6 a.m. hour beside a light still pointing
    /// where it did at noon is not a configuration anyone wants, and leaving the two independent means every
    /// caller changing the time of day has to remember to update both. A caller wanting them to disagree —
    /// a test pinning a sun angle while varying the weather, say — assigns [`Self::light`] afterwards, which
    /// reads as the deliberate override it is.
    #[must_use]
    pub fn in_environment(mut self, environment: Environment) -> Self {
        self.light = environment.sun_light();
        self.environment = environment;
        self
    }

    /// Returns the frame with its scene time replaced.
    #[must_use]
    pub const fn at_time(mut self, time: f32) -> Self {
        self.time = time;
        self
    }

    /// Returns the frame with its jitter phase replaced.
    #[must_use]
    pub const fn at_jitter(mut self, jitter: u32) -> Self {
        self.jitter = jitter;
        self
    }
}

/// Every render target the chain writes, and the two sizes it writes them at.
///
/// This type is the single source of truth for both. Every screen-space target from the G-buffer to the
/// HDR accumulation is allocated at the *render* size; the LDR intermediate, when there is one, is
/// allocated at the *output* size, because the antialias pass that reads it works on final pixels. A
/// [`DeferredRenderer`] takes its uniform sizes and its output format from here rather than from its own
/// parameters, so a renderer and the targets it draws into cannot disagree about either.
#[derive(Debug)]
pub struct DeferredTargets {
    albedo: wgpu::TextureView,
    normal: wgpu::TextureView,
    coverage: wgpu::TextureView,
    depth: wgpu::TextureView,
    shadow_array: wgpu::TextureView,
    shadow_layers: Vec<wgpu::TextureView>,
    ao_raw: wgpu::TextureView,
    ao_blurred: wgpu::TextureView,
    motion: wgpu::TextureView,
    hdr: wgpu::TextureView,
    /// The temporal history: one two-layer texture, read as an array and written a layer at a time.
    ///
    /// One texture rather than two, so the resolve needs one bind group with the layer as a uniform instead
    /// of two bind groups that must not drift apart. `None` unless the settings asked for a temporal
    /// resolve, since two float targets at output size is real memory to spend on a pass that will not run.
    history: Option<HistoryTargets>,
    /// The tone-mapped image, at output size and in the output format, when a pass reads it.
    ///
    /// `None` with antialiasing off, and then the composite writes into the caller's target directly.
    /// Allocating it regardless would cost a full-resolution texture for a pass that never runs.
    ldr: Option<wgpu::TextureView>,
    render: [u32; 2],
    output: [u32; 2],
    output_format: wgpu::TextureFormat,
    display: DisplaySettings,
}

impl DeferredTargets {
    /// Allocates every intermediate target for an output size, at the display settings' render scale.
    ///
    /// `output_format` is what the last pass writes into — a capture target, or a surface's own format,
    /// which is commonly BGRA rather than RGBA. It is taken here rather than at
    /// [`DeferredRenderer::new`] because the LDR intermediate has to be allocated in it: an antialias
    /// pass writing to the caller's target has to read something the composite could write.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::EmptyCapture`] for a zero dimension.
    pub fn new(
        context: &GpuContext,
        width: u32,
        height: u32,
        output_format: wgpu::TextureFormat,
        display: DisplaySettings,
    ) -> Result<Self, RenderError> {
        if width == 0 || height == 0 {
            return Err(RenderError::EmptyCapture);
        }
        let (render_width, render_height) = display.render_size(width, height);
        let (occlusion_width, occlusion_height) = occlusion_size(render_width, render_height);
        let device = context.device();
        let sized = |label: &str, format: wgpu::TextureFormat, width: u32, height: u32| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let screen = |label: &str, format: wgpu::TextureFormat| {
            sized(label, format, render_width, render_height)
        };

        let shadow = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cic-render shadow cascades"),
            size: wgpu::Extent3d {
                width: CASCADE_RESOLUTION,
                height: CASCADE_RESOLUTION,
                depth_or_array_layers: u32::try_from(CASCADE_COUNT).unwrap_or(1),
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let shadow_array = shadow.create_view(&wgpu::TextureViewDescriptor {
            label: Some("cic-render shadow array view"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        // One single-layer view per cascade, because a render pass attaches one layer at a time.
        let shadow_layers = (0..CASCADE_COUNT)
            .map(|index| {
                shadow.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("cic-render shadow cascade layer"),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: u32::try_from(index).unwrap_or(0),
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();

        Ok(Self {
            albedo: screen("cic-render gbuffer albedo", ALBEDO_FORMAT),
            normal: screen("cic-render gbuffer normal", NORMAL_FORMAT),
            coverage: screen("cic-render gbuffer coverage", COVERAGE_FORMAT),
            // Both depth-tested against in the G-buffer pass and sampled in the AO and lighting
            // passes, which is why it carries `TEXTURE_BINDING` as well.
            depth: screen("cic-render scene depth", DEPTH_FORMAT),
            shadow_array,
            shadow_layers,
            // The estimate is half resolution and its resolve is full, which is the whole optimisation.
            // See `occlusion_size`.
            ao_raw: sized(
                "cic-render ao raw",
                AO_FORMAT,
                occlusion_width,
                occlusion_height,
            ),
            ao_blurred: screen("cic-render ao blurred", AO_FORMAT),
            motion: screen("cic-render motion", MOTION_FORMAT),
            hdr: screen("cic-render hdr scene", HDR_FORMAT),
            history: display
                .needs_history()
                .then(|| HistoryTargets::new(device, width, height)),
            // At output size, not render size: the composite has already downsampled by the time this
            // is written, so the antialias pass reading it measures edges in pixels the viewer sees.
            ldr: display
                .needs_resolve_target()
                .then(|| sized("cic-render tone mapped", output_format, width, height)),
            render: [render_width, render_height],
            output: [width, height],
            output_format,
            display,
        })
    }

    /// Returns the size every pass from the G-buffer to the HDR accumulation is allocated at.
    #[must_use]
    pub const fn render_size(&self) -> (u32, u32) {
        (self.render[0], self.render[1])
    }

    /// Returns the size the occlusion estimate is computed at. See [`occlusion_size`].
    #[must_use]
    pub const fn occlusion_size(&self) -> (u32, u32) {
        occlusion_size(self.render[0], self.render[1])
    }

    /// Returns the size of the caller's target, which the composite resolves to.
    #[must_use]
    pub const fn output_size(&self) -> (u32, u32) {
        (self.output[0], self.output[1])
    }

    /// Returns the format the last pass writes.
    #[must_use]
    pub const fn output_format(&self) -> wgpu::TextureFormat {
        self.output_format
    }

    /// Returns the settings these targets were allocated for.
    #[must_use]
    pub const fn display(&self) -> DisplaySettings {
        self.display
    }
}

/// The two temporal history layers.
///
/// One texture of two layers rather than two textures, so the pair cannot drift apart in size or format —
/// they are one allocation with one descriptor. Each layer has its own single-layer view, which serves as
/// both the attachment it is written through and the sampled resource it is read through, in alternate
/// frames.
#[derive(Debug)]
struct HistoryTargets {
    layers: [wgpu::TextureView; 2],
}

impl HistoryTargets {
    fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cic-render temporal history"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 2,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HISTORY_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let layer = |index: u32| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("cic-render temporal history layer"),
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: index,
                array_layer_count: Some(1),
                ..Default::default()
            })
        };
        Self {
            layers: [layer(0), layer(1)],
        }
    }
}

/// The deferred pipelines and their uniforms.
#[derive(Debug)]
pub struct DeferredRenderer {
    shadow_pipeline: wgpu::RenderPipeline,
    gbuffer_pipeline: wgpu::RenderPipeline,
    model_shadow_pipeline: wgpu::RenderPipeline,
    model_gbuffer_pipeline: wgpu::RenderPipeline,
    /// The alpha-tested pair. Built unconditionally rather than on demand, because a batch can gain a
    /// masked material through [`ModelBatch::set_instances`]-style reuse and a pipeline created mid-frame
    /// would stall; two pipelines are cheap, and a scene with no foliage simply never records them.
    model_masked_shadow_pipeline: wgpu::RenderPipeline,
    model_masked_gbuffer_pipeline: wgpu::RenderPipeline,
    material_layout: wgpu::BindGroupLayout,
    water_pipeline: wgpu::RenderPipeline,
    water_layout: wgpu::BindGroupLayout,
    ao: AoStage,
    lighting: LightingStage,
    /// The post-pass antialias resolve, present only when the settings asked for that one.
    antialias: Option<AntialiasStage>,
    /// The temporal resolve, present only when the settings asked for that one.
    temporal: Option<TemporalStage>,
    scene_uniform: wgpu::Buffer,
    /// The temporal resolve's own block. Its own rather than three more fields on the scene block, because
    /// two of the three change *during* a frame's recording rather than before it — which the scene block,
    /// written once in `set_frame` and read by six passes, has no room to express.
    temporal_uniform: wgpu::Buffer,
    shadow_uniform: wgpu::Buffer,
    cascade_uniforms: Vec<wgpu::Buffer>,
    cascade_groups: Vec<wgpu::BindGroup>,
    /// Per-pass GPU timing, present only when it has been asked for and the device supports it.
    ///
    /// Held here rather than passed in per frame so [`Self::render`] can stay `&self`: a timestamp write
    /// mutates the query set on the device, not this struct.
    timer: Option<PassTimer>,
    /// Copied from the targets this was built against. See [`DeferredFrame`] for why the frame does not
    /// carry them.
    render: [u32; 2],
    output: [u32; 2],
    /// Which terrain chunks each pass draws, decided by [`Self::set_frame`] and read by [`Self::render`].
    ///
    /// In a cell so that both can keep taking `&self`. The alternative was threading the visible set
    /// through `render` as a further argument, and it is not the caller's business: it is derived from the
    /// frame the caller already supplied, and a caller free to pass a *different* set than the one
    /// `set_frame` computed could cull geometry the cascades were fitted around.
    visible: RefCell<VisibleChunks>,
    /// What the previous frame looked like, for the motion target and the history ping-pong.
    ///
    /// In a cell for the same reason the visible set is: this is derived from frames the caller already
    /// supplied, and it is the renderer's own bookkeeping rather than the caller's. Making the caller carry
    /// it would make a temporal resolve silently wrong for anyone who forgot to thread it through, which is
    /// the worst available failure mode — a frame that renders and is subtly stale.
    previous: RefCell<PreviousFrame>,
}

/// The state one frame of a temporal resolve inherits from the frame before it.
#[derive(Debug)]
struct PreviousFrame {
    /// The unjittered view-projection the previous frame was rendered with.
    view_projection: [[f32; 4]; 4],
    /// The previous frame's scene time, so a swaying vertex can be evaluated where it was.
    time: f32,
    /// Which history layer holds the accumulation. The resolve reads this one and writes the other.
    history: usize,
    /// Whether there is an accumulation at all.
    ///
    /// False until the first frame has been recorded, and reset by a rebuild — which is what a resize does,
    /// and a resize is precisely when the history describes an image of the wrong size.
    accumulated: bool,
}

impl Default for PreviousFrame {
    fn default() -> Self {
        Self {
            view_projection: [[0.0; 4]; 4],
            time: 0.0,
            history: 0,
            accumulated: false,
        }
    }
}

/// The chunk runs each terrain pass draws, for one frame.
///
/// One entry per shadow cascade plus one for the camera. Kept as runs rather than as indices because that
/// is the form the draw call takes, and converting once per frame beats converting once per pass.
#[derive(Debug, Default)]
struct VisibleChunks {
    cascades: [Vec<Range<u32>>; CASCADE_COUNT],
    camera: Vec<Range<u32>>,
    /// Scratch for the index list, reused across the five culls rather than reallocated per pass.
    scratch: Vec<u32>,
}

impl DeferredRenderer {
    /// Builds every pipeline and bind group for one terrain and target set.
    ///
    /// The output format comes from `targets` rather than from a parameter of its own. A pipeline built
    /// for the wrong format fails at creation rather than rendering something subtly wrong — which was
    /// the argument for passing it explicitly — but the LDR intermediate has to be *allocated* in that
    /// format too, so the two were bound to agree and nothing checked that they did.
    ///
    /// # Errors
    ///
    /// Currently infallible, but returns `Result` so adding a fallible resource later is not a
    /// breaking change.
    pub fn new(
        context: &GpuContext,
        terrain: &TerrainRenderer,
        targets: &DeferredTargets,
    ) -> Result<Self, RenderError> {
        let output_format = targets.output_format;
        let device = context.device();
        let scene_uniform = uniform_buffer(device, "cic-render scene camera", SCENE_UNIFORM_BYTES);
        let shadow_uniform =
            uniform_buffer(device, "cic-render shadow camera", SHADOW_UNIFORM_BYTES);

        // Composed rather than read from one file each. `lighting`, `composite` and `water` all need the
        // scene bindings and the cascade selection, and before composition they had to share a file to
        // reach them. See `crate::shader`.
        let gbuffer_shader = shader_module(device, "terrain_gbuffer");
        let model_shader = shader_module(device, "model_gbuffer");
        let lighting_shader = shader_module(device, "lighting");
        let composite_shader = shader_module(device, "composite");
        let water_shader = shader_module(device, "water");
        let ao_shader = shader_module(device, "terrain_ao");
        let temporal_uniform =
            uniform_buffer(device, "cic-render temporal", TEMPORAL_UNIFORM_BYTES);

        let (cascade_layout, cascade_uniforms, cascade_groups) = build_cascade_bindings(device);
        let material_layout = ModelBatch::material_layout(device);
        let water_layout = WaterBody::layout(device);
        // The scene and shadow layouts come back out rather than staying inside the stage, because the
        // water pass binds both of those groups: it needs the camera and the scene depth from the one,
        // and the fitted cascades from the other. Building second identical layouts for it would be two
        // more declarations to keep in step by hand.
        let (lighting, scene_layout, shadow_layout, resolved_layout) = build_lighting(
            device,
            targets,
            &scene_uniform,
            &shadow_uniform,
            &lighting_shader,
            &composite_shader,
            output_format,
        );
        // Built only when there is a target for the composite to write into, which is the same condition
        // that allocated one. Both come from the settings the targets carry, so the pass and the texture
        // it reads exist or not together rather than by two independent decisions.
        // Only one of the two resolves is ever built, because only one is ever recorded: they share the
        // LDR intermediate and the pass slot, and the settings that allocate the one thing select which.
        let temporal = targets
            .ldr
            .as_ref()
            .zip(targets.history.as_ref())
            .map(|(ldr, history)| {
                build_temporal(
                    device,
                    ldr,
                    history,
                    &targets.motion,
                    &temporal_uniform,
                    &scene_layout,
                    output_format,
                )
            });
        let antialias = targets
            .ldr
            .as_ref()
            .filter(|_| temporal.is_none())
            .map(|ldr| {
                build_antialias(device, ldr, &scene_layout, &resolved_layout, output_format)
            });

        let model = ModelPipelines::new(
            device,
            terrain.bind_group_layout(),
            &cascade_layout,
            &material_layout,
            &model_shader,
        );

        Ok(Self {
            shadow_pipeline: build_shadow_pipeline(
                device,
                terrain.bind_group_layout(),
                &cascade_layout,
                &gbuffer_shader,
            ),
            gbuffer_pipeline: build_gbuffer_pipeline(
                device,
                terrain.bind_group_layout(),
                &gbuffer_shader,
            ),
            model_shadow_pipeline: model.shadow,
            model_gbuffer_pipeline: model.gbuffer,
            model_masked_shadow_pipeline: model.masked_shadow,
            model_masked_gbuffer_pipeline: model.masked_gbuffer,
            material_layout,
            water_pipeline: build_water_pipeline(
                device,
                &scene_layout,
                &water_layout,
                &shadow_layout,
                &water_shader,
            ),
            water_layout,
            ao: build_ao(device, targets, &scene_uniform, &ao_shader),
            lighting,
            antialias,
            temporal,
            temporal_uniform,
            scene_uniform,
            shadow_uniform,
            cascade_uniforms,
            cascade_groups,
            // Off unless asked for. A query set and two small buffers are cheap, but timing costs a
            // blocking readback to be of any use, so it is opt-in rather than always present.
            timer: None,
            render: targets.render,
            output: targets.output,
            visible: RefCell::default(),
            previous: RefCell::default(),
        })
    }

    /// Discards the temporal accumulation, so the next frame starts a new sequence.
    ///
    /// # When a caller has to
    ///
    /// A temporal resolve assumes the next frame continues the last one, and a motion vector is what makes
    /// that assumption safe — it says where each surface point *went*. Some transitions have no answer to
    /// that question at all: a jump cut to another part of the map, a scenario loading, a replay seeking.
    /// Accumulating across one of those blends two unrelated images, and the neighbourhood clamp only
    /// bounds how wrong the result is rather than preventing it.
    ///
    /// A resize does not need this. It rebuilds the whole chain, which replaces this renderer along with
    /// the history it describes — and it has to, because every bind group in it holds views of targets that
    /// were reallocated.
    ///
    /// Cheap: it clears a flag. The next frame writes its own image into the history rather than blending,
    /// which is the same path the very first frame of a sequence takes.
    pub fn reset_history(&self) {
        let mut previous = self.previous.borrow_mut();
        previous.accumulated = false;
    }

    /// Turns per-pass GPU timing on or off, returning whether it is on afterwards.
    ///
    /// Asking for it and getting `false` is a normal answer rather than a fault: `TIMESTAMP_QUERY` is
    /// optional and a software rasteriser may not offer it. The chain then renders exactly as it did
    /// before and [`Self::timings`] reports nothing — the same shape as the render tests skipping when
    /// there is no adapter.
    pub fn set_timing(&mut self, context: &GpuContext, enabled: bool) -> bool {
        self.timer = enabled.then(|| PassTimer::new(context)).flatten();
        self.timer.is_some()
    }

    /// Whether per-pass timing is on.
    #[must_use]
    pub const fn is_timing(&self) -> bool {
        self.timer.is_some()
    }

    /// Reads back the last rendered frame's per-pass breakdown.
    ///
    /// `None` when timing is off. **This blocks until the GPU has finished the frame it is reporting on**,
    /// so it is a diagnostic to take occasionally rather than something to call every frame — polling it
    /// per frame serialises the CPU against the GPU and changes the numbers being read. See
    /// [`PassTimer::read`].
    ///
    /// # Errors
    ///
    /// Returns a structured [`RenderError`] when polling or mapping the readback fails.
    #[must_use]
    pub fn timings(&self, context: &GpuContext) -> Option<Result<FrameTimings, RenderError>> {
        self.timer.as_ref().map(|timer| timer.read(context))
    }

    /// Returns the layout a [`ModelBatch`] binds its materials through.
    ///
    /// Shared rather than created per batch, so the pipelines and every batch bind group are built
    /// against the same layout and cannot drift apart.
    #[must_use]
    pub const fn material_layout(&self) -> &wgpu::BindGroupLayout {
        &self.material_layout
    }

    /// Returns the layout a [`WaterBody`] binds its uniform through.
    ///
    /// Shared for the same reason the material layout is.
    #[must_use]
    pub const fn water_layout(&self) -> &wgpu::BindGroupLayout {
        &self.water_layout
    }

    /// Uploads every per-frame uniform: the scene camera, the fitted cascades, and each cascade view.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::SingularCamera`] when the view-projection cannot be inverted, which
    /// would otherwise leave every reconstructed world position at the origin.
    pub fn set_frame(
        &self,
        context: &GpuContext,
        terrain: &TerrainRenderer,
        models: &[ModelBatch],
        water: &[WaterBody],
        frame: DeferredFrame,
    ) -> Result<[Cascade; CASCADE_COUNT], RenderError> {
        let view = look_at(frame.pose.eye, frame.pose.focus, [0.0, 0.0, 1.0]);
        // Unjittered, and kept: it is what the motion target reprojects against next frame, and what the
        // shadow cascades are fitted from. Jittering the cascade fitting would move every shadow by a
        // sub-pixel amount of the *camera's* pixels, which is not a quantity a light-space frustum has any
        // business knowing about.
        let unjittered = multiply(perspective(frame.projection), view);
        let (view_projection, motion, previous_time) = self.reproject(frame, &unjittered);
        let inverse = invert(view_projection).ok_or(RenderError::SingularCamera)?;

        // The terrain pipelines read the terrain uniform's own view-projection, so it has to agree
        // with the one the lighting pass inverts or the reconstruction lands somewhere else entirely.
        //
        // The wind and scene time ride along in the same block, because the model vertex stages bind it
        // and a swaying vertex needs both. Setting them here rather than leaving it to the caller is what
        // stops a batch being drawn a frame out of step with the rest of the scene — the same reasoning
        // that puts the water bodies' time below.
        terrain.set_frame_animated(
            context,
            &view_projection,
            frame.pose.eye,
            frame.light,
            Animation {
                wind: frame.environment.weather.sanitised().wind,
                time: frame.time,
                previous_time,
            },
            motion,
        );

        let cascades = fit_cascades(
            frame.pose.eye,
            frame.pose.focus,
            frame.projection,
            frame.light.direction,
            frame.shadow_distance,
            // The tallest caster, not the tallest terrain. A model standing on terrain reaches higher
            // than the terrain does, and a cascade sized from terrain alone would fail to record it as
            // an occluder at a low sun.
            models
                .iter()
                .map(ModelBatch::world_top)
                .fold(terrain.height_range(), f32::max),
        );

        let queue = context.queue();
        queue.write_buffer(
            &self.scene_uniform,
            0,
            &scene_bytes(&view_projection, &inverse, frame, self.render, self.output),
        );

        let mut shadow = Vec::with_capacity(SHADOW_UNIFORM_BYTES);
        for cascade in &cascades {
            push_matrix(&mut shadow, &cascade.view_projection);
            push_vec4(
                &mut shadow,
                [0.0, cascade.depth_range, cascade.texel_world, 0.0],
            );
        }
        debug_assert_eq!(shadow.len(), SHADOW_UNIFORM_BYTES, "shadow uniform drifted");
        queue.write_buffer(&self.shadow_uniform, 0, &shadow);

        for (buffer, cascade) in self.cascade_uniforms.iter().zip(&cascades) {
            let mut bytes = Vec::with_capacity(CASCADE_UNIFORM_BYTES);
            push_matrix(&mut bytes, &cascade.view_projection);
            queue.write_buffer(buffer, 0, &bytes);
        }

        // Animated surfaces take their phase from the frame rather than from a clock, so this is the
        // one place scene time enters the GPU. Setting it here rather than leaving it to the caller is
        // what stops a body being drawn a frame out of step with the rest of the scene.
        for body in water {
            body.set_time(context, frame.time);
        }

        self.upload_temporal(queue);

        // Culled against the *unjittered* frustum. A sub-pixel offset cannot bring a chunk into view that a
        // full-pixel frustum excludes, and culling against the jittered one would make the visible set
        // flicker at the jitter period on chunks exactly at the edge — which is the one thing a temporal
        // accumulator handles worst.
        //
        // Done here rather than in `render` because it depends on the frame, and done for the cascades from
        // their *fitted* matrices rather than from the camera, so each cascade draws the casters it actually
        // covers instead of the whole heightfield five times.
        self.cull_terrain(terrain, &unjittered, &cascades);

        // Recorded last, so everything above read the *previous* frame's values.
        {
            let mut previous = self.previous.borrow_mut();
            previous.view_projection = unjittered;
            previous.time = frame.time;
        }

        Ok(cascades)
    }

    /// Applies this frame's sub-pixel jitter and resolves what the previous frame looked like.
    ///
    /// Returns the view-projection to rasterize with, what a geometry pass needs to write a motion vector,
    /// and the previous frame's scene time. Separate from [`Self::set_frame`] because it is the one part of
    /// that function whose correctness is about *two* frames rather than one, and because it is where the
    /// jitter's two roles have to stay consistent: the same offset that moves the projection is the offset
    /// the motion pass subtracts back out.
    fn reproject(
        &self,
        frame: DeferredFrame,
        unjittered: &[[f32; 4]; 4],
    ) -> ([[f32; 4]; 4], Motion, f32) {
        // The jitter is a clip-space translation proportional to `w`, which is why it can be applied to the
        // projection's first two rows and removed again by subtraction in the motion pass. Two render pixels
        // make one normalized unit, hence the doubling.
        let jitter = if self.temporal.is_some() {
            let [x, y] = jitter_offset(frame.jitter);
            [
                x * 2.0 / render_dimension(self.render[0]),
                y * 2.0 / render_dimension(self.render[1]),
            ]
        } else {
            [0.0, 0.0]
        };
        let mut view_projection = *unjittered;
        for column in 0..4 {
            view_projection[column][0] += jitter[0] * unjittered[column][3];
            view_projection[column][1] += jitter[1] * unjittered[column][3];
        }

        // The previous frame's view and time, read before this frame overwrites them. On the very first
        // frame of a sequence there is no predecessor, so this frame stands in for it — which reports
        // exactly zero motion, the honest answer rather than a small wrong one.
        let previous = self.previous.borrow();
        let (previous_view_projection, previous_time) = if previous.accumulated {
            (previous.view_projection, previous.time)
        } else {
            (*unjittered, frame.time)
        };
        (
            view_projection,
            Motion {
                previous_view_projection,
                jitter,
            },
            previous_time,
        )
    }

    /// Uploads the temporal resolve's own block, if there is one.
    ///
    /// Written in [`Self::set_frame`] with every other uniform, so a caller cannot render a frame whose
    /// resolve disagrees with its geometry.
    fn upload_temporal(&self, queue: &wgpu::Queue) {
        if self.temporal.is_none() {
            return;
        }
        let accumulated = self.previous.borrow().accumulated;
        let mut bytes = Vec::with_capacity(TEMPORAL_UNIFORM_BYTES);
        push_vec4(
            &mut bytes,
            [
                TEMPORAL_HISTORY_WEIGHT,
                f32::from(u8::from(accumulated)),
                0.0,
                0.0,
            ],
        );
        debug_assert_eq!(
            bytes.len(),
            TEMPORAL_UNIFORM_BYTES,
            "temporal uniform drifted"
        );
        queue.write_buffer(&self.temporal_uniform, 0, &bytes);
    }

    /// Decides the visible chunk runs for the camera and for each fitted cascade.
    ///
    /// A cascade's frustum is its own, not a subset of the camera's: it reaches *behind* the camera toward
    /// the light, because a caster outside the view can still throw a shadow into it. Culling a cascade
    /// against the camera would remove exactly those casters, and the symptom would be shadows winking out
    /// as their caster left the screen.
    fn cull_terrain(
        &self,
        terrain: &TerrainRenderer,
        view_projection: &[[f32; 4]; 4],
        cascades: &[Cascade; CASCADE_COUNT],
    ) {
        let chunks = terrain.chunks();
        let mut visible = self.visible.borrow_mut();
        let VisibleChunks {
            cascades: cascade_runs,
            camera,
            scratch,
        } = &mut *visible;

        chunks.cull_into(&Frustum::from_view_projection(view_projection), scratch);
        contiguous_runs(scratch, camera);
        for (runs, cascade) in cascade_runs.iter_mut().zip(cascades) {
            chunks.cull_into(
                &Frustum::from_view_projection(&cascade.view_projection),
                scratch,
            );
            contiguous_runs(scratch, runs);
        }
    }

    /// Records the whole chain, ending with the composite into `output`.
    ///
    /// Models draw into the *same* G-buffer pass as terrain and into every shadow cascade, so both
    /// share one depth buffer and occlude each other correctly. Whatever wrote into the G-buffer is
    /// lit identically afterwards, which is the point of deferring.
    pub fn render(
        &self,
        context: &GpuContext,
        terrain: &TerrainRenderer,
        models: &[ModelBatch],
        water: &[WaterBody],
        targets: &DeferredTargets,
        output: &wgpu::TextureView,
    ) {
        let mut encoder =
            context
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("cic-render deferred chain"),
                });

        // Which passes were recorded, in recording order, so only those are resolved. A pass that did not run
        // leaves its slot cleared, which is how an absent pass stays distinguishable from a fast one.
        let mut recorded: Vec<TimedPass> = Vec::with_capacity(TimedPass::ALL.len());

        // 1. Shadow cascades, depth only.
        self.record_shadows(&mut encoder, terrain, models, targets, &mut recorded);

        // 2. G-buffer. Coverage clears to zero, which the lighting pass reads as "sky".
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cic-render gbuffer"),
                color_attachments: &[
                    Some(clear_attachment(&targets.albedo)),
                    Some(clear_attachment(&targets.normal)),
                    Some(clear_attachment(&targets.coverage)),
                    // Cleared to zero, which reads as "did not move" — correct for the sky, which is the
                    // only thing that leaves it unwritten.
                    Some(clear_attachment(&targets.motion)),
                ],
                depth_stencil_attachment: Some(depth_attachment(&targets.depth)),
                multiview_mask: None,
                timestamp_writes: self.time(&mut recorded, TimedPass::Gbuffer),
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.gbuffer_pipeline);
            pass.set_bind_group(0, terrain.bind_group(), &[]);
            terrain.draw_chunks(&mut pass, &self.visible.borrow().camera);

            if !models.is_empty() {
                pass.set_pipeline(&self.model_gbuffer_pipeline);
                pass.set_bind_group(0, terrain.bind_group(), &[]);
                for batch in models {
                    batch.draw(&mut pass, Some(2));
                }
                // Cutout geometry last, and in one run rather than interleaved with the solid draws.
                // Solid geometry has already written its depth by then, so a leaf hidden behind a hull is
                // rejected before its fragment stage runs — which is the whole reason the ordering is
                // worth stating.
                if models.iter().any(ModelBatch::has_cutout) {
                    pass.set_pipeline(&self.model_masked_gbuffer_pipeline);
                    pass.set_bind_group(0, terrain.bind_group(), &[]);
                    for batch in models {
                        batch.draw_cutout(&mut pass, Some(2));
                    }
                }
            }
        }

        // 3 and 4. Occlusion, then its bilateral blur.
        fullscreen_pass(
            &mut encoder,
            "cic-render ao",
            &targets.ao_raw,
            &self.ao.pipeline,
            &[(0, &self.ao.group)],
            self.time(&mut recorded, TimedPass::Occlusion),
        );
        fullscreen_pass(
            &mut encoder,
            "cic-render ao blur",
            &targets.ao_blurred,
            &self.ao.blur_pipeline,
            &[(0, &self.ao.group), (1, &self.ao.source_group)],
            self.time(&mut recorded, TimedPass::OcclusionBlur),
        );

        // 5. Lighting into HDR.
        fullscreen_pass(
            &mut encoder,
            "cic-render lighting",
            &targets.hdr,
            &self.lighting.pipeline,
            &[(0, &self.lighting.group), (2, &self.lighting.shadow_group)],
            self.time(&mut recorded, TimedPass::Lighting),
        );

        // 6. Water, blended over the lit scene inside the HDR target.
        //
        // The target loads rather than clears, since the lighting result is what the water is
        // transparent *against*. No depth attachment: the lighting group bound here already samples
        // the scene depth, and one pass cannot both attach and sample the same texture — so
        // `water_fragment` runs the depth comparison itself against the value it loads.
        if !water.is_empty() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cic-render water"),
                color_attachments: &[Some(load_attachment(&targets.hdr))],
                depth_stencil_attachment: None,
                multiview_mask: None,
                timestamp_writes: self.time(&mut recorded, TimedPass::Water),
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.water_pipeline);
            pass.set_bind_group(0, &self.lighting.group, &[]);
            pass.set_bind_group(2, &self.lighting.shadow_group, &[]);
            for body in water {
                body.draw(&mut pass);
            }
        }

        // 7 and 8. Tone map, downsample, sharpen, and optionally antialias.
        self.record_resolve(&mut encoder, targets, output, &mut recorded);

        // Last, so every pass it names has been recorded. Nothing here reads back — that happens on
        // request in `timings`, long after this submission has been retired.
        if let Some(timer) = &self.timer {
            timer.resolve(&mut encoder, &recorded);
        }

        context.queue().submit([encoder.finish()]);
    }

    /// Records the four depth-only cascade passes.
    ///
    /// Terrain and every model are submitted to each of them, so this is up to four more full geometry
    /// submissions on top of the G-buffer's — the figure the outstanding terrain level-of-detail work
    /// exists to bring down, and the one [`TimedPass::CASCADES`] makes measurable.
    ///
    /// *Up to* four, because a cascade's frustum can catch no chunk at all. That cascade is still recorded,
    /// for the clear, and is deliberately not timed — see [`Self::time_if`].
    fn record_shadows(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        terrain: &TerrainRenderer,
        models: &[ModelBatch],
        targets: &DeferredTargets,
        recorded: &mut Vec<TimedPass>,
    ) {
        let visible = self.visible.borrow();
        for (((layer, group), cascade), runs) in targets
            .shadow_layers
            .iter()
            .zip(&self.cascade_groups)
            .zip(TimedPass::CASCADES)
            .zip(&visible.cascades)
        {
            // A cascade with no casters still gets its pass, because the clear is what makes the layer
            // read as unoccluded instead of as last frame's depth — but it does not get timed. See
            // [`Self::time_if`]: a pass that rasterises nothing cannot be timed at all on Metal, and a
            // near cascade with nothing in it is routine rather than exotic.
            let casts = !runs.is_empty() || !models.is_empty();
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cic-render shadow cascade"),
                color_attachments: &[],
                depth_stencil_attachment: Some(depth_attachment(layer)),
                multiview_mask: None,
                timestamp_writes: self.time_if(casts, recorded, *cascade),
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.shadow_pipeline);
            pass.set_bind_group(0, terrain.bind_group(), &[]);
            pass.set_bind_group(1, group, &[]);
            terrain.draw_chunks(&mut pass, runs);

            // Models cast into the same cascade. Without this a model is lit as though it were
            // present but throws no shadow, which reads as the model floating.
            if !models.is_empty() {
                pass.set_pipeline(&self.model_shadow_pipeline);
                pass.set_bind_group(0, terrain.bind_group(), &[]);
                pass.set_bind_group(1, group, &[]);
                for batch in models {
                    batch.draw(&mut pass, None);
                }
                // And foliage cuts its own shadow, in every cascade. A leaf card casting the rectangle
                // its geometry occupies would darken the ground in slabs.
                if models.iter().any(ModelBatch::has_cutout) {
                    pass.set_pipeline(&self.model_masked_shadow_pipeline);
                    pass.set_bind_group(0, terrain.bind_group(), &[]);
                    pass.set_bind_group(1, group, &[]);
                    for batch in models {
                        batch.draw_cutout(&mut pass, Some(2));
                    }
                }
            }
        }
    }

    /// Returns the timestamp writes for one pass, noting that it was recorded.
    ///
    /// `None` when timing is off. That and [`Self::time_if`] declining a pass with nothing to draw are the
    /// only reasons a pass descriptor carries no writes — so a pass whose slot stays cleared either did not
    /// run or had no work in it, rather than having been forgotten here.
    fn time(
        &self,
        recorded: &mut Vec<TimedPass>,
        pass: TimedPass,
    ) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        let timer = self.timer.as_ref()?;
        recorded.push(pass);
        Some(timer.writes(pass))
    }

    /// The same, for a pass that only sometimes has anything to draw.
    ///
    /// **A pass that issues no draw cannot be timed on every backend, and asking anyway produces a false
    /// claim rather than a missing one.** The two backends realise a pass's timestamp pair differently:
    /// Vulkan writes them with commands recorded at the pass boundaries, which run whatever the pass
    /// contains, while Metal declares them as *stage* boundaries on the pass descriptor — the beginning at
    /// the start of the vertex stage and the end at the end of the fragment stage. A pass that rasterises
    /// nothing never reaches the second one, so on Metal the beginning lands, the end stays at the zero the
    /// resolve buffer was cleared to, and [`crate::timing::timings_from_ticks`] reads the pair as a pass
    /// that did not run. Timing it anyway therefore says "did not run" about a pass that did, on one backend
    /// and not the other. Declining the pair up front makes the absence mean the same thing everywhere:
    /// this pass had nothing to draw.
    ///
    /// A shadow cascade is why this exists. The near cascade covers the first few percent of the shadow
    /// distance, so any camera an appreciable height above the ground has one whose frustum sits entirely
    /// in the air and catches no chunk — the common case for this game's camera, not a corner of it. That
    /// it is *absent* rather than reported at nearly zero is the more useful answer anyway: it says the
    /// cascade found no casters, which is a thing worth knowing about a shadow distance.
    ///
    /// What this gives up, stated plainly: the clear itself stops being attributed to anything, so
    /// [`crate::timing::FrameTimings::sum`] under-counts the frame by it. On hardware that is not a real
    /// figure — a cascade holding geometry measures a tenth of a millisecond on an M1 Pro, and the clear
    /// alone is beneath that. On llvmpipe it is 8.7ms, because clearing four million depth values on a CPU
    /// is genuine work. The trade is deliberate: a breakdown that means the same thing on every backend is
    /// worth more than one that accounts for a clear nobody can act on without deleting the cascade.
    fn time_if(
        &self,
        draws: bool,
        recorded: &mut Vec<TimedPass>,
        pass: TimedPass,
    ) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        if draws {
            self.time(recorded, pass)
        } else {
            None
        }
    }

    /// Records the two passes that turn the HDR scene into the caller's image.
    ///
    /// Separate from [`Self::render`] because it is the half of the chain that works in *output* pixels
    /// rather than render pixels — everything above it is sized to the G-buffer.
    fn record_resolve(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        targets: &DeferredTargets,
        output: &wgpu::TextureView,
        recorded: &mut Vec<TimedPass>,
    ) {
        // The composite goes straight into the caller's target when nothing follows it, and into the LDR
        // intermediate when the antialias pass does. Paired rather than checked separately: the stage and
        // the target it reads were decided by the same settings, and taking them together means a
        // half-configured chain draws the frame without the resolve rather than skipping the composite
        // or panicking.
        let resolve = self.antialias.as_ref().zip(targets.ldr.as_ref());
        let temporal = self
            .temporal
            .as_ref()
            .zip(targets.ldr.as_ref())
            .zip(targets.history.as_ref());
        let composite_target = match (resolve, temporal) {
            (Some((_, ldr)), _) | (_, Some(((_, ldr), _))) => ldr,
            (None, None) => output,
        };
        fullscreen_pass(
            encoder,
            "cic-render composite",
            composite_target,
            &self.lighting.composite_pipeline,
            &[
                (0, &self.lighting.group),
                (1, &self.lighting.composite_group),
            ],
            self.time(recorded, TimedPass::Composite),
        );

        if let Some((antialias, _)) = resolve {
            fullscreen_pass(
                encoder,
                "cic-render antialias",
                output,
                &antialias.pipeline,
                &[(0, &self.lighting.group), (1, &antialias.group)],
                self.time(recorded, TimedPass::Antialias),
            );
        }

        if let Some(((stage, _), history)) = temporal {
            // Written to the layer the resolve is *not* reading, then swapped. Reading and writing one
            // texture in a pass is not allowed and would not be meaningful anyway: each output pixel reads a
            // reprojected neighbourhood rather than itself.
            let source = self.previous.borrow().history;
            let target = (source + 1) % 2;
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("cic-render temporal resolve"),
                    color_attachments: &[
                        Some(clear_attachment(output)),
                        Some(clear_attachment(&history.layers[target])),
                    ],
                    depth_stencil_attachment: None,
                    multiview_mask: None,
                    timestamp_writes: self.time(recorded, TimedPass::Antialias),
                    occlusion_query_set: None,
                });
                pass.set_pipeline(&stage.pipeline);
                pass.set_bind_group(0, &self.lighting.group, &[]);
                pass.set_bind_group(1, &stage.groups[source], &[]);
                pass.draw(0..3, 0..1);
            }
            let mut previous = self.previous.borrow_mut();
            previous.history = target;
            previous.accumulated = true;
        }
    }
}

/// The share of a temporal result taken from the history when nothing is moving.
///
/// Duplicated across the language boundary because the shader documents the trade in full and the CPU needs
/// the number to upload. A test pins the pair rather than trusting the comment.
const TEMPORAL_HISTORY_WEIGHT: f32 = 0.9;

/// One render dimension as a float, floored at one so the jitter cannot divide by zero.
fn render_dimension(size: u32) -> f32 {
    // Bounded by `MAX_RENDER_DIMENSION`, far inside exact `f32` range.
    #[allow(clippy::cast_precision_loss)]
    {
        size.max(1) as f32
    }
}

/// Packs the `SceneCamera` uniform block.
///
/// `render` is the size the scene passes run at and `output` the size of the caller's target; they are
/// equal at a resolution scale of one. Both come from the targets, which is what stops them disagreeing
/// with the textures they describe.
fn scene_bytes(
    view_projection: &[[f32; 4]; 4],
    inverse: &[[f32; 4]; 4],
    frame: DeferredFrame,
    render: [u32; 2],
    output: [u32; 2],
) -> Vec<u8> {
    let mut scene = Vec::with_capacity(SCENE_UNIFORM_BYTES);
    push_matrix(&mut scene, view_projection);
    push_matrix(&mut scene, inverse);
    let [ex, ey, ez] = frame.pose.eye;
    push_vec4(&mut scene, [ex, ey, ez, 0.0]);
    // Both sizes and their reciprocals, so the shaders never divide per pixel.
    push_vec4(&mut scene, size_and_reciprocal(render));
    // Slot 0 is the primary and the only shadowed light. The two fill slots are zeroed: the shader
    // skips a zero-length direction, and leaving them empty is what makes shadow contrast achievable,
    // since every fill would contribute unoccluded ambient that shade cannot remove.
    let [ar, ag, ab] = frame.light.ambient;
    let [dr, dg, db] = frame.light.diffuse;
    let [lx, ly, lz] = normalize(frame.light.direction);
    push_vec4(&mut scene, [ar, ag, ab, 0.0]);
    push_vec4(&mut scene, [dr, dg, db, 0.0]);
    // The shader lights a receiver by `-source_direction`, so this is the direction the light travels
    // along rather than the direction toward it.
    push_vec4(&mut scene, [-lx, -ly, -lz, 0.0]);
    for _ in 0..6 {
        push_vec4(&mut scene, [0.0; 4]);
    }

    let environment = frame.environment;
    let weather = environment.weather.sanitised();
    // The fog colour is the sky's horizon colour, not a separately authored one. Fog fades distance
    // toward whatever is behind it, and behind it is the sky — a fog colour that disagrees puts a band
    // along the horizon precisely where the terrain silhouette meets it. Overcast desaturates both
    // together, which is why this is computed rather than configured.
    let [fog_r, fog_g, fog_b] = mix_colour(SKY_HORIZON, OVERCAST_HORIZON, weather.overcast);
    push_vec4(
        &mut scene,
        [fog_r, fog_g, fog_b, environment.fog.density.max(0.0)],
    );
    push_vec4(
        &mut scene,
        [
            environment.fog.height_falloff.max(0.001),
            environment.fog.base,
            environment.fog.patchiness.clamp(0.0, 1.0),
            environment.fog.patch_scale.max(1.0),
        ],
    );
    let clouds = environment.clouds;
    push_vec4(
        &mut scene,
        [
            clouds.coverage.clamp(0.0, 1.0),
            clouds.scale.max(1.0),
            clouds.strength.clamp(0.0, 1.0),
            clouds.softness.clamp(0.0, 1.0),
        ],
    );
    // Drift is the wind integrated over scene time, computed here rather than in the shader so the two
    // callers of `cloud_shadow` cannot disagree about where the deck has got to.
    push_vec4(
        &mut scene,
        [
            weather.wind[0] * frame.time,
            weather.wind[1] * frame.time,
            0.0,
            0.0,
        ],
    );
    push_vec4(&mut scene, [weather.wetness, weather.snow, 0.0, 0.0]);
    // Last in the block, and it has to stay last: `terrain_ao.wgsl` binds this same buffer through a
    // struct declaring only the fields it reads, which holds exactly as long as that declaration is a
    // prefix of this one.
    push_vec4(&mut scene, size_and_reciprocal(output));

    debug_assert_eq!(scene.len(), SCENE_UNIFORM_BYTES, "scene uniform drifted");
    scene
}

/// Packs a pixel size as `[width, height, 1 / width, 1 / height]`.
///
/// Dimensions are bounded by [`crate::display::MAX_RENDER_DIMENSION`], far inside the range `f32`
/// represents exactly, and the reciprocal is taken against a floor of one so a zero cannot produce an
/// infinity the shaders would multiply every coordinate by.
#[allow(clippy::cast_precision_loss)]
fn size_and_reciprocal(size: [u32; 2]) -> [f32; 4] {
    let (width, height) = (size[0] as f32, size[1] as f32);
    [width, height, 1.0 / width.max(1.0), 1.0 / height.max(1.0)]
}

/// The clear sky's horizon colour, matching `SKY_HORIZON` in `atmosphere.wgsl`.
///
/// Duplicated across the language boundary because the shader needs it as a constant and the fog colour
/// is derived from it on the CPU. A test pins the pair rather than trusting the comment.
const SKY_HORIZON: [f32; 3] = [0.12, 0.20, 0.30];

/// The horizon under a full cloud deck: brighter, and desaturated toward grey.
///
/// Brighter *and* flatter, which is the part that is easy to get backwards. An overcast sky scatters the
/// sun across the whole dome, so the horizon gains light even as the ground loses it.
const OVERCAST_HORIZON: [f32; 3] = [0.34, 0.36, 0.39];

fn mix_colour(from: [f32; 3], to: [f32; 3], amount: f32) -> [f32; 3] {
    let amount = amount.clamp(0.0, 1.0);
    [
        from[0] + (to[0] - from[0]) * amount,
        from[1] + (to[1] - from[1]) * amount,
        from[2] + (to[2] - from[2]) * amount,
    ]
}

/// Builds one uniform buffer and bind group per shadow cascade.
///
/// Their own group, so the terrain's bindings stay bound across all four shadow passes and only this
/// one is swapped.
fn build_cascade_bindings(
    device: &wgpu::Device,
) -> (
    wgpu::BindGroupLayout,
    Vec<wgpu::Buffer>,
    Vec<wgpu::BindGroup>,
) {
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cic-render cascade view layout"),
        entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX)],
    });
    let mut uniforms = Vec::with_capacity(CASCADE_COUNT);
    let mut groups = Vec::with_capacity(CASCADE_COUNT);
    for _ in 0..CASCADE_COUNT {
        let buffer = uniform_buffer(device, "cic-render cascade view", CASCADE_UNIFORM_BYTES);
        groups.push(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cic-render cascade view bindings"),
            layout: &layout,
            entries: &[buffer_entry(0, &buffer)],
        }));
        uniforms.push(buffer);
    }
    (layout, uniforms, groups)
}

/// Builds the depth-only shadow pipeline.
fn build_shadow_pipeline(
    device: &wgpu::Device,
    terrain_layout: &wgpu::BindGroupLayout,
    cascade_layout: &wgpu::BindGroupLayout,
    gbuffer_shader: &wgpu::ShaderModule,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("cic-render shadow pipeline layout"),
        bind_group_layouts: &[Some(terrain_layout), Some(cascade_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("cic-render terrain shadow pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: gbuffer_shader,
            entry_point: Some("shadow_vertex"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            // Slope-scaled bias in the rasterizer rather than a second term in the receiver's
            // comparison. A surface nearly edge-on to the light spans many depth units per texel, and
            // only the rasterizer knows the actual slope; a constant receiver-side bias large enough
            // for those cases detaches every other shadow from its caster.
            bias: wgpu::DepthBiasState {
                constant: 2,
                slope_scale: 2.5,
                clamp: 0.0,
            },
        }),
        multisample: wgpu::MultisampleState::default(),
        // No fragment stage: terrain is opaque, so the depth write is the entire output.
        fragment: None,
        multiview_mask: None,
        cache: None,
    })
}

/// Builds the G-buffer pipeline.
fn build_gbuffer_pipeline(
    device: &wgpu::Device,
    terrain_layout: &wgpu::BindGroupLayout,
    gbuffer_shader: &wgpu::ShaderModule,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("cic-render gbuffer pipeline layout"),
        bind_group_layouts: &[Some(terrain_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("cic-render terrain gbuffer pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: gbuffer_shader,
            entry_point: Some("gbuffer_vertex"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: gbuffer_shader,
            entry_point: Some("gbuffer_fragment"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[
                Some(colour_target(ALBEDO_FORMAT)),
                Some(colour_target(NORMAL_FORMAT)),
                Some(colour_target(COVERAGE_FORMAT)),
                Some(colour_target(MOTION_FORMAT)),
            ],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// The four pipelines an instanced model draws through: two paths times two passes.
///
/// Grouped because they are one decision built four ways, and because constructing them inline made the
/// renderer's constructor long enough to hide the rest of what it does.
struct ModelPipelines {
    gbuffer: wgpu::RenderPipeline,
    shadow: wgpu::RenderPipeline,
    masked_gbuffer: wgpu::RenderPipeline,
    masked_shadow: wgpu::RenderPipeline,
}

impl ModelPipelines {
    fn new(
        device: &wgpu::Device,
        terrain_layout: &wgpu::BindGroupLayout,
        cascade_layout: &wgpu::BindGroupLayout,
        material_layout: &wgpu::BindGroupLayout,
        model_shader: &wgpu::ShaderModule,
    ) -> Self {
        Self {
            gbuffer: build_model_gbuffer_pipeline(
                device,
                terrain_layout,
                material_layout,
                model_shader,
                false,
            ),
            shadow: build_model_shadow_pipeline(
                device,
                terrain_layout,
                cascade_layout,
                material_layout,
                model_shader,
                false,
            ),
            masked_gbuffer: build_model_gbuffer_pipeline(
                device,
                terrain_layout,
                material_layout,
                model_shader,
                true,
            ),
            masked_shadow: build_model_shadow_pipeline(
                device,
                terrain_layout,
                cascade_layout,
                material_layout,
                model_shader,
                true,
            ),
        }
    }
}

/// Builds the instanced model G-buffer pipeline, opaque or alpha-tested.
///
/// Group 1 is left empty: the shader declares the shadow cascade there, and neither entry point uses it.
/// The pipeline layout takes an optional layout per slot for exactly this case.
///
/// The two differ in three things, and each of them is why they are two pipelines rather than one:
///
/// - **The fragment stage.** A stage that *can* discard forfeits early depth rejection on most hardware,
///   and opaque geometry is the overwhelming majority. Paying for the possibility everywhere to serve the
///   foliage would be the wrong trade.
/// - **Culling.** A leaf card is a single quad meant to be seen from either face, so a masked pipeline
///   draws both. Culling one would make half of a canopy vanish depending on where the camera stands.
/// - **Nothing else.** Same targets, same depth state, same vertex stage.
fn build_model_gbuffer_pipeline(
    device: &wgpu::Device,
    terrain_layout: &wgpu::BindGroupLayout,
    material_layout: &wgpu::BindGroupLayout,
    model_shader: &wgpu::ShaderModule,
    masked: bool,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("cic-render model gbuffer pipeline layout"),
        bind_group_layouts: &[Some(terrain_layout), None, Some(material_layout)],
        immediate_size: 0,
    });
    let buffers = buffer_layouts();
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("cic-render model gbuffer pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: model_shader,
            entry_point: Some("gbuffer_vertex"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &buffers,
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            // Back faces are culled for opaque geometry, unlike terrain. A model is a closed solid, so
            // its back faces are its interior; drawing them wastes fill and can win the depth test at
            // grazing angles. Alpha-tested geometry is the opposite case — see above.
            cull_mode: if masked { None } else { Some(wgpu::Face::Back) },
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: model_shader,
            entry_point: Some(if masked {
                "gbuffer_masked_fragment"
            } else {
                "gbuffer_fragment"
            }),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[
                Some(colour_target(ALBEDO_FORMAT)),
                Some(colour_target(NORMAL_FORMAT)),
                Some(colour_target(COVERAGE_FORMAT)),
                Some(colour_target(MOTION_FORMAT)),
            ],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Builds the instanced model shadow pipeline, depth-only or alpha-tested.
///
/// The masked variant is the reason the alpha test cannot live in the lit frame alone: a leaf card that
/// cast a rectangular shadow would be *worse* than one that cast none, because the eye reads a hard
/// quadrilateral on the ground as a solid object. So it carries a fragment stage whose only output is the
/// discard, and it binds the materials the opaque variant does not need.
fn build_model_shadow_pipeline(
    device: &wgpu::Device,
    terrain_layout: &wgpu::BindGroupLayout,
    cascade_layout: &wgpu::BindGroupLayout,
    material_layout: &wgpu::BindGroupLayout,
    model_shader: &wgpu::ShaderModule,
    masked: bool,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("cic-render model shadow pipeline layout"),
        bind_group_layouts: &[
            Some(terrain_layout),
            Some(cascade_layout),
            masked.then_some(material_layout),
        ],
        immediate_size: 0,
    });
    let buffers = buffer_layouts();
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("cic-render model shadow pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: model_shader,
            entry_point: Some(if masked {
                "shadow_masked_vertex"
            } else {
                "shadow_vertex"
            }),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &buffers,
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            // Front faces are culled for a solid, not back. Recording only the far side moves the stored
            // depth away from the receiver, which removes self-shadowing acne at its source rather than
            // biasing it away afterwards. A leaf card has no far side to record, so it draws both.
            cull_mode: if masked {
                None
            } else {
                Some(wgpu::Face::Front)
            },
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState {
                constant: 2,
                slope_scale: 2.5,
                clamp: 0.0,
            },
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: masked.then(|| wgpu::FragmentState {
            module: model_shader,
            entry_point: Some("shadow_masked_fragment"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// The ambient-occlusion pass and its bilateral blur.
#[derive(Debug)]
struct AoStage {
    pipeline: wgpu::RenderPipeline,
    blur_pipeline: wgpu::RenderPipeline,
    group: wgpu::BindGroup,
    source_group: wgpu::BindGroup,
}

fn build_ao(
    device: &wgpu::Device,
    targets: &DeferredTargets,
    scene_uniform: &wgpu::Buffer,
    ao_shader: &wgpu::ShaderModule,
) -> AoStage {
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cic-render ao layout"),
        entries: &[
            texture_entry(0, wgpu::TextureSampleType::Float { filterable: false }),
            texture_entry(1, wgpu::TextureSampleType::Float { filterable: false }),
            uniform_entry(2, wgpu::ShaderStages::FRAGMENT),
            texture_entry(3, wgpu::TextureSampleType::Depth),
        ],
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("cic-render ao bindings"),
        layout: &layout,
        entries: &[
            view_entry(0, &targets.normal),
            view_entry(1, &targets.coverage),
            buffer_entry(2, scene_uniform),
            view_entry(3, &targets.depth),
        ],
    });

    let source_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cic-render ao source layout"),
        entries: &[texture_entry(
            0,
            wgpu::TextureSampleType::Float { filterable: false },
        )],
    });
    let source_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("cic-render ao source bindings"),
        layout: &source_layout,
        entries: &[view_entry(0, &targets.ao_raw)],
    });

    AoStage {
        pipeline: fullscreen_pipeline(
            device,
            "cic-render ao pipeline",
            ao_shader,
            "ao_fragment",
            &[Some(&layout)],
            AO_FORMAT,
        ),
        blur_pipeline: fullscreen_pipeline(
            device,
            "cic-render ao blur pipeline",
            ao_shader,
            "ao_blur_fragment",
            &[Some(&layout), Some(&source_layout)],
            AO_FORMAT,
        ),
        group,
        source_group,
    }
}

/// The deferred lighting resolve and the tone-mapping composite.
#[derive(Debug)]
struct LightingStage {
    pipeline: wgpu::RenderPipeline,
    composite_pipeline: wgpu::RenderPipeline,
    group: wgpu::BindGroup,
    composite_group: wgpu::BindGroup,
    /// Group 2: how the primary light's shadow term is produced.
    ///
    /// Held here rather than beside the pipelines because the two passes that bind it — lighting and
    /// water — are recorded from different places, and both take it from the same field so neither can
    /// bind a stale one.
    shadow_group: wgpu::BindGroup,
}

/// Builds the lighting and composite stage, returning the three bind group layouts as well.
///
/// The scene layout is returned because the water pass and both resolves bind that very group and would
/// otherwise need an identical second declaration. The shadow layout is returned because the water pass
/// samples the shadow term too. The composite layout is returned because the antialias pass wants exactly
/// the same shape — one sampled colour texture and one filtering sampler — and reusing it is one
/// declaration instead of two that must not drift.
///
/// # Why the shadow bindings are their own group
///
/// They were part of the scene layout, which meant the composite and both antialias resolves bound a
/// shadow array and a cascade uniform none of them samples, purely because sharing one layout with the
/// lighting pass was cheaper than declaring a second. Splitting them costs one more `set_bind_group` on
/// the two passes that do sample it and buys the property that matters later: how the shadow term is
/// *produced* is now one layout and one WGSL chunk, replaceable without touching a binding the rest of
/// the chain depends on. See `shaders/shadow.wgsl`.
fn build_lighting(
    device: &wgpu::Device,
    targets: &DeferredTargets,
    scene_uniform: &wgpu::Buffer,
    shadow_uniform: &wgpu::Buffer,
    lighting_shader: &wgpu::ShaderModule,
    composite_shader: &wgpu::ShaderModule,
    output_format: wgpu::TextureFormat,
) -> (
    LightingStage,
    wgpu::BindGroupLayout,
    wgpu::BindGroupLayout,
    wgpu::BindGroupLayout,
) {
    let scene_sampler = build_scene_sampler(device);

    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cic-render scene layout"),
        entries: &[
            texture_entry(0, wgpu::TextureSampleType::Float { filterable: false }),
            texture_entry(1, wgpu::TextureSampleType::Float { filterable: false }),
            texture_entry(2, wgpu::TextureSampleType::Float { filterable: false }),
            // Visible to the vertex stage as well, for the water pass: its vertex shader transforms by
            // this block's view-projection. The two fullscreen entry points using the same group do
            // not read it there, and extra visibility costs them nothing.
            uniform_entry(3, wgpu::ShaderStages::VERTEX_FRAGMENT),
            texture_entry(4, wgpu::TextureSampleType::Float { filterable: false }),
            texture_entry(5, wgpu::TextureSampleType::Depth),
        ],
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("cic-render scene bindings"),
        layout: &layout,
        entries: &[
            view_entry(0, &targets.albedo),
            view_entry(1, &targets.normal),
            view_entry(2, &targets.coverage),
            buffer_entry(3, scene_uniform),
            view_entry(4, &targets.ao_blurred),
            view_entry(5, &targets.depth),
        ],
    });

    let (shadow_layout, shadow_group) = build_shadow_bindings(device, targets, shadow_uniform);

    let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cic-render composite layout"),
        entries: &[
            texture_entry(0, wgpu::TextureSampleType::Float { filterable: true }),
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let composite_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("cic-render composite bindings"),
        layout: &composite_layout,
        entries: &[
            view_entry(0, &targets.hdr),
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&scene_sampler),
            },
        ],
    });

    let stage = LightingStage {
        pipeline: fullscreen_pipeline(
            device,
            "cic-render lighting pipeline",
            lighting_shader,
            "lighting_fragment",
            // The hole at group 1 is the per-pass slot this pass has no resources for. Group 2 is the
            // shadow provider, and it keeps that index here so the one WGSL declaration in
            // `shadow.wgsl` serves this pass and the water pass alike.
            &[Some(&layout), None, Some(&shadow_layout)],
            HDR_FORMAT,
        ),
        composite_pipeline: fullscreen_pipeline(
            device,
            "cic-render composite pipeline",
            composite_shader,
            "composite_fragment",
            // No shadow group: the composite tone maps a lit image and never asks how it was shadowed.
            &[Some(&layout), Some(&composite_layout)],
            output_format,
        ),
        group,
        composite_group,
        shadow_group,
    };
    (stage, layout, shadow_layout, composite_layout)
}

/// Builds group 2: everything the shadow term is produced *from*.
///
/// Separate from [`build_lighting`] because this is the one group a different shadow technique would
/// replace wholesale. A provider that ray-traced the term would supply this function's counterpart —
/// declaring an acceleration structure where the depth array and its comparison sampler are — and
/// nothing above it would change, because the passes that bind group 2 do not know what is in it.
fn build_shadow_bindings(
    device: &wgpu::Device,
    targets: &DeferredTargets,
    shadow_uniform: &wgpu::Buffer,
) -> (wgpu::BindGroupLayout, wgpu::BindGroup) {
    let comparison_sampler = build_shadow_sampler(device);
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cic-render shadow layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                count: None,
            },
            uniform_entry(2, wgpu::ShaderStages::FRAGMENT),
        ],
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("cic-render shadow bindings"),
        layout: &layout,
        entries: &[
            view_entry(0, &targets.shadow_array),
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&comparison_sampler),
            },
            buffer_entry(2, shadow_uniform),
        ],
    });
    (layout, group)
}

/// The antialias resolve: one fullscreen pass over the tone-mapped image.
#[derive(Debug)]
struct AntialiasStage {
    pipeline: wgpu::RenderPipeline,
    group: wgpu::BindGroup,
}

/// Builds the antialias pass against the LDR intermediate it reads.
///
/// Group 0 is the scene group, as the composite's is, purely for the scene uniform: the pass needs the
/// output size to step by one pixel and nothing else from it. Group 1 reuses the composite's layout, with
/// the tone-mapped image bound where the HDR target is bound there. No shadow group — it resolves edges in
/// an already-lit image.
fn build_antialias(
    device: &wgpu::Device,
    ldr: &wgpu::TextureView,
    scene_layout: &wgpu::BindGroupLayout,
    resolved_layout: &wgpu::BindGroupLayout,
    output_format: wgpu::TextureFormat,
) -> AntialiasStage {
    let sampler = build_scene_sampler(device);
    AntialiasStage {
        pipeline: fullscreen_pipeline(
            device,
            "cic-render antialias pipeline",
            &shader_module(device, "antialias"),
            "antialias_fragment",
            &[Some(scene_layout), Some(resolved_layout)],
            output_format,
        ),
        group: device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cic-render antialias bindings"),
            layout: resolved_layout,
            entries: &[
                view_entry(0, ldr),
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        }),
    }
}

/// The temporal resolve: one fullscreen pass that reads the tone-mapped image, the motion target and the
/// history, and writes the presented frame and the next history in one go.
#[derive(Debug)]
struct TemporalStage {
    pipeline: wgpu::RenderPipeline,
    /// One group per history layer. The resolve binds the group that *reads* the layer it is not writing.
    ///
    /// Two rather than one with the layer as a uniform, because a colour attachment is an exclusive usage
    /// over the layers it covers: an array view of both layers bound as a sampled resource while one of them
    /// is the attachment is refused by the API, and correctly so — nothing defines what such a read returns.
    groups: [wgpu::BindGroup; 2],
}

/// Builds the temporal resolve against the targets it reads.
///
/// Group 0 is the lighting group, as the composite's and the post pass's are, purely for the scene uniform:
/// this pass needs the output size to step by one pixel and nothing else from it. Group 1 is its own — it is
/// the only pass in the chain reading four different resources, so there is nothing to share a layout with.
fn build_temporal(
    device: &wgpu::Device,
    ldr: &wgpu::TextureView,
    history: &HistoryTargets,
    motion: &wgpu::TextureView,
    uniform: &wgpu::Buffer,
    lighting_layout: &wgpu::BindGroupLayout,
    output_format: wgpu::TextureFormat,
) -> TemporalStage {
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cic-render temporal layout"),
        entries: &[
            texture_entry(0, FILTERABLE),
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            texture_entry(2, FILTERABLE),
            texture_entry(3, FILTERABLE),
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });
    // One filtering sampler for all three textures. The motion target is at render resolution and the other
    // two at output resolution, and a filtered read is the right thing for each: a bilinear tap of the motion
    // field is a reasonable estimate between two samples of a continuous flow, and the history is being read
    // at a fractional coordinate by construction.
    let sampler = build_scene_sampler(device);
    let group = |source: &wgpu::TextureView| {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cic-render temporal"),
            layout: &layout,
            entries: &[
                view_entry(0, ldr),
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                view_entry(2, source),
                view_entry(3, motion),
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: uniform.as_entire_binding(),
                },
            ],
        })
    };
    let groups = [group(&history.layers[0]), group(&history.layers[1])];

    let module = shader_module(device, "taa");
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("cic-render temporal pipeline layout"),
        bind_group_layouts: &[Some(lighting_layout), Some(&layout)],
        immediate_size: 0,
    });
    TemporalStage {
        pipeline: device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cic-render temporal pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("fullscreen_vertex"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("taa_fragment"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                // Two attachments: the frame the viewer sees and the history the next frame reads. See
                // `TemporalOutput` in `taa.wgsl` for why this is one pass rather than a pass and a copy.
                targets: &[
                    Some(colour_target(output_format)),
                    Some(colour_target(HISTORY_FORMAT)),
                ],
            }),
            multiview_mask: None,
            cache: None,
        }),
        groups,
    }
}

/// Builds the water pipeline.
///
/// Not a [`fullscreen_pipeline`]: water has its own procedural grid rather than one covering triangle,
/// and it blends rather than overwriting.
fn build_water_pipeline(
    device: &wgpu::Device,
    scene_layout: &wgpu::BindGroupLayout,
    water_layout: &wgpu::BindGroupLayout,
    shadow_layout: &wgpu::BindGroupLayout,
    water_shader: &wgpu::ShaderModule,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("cic-render water pipeline layout"),
        // Contiguous, with no empty slot. Water reuses the scene group because it needs the camera and
        // the scene depth already declared there; its own uniform follows in group 1, and the shadow
        // provider it samples the light's visibility from is group 2, the same index the lighting pass
        // binds it at. It declares only the group-0 bindings it reads, and a layout carrying more than
        // a shader uses is allowed.
        bind_group_layouts: &[Some(scene_layout), Some(water_layout), Some(shadow_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("cic-render water pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: water_shader,
            entry_point: Some("water_vertex"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            // Unculled. The surface is a single sheet rather than a solid, so it has no interior, and
            // a camera that dips below the waterline should still see it from underneath.
            cull_mode: None,
            ..Default::default()
        },
        // No depth state, because there is no depth attachment. See the water pass in `render`.
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: water_shader,
            entry_point: Some("water_fragment"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: HDR_FORMAT,
                blend: Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::SrcAlpha,
                        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                        operation: wgpu::BlendOperation::Add,
                    },
                    // Replaced rather than accumulated: nothing downstream reads the HDR target's
                    // alpha, so there is no coverage to keep track of across bodies.
                    alpha: wgpu::BlendComponent::REPLACE,
                }),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// A depth-comparison sampler, which is what makes shadow filtering cheap.
///
/// The hardware compares each tap against the receiver's depth and returns the *fraction* that
/// passed, so a 3x3 kernel gets sub-texel filtering for free rather than nine binary results.
fn build_shadow_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("cic-render shadow comparison sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        compare: Some(wgpu::CompareFunction::LessEqual),
        ..Default::default()
    })
}

/// The sample type every colour texture in the chain is read as.
///
/// Named rather than repeated, because the *other* value is a real possibility and a silent one: a
/// `Rg16Float` bound as unfilterable would refuse the sampler beside it at bind group creation, and the
/// error names a binding index rather than the decision behind it.
const FILTERABLE: wgpu::TextureSampleType = wgpu::TextureSampleType::Float { filterable: true };

/// A plain filtering sampler for reading the HDR scene during the composite.
fn build_scene_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("cic-render scene sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    })
}

/// Records one fullscreen pass that overwrites its target.
fn fullscreen_pass(
    encoder: &mut wgpu::CommandEncoder,
    label: &str,
    target: &wgpu::TextureView,
    pipeline: &wgpu::RenderPipeline,
    // Each group carries the index it binds at rather than taking it from its position, so a pass with
    // a hole — lighting binds 0 and 2, skipping the per-pass slot it has no use for — is expressible,
    // and so that every call site states which group is which instead of implying it by order.
    groups: &[(u32, &wgpu::BindGroup)],
    timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(clear_attachment(target))],
        depth_stencil_attachment: None,
        multiview_mask: None,
        timestamp_writes,
        occlusion_query_set: None,
    });
    pass.set_pipeline(pipeline);
    for (index, group) in groups {
        pass.set_bind_group(*index, *group, &[]);
    }
    pass.draw(0..3, 0..1);
}

/// Builds a shader module from a composed program.
///
/// # Panics
///
/// Panics when `name` is not a declared program. Every call site passes a literal that the shader tests
/// also compose and validate, so a wrong name is a build-time mistake caught by `cargo test` rather than
/// a condition a caller can hit at runtime.
fn shader_module(device: &wgpu::Device, name: &str) -> wgpu::ShaderModule {
    let source = crate::shader::compose(name)
        .unwrap_or_else(|| panic!("{name} is not a declared shader program"));
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(name),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
}

/// A colour attachment that keeps what the target already holds.
///
/// The water pass needs this: it blends against the lighting result, so clearing first would leave it
/// transparent over nothing.
const fn load_attachment(view: &wgpu::TextureView) -> wgpu::RenderPassColorAttachment<'_> {
    wgpu::RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: None,
        ops: wgpu::Operations {
            load: wgpu::LoadOp::Load,
            store: wgpu::StoreOp::Store,
        },
    }
}

fn clear_attachment(view: &wgpu::TextureView) -> wgpu::RenderPassColorAttachment<'_> {
    wgpu::RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: None,
        ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
            store: wgpu::StoreOp::Store,
        },
    }
}

const fn depth_attachment(view: &wgpu::TextureView) -> wgpu::RenderPassDepthStencilAttachment<'_> {
    wgpu::RenderPassDepthStencilAttachment {
        view,
        depth_ops: Some(wgpu::Operations {
            load: wgpu::LoadOp::Clear(1.0),
            store: wgpu::StoreOp::Store,
        }),
        stencil_ops: None,
    }
}

fn fullscreen_pipeline(
    device: &wgpu::Device,
    label: &str,
    shader: &wgpu::ShaderModule,
    entry_point: &str,
    // `Option` per group rather than a plain slice, because the lighting pass has a hole: its scene
    // bindings are group 0 and the shadow provider's are group 2, with group 1 belonging to whatever
    // per-pass resources a program has and lighting having none. Declaring the hole is how a group
    // index stays the same across the two programs that bind it.
    layouts: &[Option<&wgpu::BindGroupLayout>],
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: layouts,
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("fullscreen_vertex"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(entry_point),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(colour_target(format))],
        }),
        multiview_mask: None,
        cache: None,
    })
}

const fn colour_target(format: wgpu::TextureFormat) -> wgpu::ColorTargetState {
    wgpu::ColorTargetState {
        format,
        blend: None,
        write_mask: wgpu::ColorWrites::ALL,
    }
}

/// Allocates a uniform buffer.
///
/// Shared with [`crate::water`], which builds its own block against the same conventions.
pub(crate) fn uniform_buffer(device: &wgpu::Device, label: &str, size: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

pub(crate) const fn uniform_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

const fn texture_entry(
    binding: u32,
    sample_type: wgpu::TextureSampleType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

const fn view_entry(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    }
}

pub(crate) fn buffer_entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

fn push_matrix(bytes: &mut Vec<u8>, matrix: &[[f32; 4]; 4]) {
    for column in matrix {
        for value in column {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
}

pub(crate) fn push_vec4(bytes: &mut Vec<u8>, values: [f32; 4]) {
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
}

fn normalize(vector: [f32; 3]) -> [f32; 3] {
    let length = (vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]).sqrt();
    if length > 0.0 && length.is_finite() {
        [vector[0] / length, vector[1] / length, vector[2] / length]
    } else {
        [0.0, 0.0, 1.0]
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CASCADE_COUNT, SCENE_UNIFORM_BYTES, SHADOW_UNIFORM_BYTES, SKY_HORIZON, occlusion_size,
    };

    #[test]
    fn the_occlusion_estimate_is_half_resolution_rounded_up() {
        assert_eq!(occlusion_size(1920, 1200), (960, 600));
        // Rounded up, so an odd size keeps its last column and row. Rounding down instead leaves the far
        // edge of the frame with no estimate to upsample from, and the fallback there is the co-located
        // tap -- which is the unblurred noise, in a one-pixel stripe along two borders.
        assert_eq!(occlusion_size(721, 481), (361, 241));
        // Never zero, whatever it is handed. Target allocation refuses a zero dimension as an error, and
        // this must not be what turns that into a panic on the way there.
        assert_eq!(occlusion_size(1, 1), (1, 1));
        assert_eq!(occlusion_size(0, 0), (1, 1));
    }

    #[test]
    fn the_occlusion_shader_rounds_the_same_way_this_does() {
        // The figure exists on both sides of the language boundary and cannot be reconciled through the
        // uniform: `terrain_ao.wgsl` reads the scene block through a deliberately truncated prefix, so a
        // field appended for this would be unreachable there. Rounding down in the shader while rounding
        // up here would clamp every upsample tap one row short of the frame.
        let declared = crate::shader::chunk("terrain_ao").expect("terrain_ao chunk");
        assert!(
            declared.contains("(vec2<i32>(camera.viewport.xy) + vec2<i32>(1)) / vec2<i32>(2)"),
            "terrain_ao.wgsl no longer derives the half-resolution size by rounding up"
        );
    }

    #[test]
    fn uniform_block_sizes_match_the_shader_declarations() {
        // These are the sizes `SceneCamera` and `ShadowCamera` occupy in the WGSL. A mismatch does not
        // fail validation -- it silently misaligns every field past the drift -- so it is asserted here
        // as well as debug-asserted at upload.
        assert_eq!(SCENE_UNIFORM_BYTES, 400);
        assert_eq!(SHADOW_UNIFORM_BYTES, CASCADE_COUNT * 80);
    }

    #[test]
    fn the_occlusion_camera_is_a_prefix_of_the_scene_camera() {
        // `terrain_ao.wgsl` declares its own truncated copy of `SceneCamera` because its bind group is a
        // different one, and both bind the same buffer. That is sound only while its declaration is a
        // *prefix* of the full block, so a field inserted rather than appended above would misalign
        // everything after it there -- silently, since neither shader fails to validate.
        //
        // Checked as a substring rather than by parsing: the point is that the leading fields appear in
        // the same order with the same types, which is exactly what determines the offsets.
        let scene = crate::shader::chunk("scene").expect("scene chunk");
        let occlusion = crate::shader::chunk("terrain_ao").expect("terrain_ao chunk");
        for field in [
            "view_projection: mat4x4<f32>",
            "inverse_view_projection: mat4x4<f32>",
            "camera_position: vec4<f32>",
            "viewport: vec4<f32>",
            "lights: array<DirectionalLight, 3>",
        ] {
            assert!(scene.contains(field), "scene.wgsl lost {field}");
            assert!(occlusion.contains(field), "terrain_ao.wgsl lost {field}");
        }
        // The one field the occlusion pass must *not* have to know about, and the reason the order above
        // is load-bearing: it is appended after everything that pass declares.
        assert!(scene.contains("output: vec4<f32>"));
        assert!(!occlusion.contains("output: vec4<f32>"));
    }

    #[test]
    fn the_clear_horizon_colour_matches_the_shader_constant() {
        // `SKY_HORIZON` exists on both sides of the language boundary: the sky gradient needs it as a
        // WGSL constant, and the fog colour is derived from it here. Nothing but this test stops the two
        // drifting, and the symptom would be a fog bank a different colour from the sky it fades into --
        // a band along the horizon, exactly where it is most visible.
        let declared = crate::shader::chunk("atmosphere").expect("atmosphere chunk");
        let expected = format!(
            "vec3<f32>({:.2}, {:.2}, {:.2})",
            SKY_HORIZON[0], SKY_HORIZON[1], SKY_HORIZON[2]
        );
        assert!(
            declared.contains(&expected),
            "atmosphere.wgsl does not declare SKY_HORIZON as {expected}"
        );
    }
}
