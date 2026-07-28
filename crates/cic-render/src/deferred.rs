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
//! 7. composite      tone map and sharpen -> the caller's target
//! ```
//!
//! Water sits *inside* the HDR target rather than being composited over the finished image, so its
//! glitter tone maps with everything else. Over the composite it would clip to white instead of
//! rolling off, which is the whole reason the accumulation target has range above one.
//!
//! The G-buffer stores no world position. It is reconstructed from depth in step 5, for the reason
//! documented at `world_from_depth` in `terrain_deferred.wgsl`: a half-float position target
//! quantises to whole world units past 1024, which striped self-shadowing across the whole terrain in
//! a way no bias setting could reach.

use cic_camera::CameraPose;

use crate::RenderError;
use crate::gpu::{DEPTH_FORMAT, GpuContext};
use crate::model::{ModelBatch, buffer_layouts};
use crate::shadow::{CASCADE_COUNT, CASCADE_RESOLUTION, Cascade, fit_cascades};
use crate::terrain::{DirectionalLight, TerrainRenderer};
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

/// Lighting accumulates before tone mapping, so it needs range above one.
pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Byte size of the `SceneCamera` uniform block: two matrices, two vectors, three lights.
const SCENE_UNIFORM_BYTES: usize = 64 + 64 + 16 + 16 + 3 * 48;

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
    /// Viewport size in pixels.
    ///
    /// Carried on the frame rather than passed separately to [`DeferredRenderer::set_frame`], because
    /// it was previously supplied twice — once to build the projection and once to fill the shaders'
    /// viewport uniform — and nothing checked that the two agreed. They must: the lighting pass
    /// reconstructs a world position using the viewport's reciprocal, so a disagreement moves every
    /// receiver and reads as a shadowing fault rather than as a wrong number.
    pub viewport: [u32; 2],
    /// Scene time in seconds, which drives every animated surface.
    ///
    /// A frame *parameter* rather than a clock reading taken inside the renderer, and that is a
    /// deliberate constraint rather than a convenience. A capture of an animated scene is only a
    /// usable regression reference if the same inputs produce the same image, so the one thing that
    /// would make it irreproducible — the wall clock — is kept out of the renderer entirely. The
    /// caller running a window advances this; a test pins it.
    pub time: f32,
}

impl DeferredFrame {
    /// Builds a frame from a pose and viewport, with the default light and shadow distance, at time
    /// zero.
    #[must_use]
    pub fn new(pose: CameraPose, width: u32, height: u32) -> Self {
        Self {
            pose,
            projection: Projection::for_viewport(width, height),
            // Not `DirectionalLight::default()`: this chain computes occlusion, which is what makes a
            // realistic skylight ambient affordable. See `daylight_with_occlusion`.
            light: DirectionalLight::daylight_with_occlusion(),
            shadow_distance: DEFAULT_SHADOW_DISTANCE,
            viewport: [width, height],
            time: 0.0,
        }
    }

    /// Returns the frame with its scene time replaced.
    #[must_use]
    pub const fn at_time(mut self, time: f32) -> Self {
        self.time = time;
        self
    }
}

/// Every render target the chain writes.
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
    hdr: wgpu::TextureView,
    width: u32,
    height: u32,
}

impl DeferredTargets {
    /// Allocates every intermediate target for a viewport.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::EmptyCapture`] for a zero dimension.
    pub fn new(context: &GpuContext, width: u32, height: u32) -> Result<Self, RenderError> {
        if width == 0 || height == 0 {
            return Err(RenderError::EmptyCapture);
        }
        let device = context.device();
        let screen = |label: &str, format: wgpu::TextureFormat| {
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
            ao_raw: screen("cic-render ao raw", AO_FORMAT),
            ao_blurred: screen("cic-render ao blurred", AO_FORMAT),
            hdr: screen("cic-render hdr scene", HDR_FORMAT),
            width,
            height,
        })
    }

    /// Returns the viewport these targets were sized for.
    #[must_use]
    pub const fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

/// The deferred pipelines and their uniforms.
#[derive(Debug)]
pub struct DeferredRenderer {
    shadow_pipeline: wgpu::RenderPipeline,
    gbuffer_pipeline: wgpu::RenderPipeline,
    model_shadow_pipeline: wgpu::RenderPipeline,
    model_gbuffer_pipeline: wgpu::RenderPipeline,
    material_layout: wgpu::BindGroupLayout,
    water_pipeline: wgpu::RenderPipeline,
    water_layout: wgpu::BindGroupLayout,
    ao: AoStage,
    lighting: LightingStage,
    scene_uniform: wgpu::Buffer,
    shadow_uniform: wgpu::Buffer,
    cascade_uniforms: Vec<wgpu::Buffer>,
    cascade_groups: Vec<wgpu::BindGroup>,
}

impl DeferredRenderer {
    /// Builds every pipeline and bind group for one terrain and target set.
    ///
    /// `output_format` is the format of whatever the composite writes into — a capture target, or a
    /// surface's own format, which is commonly BGRA rather than RGBA. A pipeline built for the wrong
    /// one fails at creation rather than rendering something subtly wrong, which is why it is a
    /// parameter and not a constant.
    ///
    /// # Errors
    ///
    /// Currently infallible, but returns `Result` so adding a fallible resource later is not a
    /// breaking change.
    pub fn new(
        context: &GpuContext,
        terrain: &TerrainRenderer,
        targets: &DeferredTargets,
        output_format: wgpu::TextureFormat,
    ) -> Result<Self, RenderError> {
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

        let (cascade_layout, cascade_uniforms, cascade_groups) = build_cascade_bindings(device);
        let material_layout = ModelBatch::material_layout(device);
        let water_layout = WaterBody::layout(device);
        // The lighting layout comes back out rather than staying inside the stage, because the water
        // pass binds that very group: it needs the camera, the fitted cascades, the shadow array, and
        // the scene depth, all of which are already declared there. Building a second identical layout
        // for it would be two declarations to keep in step by hand.
        let (lighting, lighting_layout) = build_lighting(
            device,
            targets,
            &scene_uniform,
            &shadow_uniform,
            &lighting_shader,
            &composite_shader,
            output_format,
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
            model_shadow_pipeline: build_model_shadow_pipeline(
                device,
                terrain.bind_group_layout(),
                &cascade_layout,
                &model_shader,
            ),
            model_gbuffer_pipeline: build_model_gbuffer_pipeline(
                device,
                terrain.bind_group_layout(),
                &material_layout,
                &model_shader,
            ),
            material_layout,
            water_pipeline: build_water_pipeline(
                device,
                &lighting_layout,
                &water_layout,
                &water_shader,
            ),
            water_layout,
            ao: build_ao(device, targets, &scene_uniform, &ao_shader),
            lighting,
            scene_uniform,
            shadow_uniform,
            cascade_uniforms,
            cascade_groups,
        })
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
        let view_projection = multiply(perspective(frame.projection), view);
        let inverse = invert(view_projection).ok_or(RenderError::SingularCamera)?;

        // The terrain pipelines read the terrain uniform's own view-projection, so it has to agree
        // with the one the lighting pass inverts or the reconstruction lands somewhere else entirely.
        terrain.set_frame(context, &view_projection, frame.pose.eye, frame.light);

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
            &scene_bytes(&view_projection, &inverse, frame),
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

        Ok(cascades)
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

        // 1. Shadow cascades, depth only.
        for (layer, group) in targets.shadow_layers.iter().zip(&self.cascade_groups) {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cic-render shadow cascade"),
                color_attachments: &[],
                depth_stencil_attachment: Some(depth_attachment(layer)),
                multiview_mask: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.shadow_pipeline);
            pass.set_bind_group(0, terrain.bind_group(), &[]);
            pass.set_bind_group(1, group, &[]);
            pass.draw(0..terrain.vertex_count(), 0..1);

            // Models cast into the same cascade. Without this a model is lit as though it were
            // present but throws no shadow, which reads as the model floating.
            if !models.is_empty() {
                pass.set_pipeline(&self.model_shadow_pipeline);
                pass.set_bind_group(0, terrain.bind_group(), &[]);
                pass.set_bind_group(1, group, &[]);
                for batch in models {
                    batch.draw(&mut pass, None);
                }
            }
        }

        // 2. G-buffer. Coverage clears to zero, which the lighting pass reads as "sky".
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cic-render gbuffer"),
                color_attachments: &[
                    Some(clear_attachment(&targets.albedo)),
                    Some(clear_attachment(&targets.normal)),
                    Some(clear_attachment(&targets.coverage)),
                ],
                depth_stencil_attachment: Some(depth_attachment(&targets.depth)),
                multiview_mask: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.gbuffer_pipeline);
            pass.set_bind_group(0, terrain.bind_group(), &[]);
            pass.draw(0..terrain.vertex_count(), 0..1);

            if !models.is_empty() {
                pass.set_pipeline(&self.model_gbuffer_pipeline);
                pass.set_bind_group(0, terrain.bind_group(), &[]);
                for batch in models {
                    batch.draw(&mut pass, Some(2));
                }
            }
        }

        // 3 and 4. Occlusion, then its bilateral blur.
        fullscreen_pass(
            &mut encoder,
            "cic-render ao",
            &targets.ao_raw,
            &self.ao.pipeline,
            &[&self.ao.group],
        );
        fullscreen_pass(
            &mut encoder,
            "cic-render ao blur",
            &targets.ao_blurred,
            &self.ao.blur_pipeline,
            &[&self.ao.group, &self.ao.source_group],
        );

        // 5. Lighting into HDR.
        fullscreen_pass(
            &mut encoder,
            "cic-render lighting",
            &targets.hdr,
            &self.lighting.pipeline,
            &[&self.lighting.group],
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
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.water_pipeline);
            pass.set_bind_group(0, &self.lighting.group, &[]);
            for body in water {
                body.draw(&mut pass);
            }
        }

        // 7. Tone map and sharpen into the output.
        fullscreen_pass(
            &mut encoder,
            "cic-render composite",
            output,
            &self.lighting.composite_pipeline,
            &[&self.lighting.group, &self.lighting.composite_group],
        );

        context.queue().submit([encoder.finish()]);
    }
}

/// Packs the `SceneCamera` uniform block.
fn scene_bytes(
    view_projection: &[[f32; 4]; 4],
    inverse: &[[f32; 4]; 4],
    frame: DeferredFrame,
) -> Vec<u8> {
    let [width, height] = frame.viewport;
    let mut scene = Vec::with_capacity(SCENE_UNIFORM_BYTES);
    push_matrix(&mut scene, view_projection);
    push_matrix(&mut scene, inverse);
    let [ex, ey, ez] = frame.pose.eye;
    push_vec4(&mut scene, [ex, ey, ez, 0.0]);
    // Both the viewport and its reciprocal, so the shaders never divide per pixel. Dimensions are
    // bounded by the capture limits, far inside exact f32 range.
    #[allow(clippy::cast_precision_loss)]
    let (viewport_x, viewport_y) = (width as f32, height as f32);
    push_vec4(
        &mut scene,
        [
            viewport_x,
            viewport_y,
            1.0 / viewport_x.max(1.0),
            1.0 / viewport_y.max(1.0),
        ],
    );
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
    debug_assert_eq!(scene.len(), SCENE_UNIFORM_BYTES, "scene uniform drifted");
    scene
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
            ],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Builds the instanced model G-buffer pipeline.
///
/// Group 1 is left empty: the shader declares the shadow cascade there, and this entry point does not
/// use it. The pipeline layout takes an optional layout per slot for exactly this case.
fn build_model_gbuffer_pipeline(
    device: &wgpu::Device,
    terrain_layout: &wgpu::BindGroupLayout,
    material_layout: &wgpu::BindGroupLayout,
    model_shader: &wgpu::ShaderModule,
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
            // Back faces are culled, unlike terrain. A model is a closed solid, so its back faces are
            // its interior; drawing them wastes fill and can win the depth test at grazing angles.
            cull_mode: Some(wgpu::Face::Back),
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
            entry_point: Some("gbuffer_fragment"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[
                Some(colour_target(ALBEDO_FORMAT)),
                Some(colour_target(NORMAL_FORMAT)),
                Some(colour_target(COVERAGE_FORMAT)),
            ],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Builds the instanced model depth-only shadow pipeline.
fn build_model_shadow_pipeline(
    device: &wgpu::Device,
    terrain_layout: &wgpu::BindGroupLayout,
    cascade_layout: &wgpu::BindGroupLayout,
    model_shader: &wgpu::ShaderModule,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("cic-render model shadow pipeline layout"),
        bind_group_layouts: &[Some(terrain_layout), Some(cascade_layout)],
        immediate_size: 0,
    });
    let buffers = buffer_layouts();
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("cic-render model shadow pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: model_shader,
            entry_point: Some("shadow_vertex"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &buffers,
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            // Front faces are culled here, not back. Recording only the far side of a solid moves the
            // stored depth away from the receiver, which removes self-shadowing acne at its source
            // rather than biasing it away afterwards.
            cull_mode: Some(wgpu::Face::Front),
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
        fragment: None,
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
            &[&layout],
            AO_FORMAT,
        ),
        blur_pipeline: fullscreen_pipeline(
            device,
            "cic-render ao blur pipeline",
            ao_shader,
            "ao_blur_fragment",
            &[&layout, &source_layout],
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
}

/// Builds the lighting and composite stage, returning its bind group layout as well.
///
/// The layout is returned because the water pass binds this same group and would otherwise need an
/// identical second declaration.
fn build_lighting(
    device: &wgpu::Device,
    targets: &DeferredTargets,
    scene_uniform: &wgpu::Buffer,
    shadow_uniform: &wgpu::Buffer,
    lighting_shader: &wgpu::ShaderModule,
    composite_shader: &wgpu::ShaderModule,
    output_format: wgpu::TextureFormat,
) -> (LightingStage, wgpu::BindGroupLayout) {
    let comparison_sampler = build_shadow_sampler(device);
    let scene_sampler = build_scene_sampler(device);

    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cic-render lighting layout"),
        entries: &[
            texture_entry(0, wgpu::TextureSampleType::Float { filterable: false }),
            texture_entry(1, wgpu::TextureSampleType::Float { filterable: false }),
            texture_entry(2, wgpu::TextureSampleType::Float { filterable: false }),
            // Visible to the vertex stage as well, for the water pass: its vertex shader transforms by
            // this block's view-projection. The two fullscreen entry points using the same group do
            // not read it there, and extra visibility costs them nothing.
            uniform_entry(3, wgpu::ShaderStages::VERTEX_FRAGMENT),
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 5,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                count: None,
            },
            uniform_entry(6, wgpu::ShaderStages::FRAGMENT),
            texture_entry(7, wgpu::TextureSampleType::Float { filterable: false }),
            texture_entry(8, wgpu::TextureSampleType::Depth),
        ],
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("cic-render lighting bindings"),
        layout: &layout,
        entries: &[
            view_entry(0, &targets.albedo),
            view_entry(1, &targets.normal),
            view_entry(2, &targets.coverage),
            buffer_entry(3, scene_uniform),
            view_entry(4, &targets.shadow_array),
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::Sampler(&comparison_sampler),
            },
            buffer_entry(6, shadow_uniform),
            view_entry(7, &targets.ao_blurred),
            view_entry(8, &targets.depth),
        ],
    });

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
            &[&layout],
            HDR_FORMAT,
        ),
        composite_pipeline: fullscreen_pipeline(
            device,
            "cic-render composite pipeline",
            composite_shader,
            "composite_fragment",
            &[&layout, &composite_layout],
            output_format,
        ),
        group,
        composite_group,
    };
    (stage, layout)
}

/// Builds the water pipeline.
///
/// Not a [`fullscreen_pipeline`]: water has its own procedural grid rather than one covering triangle,
/// and it blends rather than overwriting.
fn build_water_pipeline(
    device: &wgpu::Device,
    lighting_layout: &wgpu::BindGroupLayout,
    water_layout: &wgpu::BindGroupLayout,
    water_shader: &wgpu::ShaderModule,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("cic-render water pipeline layout"),
        // Contiguous, with no empty slot. Water reuses the lighting group because it needs the camera,
        // the cascades and the scene depth that are already declared there; its own uniform follows in
        // group 1. It declares only the group-0 bindings it reads, and a layout carrying more than a
        // shader uses is allowed.
        bind_group_layouts: &[Some(lighting_layout), Some(water_layout)],
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
    groups: &[&wgpu::BindGroup],
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(clear_attachment(target))],
        depth_stencil_attachment: None,
        multiview_mask: None,
        timestamp_writes: None,
        occlusion_query_set: None,
    });
    pass.set_pipeline(pipeline);
    for (index, group) in groups.iter().enumerate() {
        pass.set_bind_group(u32::try_from(index).unwrap_or(0), *group, &[]);
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
    layouts: &[&wgpu::BindGroupLayout],
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let owned: Vec<Option<&wgpu::BindGroupLayout>> =
        layouts.iter().map(|layout| Some(*layout)).collect();
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &owned,
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
    use super::{CASCADE_COUNT, SCENE_UNIFORM_BYTES, SHADOW_UNIFORM_BYTES};

    #[test]
    fn uniform_block_sizes_match_the_shader_declarations() {
        // These are the sizes `SceneCamera` and `ShadowCamera` occupy in the WGSL. A mismatch does not
        // fail validation -- it silently misaligns every field past the drift -- so it is asserted here
        // as well as debug-asserted at upload.
        assert_eq!(SCENE_UNIFORM_BYTES, 304);
        assert_eq!(SHADOW_UNIFORM_BYTES, CASCADE_COUNT * 80);
    }
}
