// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Interactive free-flight presentation for immutable staged terrain.
//!
//! Bounded MAP `GlobalLighting` supplies selected source-authored terrain lights; the original
//! project preview remains only as an explicit fallback for maps without that chunk. A fixed-page
//! cache composes nested 16/32-texel screen-space detail on the GPU over the stable 8-texel
//! background. Camera motion changes only residency metadata; it never launches CPU texture bakes.
//! Road texture mip count follows `W3DRoadBuffer.cpp` in `GeneralsGameCode` revision
//! `9f7abb866f5afd446db14149979e744c7216baaf`, licensed under GPL-3.0-or-later with Electronic
//! Arts Section 7 terms. Polygon-line diagnostics and depth bias are project-authored modern GPU
//! policy; see `docs/provenance/map.md`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, OwnedDisplayHandle};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use cic_camera::{CameraIntent, CameraPose, GroundHeight, RtsCamera, RtsCameraProfile};

use crate::model::BlendMode;
use crate::terrain::{TerrainDetailRequest, TerrainMipLevel, generate_srgb_mips};
use crate::terrain_virtual::{
    VIRTUAL_PAGE_BORDER, VIRTUAL_PAGE_EXTENT, VIRTUAL_PAGE_INTERIOR, VIRTUAL_PAGE_LAYERS,
    VIRTUAL_PAGE_MIPS, VirtualPageCache, VirtualPageJob, VirtualPageView,
};
use crate::viewer::{
    GpuResourceManager, ViewerError, create_depth, create_material_layout, nonzero_size,
};
use crate::{
    Capture, MapPresentationFrame, RenderError, StagedBoundaryFence, StagedMapOverlays,
    StagedRoads, StagedStaticScenery, StagedTerrain, StagedWater, TerrainLighting, WaterAppearance,
    WaterPresentationPolicy, capture_layout, read_back_capture,
};

const WINDOW_WIDTH: u32 = 1_280;
const WINDOW_HEIGHT: u32 = 800;
const CAMERA_UNIFORM_BYTES: u64 = 368;
/// Pixels of pointer travel, summed over the whole hold, a rotate hold may stay within and still
/// count as a click.
const ROTATE_CLICK_SLOP_PIXELS: f32 = 6.0;
/// Longest a middle-button hold may last and still count as a click rather than a rotation.
///
/// Travel alone cannot separate the two, because lining up a rotation begins with the cursor held
/// still: a hold that has not moved far yet is far more likely to be a rotation in progress than a
/// request to face north. Only a brisk tap gets the reset.
const ROTATE_CLICK_MAX_HOLD: Duration = Duration::from_millis(180);
/// Cursor offset from the scroll anchor, in pixels, at which the scroll request reaches full rate.
const SCROLL_ANCHOR_FULL_PIXELS: f32 = 90.0;
/// Offset below which an anchored scroll stays still, so a press that barely moves does not creep.
const SCROLL_ANCHOR_DEAD_ZONE_PIXELS: f32 = 6.0;

const SHADOW_CASCADE_COUNT: usize = 5;
/// `SHADOW_CASCADE_COUNT` as the array-layer count the texture and its views need. Must match.
const SHADOW_CASCADE_LAYERS: u32 = 5;
/// One `mat4x4` plus one `vec4` of parameters per cascade.
const SHADOW_CASCADE_BYTES: u64 = 80;
/// `SHADOW_CASCADE_BYTES * SHADOW_CASCADE_COUNT`, spelled as a `usize` so the packing routine can
/// return a fixed-size array. A unit test pins it against the two constants it derives from.
const SHADOW_UNIFORM_LEN: usize = 400;
const SHADOW_UNIFORM_BYTES: u64 = SHADOW_UNIFORM_LEN as u64;
/// Fractions of the shadowed view distance where each cascade ends: an even blend of a logarithmic
/// and a uniform split.
///
/// The distribution matters more than it looks, for two reasons pulling in opposite directions.
/// Because a cascade is fitted to the bounding volume of its frustum slice, its texel extent is
/// proportional to where it *ends*, at approximately `far / 823` for this field of view and cascade
/// resolution — so leaving most of the range to the outermost cascade makes that cascade coarser
/// than a single whole-map slice would have been. But a purely logarithmic split fails the other
/// way here: this camera sits well above the terrain, so the nearest *visible* ground is hundreds
/// of units out, and front-loaded cascades land on almost nothing while the whole screen falls to
/// the two coarsest. The blended split keeps density stepping by roughly a factor of two per
/// cascade across the range that actually contains visible ground.
const SHADOW_CASCADE_SPLITS: [f32; SHADOW_CASCADE_COUNT] = [0.10, 0.21, 0.33, 0.51, 1.0];
/// Cap on how far cascades chase the view, and the dominant control over outermost-cascade
/// quality: the last cascade lands near `SHADOW_MAX_DISTANCE / 823` world units per texel. Raise it
/// for more shadowed distance at the cost of blockier far shadows; lower it to sharpen them at the
/// cost of distant geometry reading as unshadowed, which is a far softer failure than blocky.
const SHADOW_MAX_DISTANCE: f32 = 1_600.0;
/// Extra world units each cascade's light camera is pulled back along the light, so that casters
/// standing above the cascade's receiver region are still inside its depth range.
///
/// Without it a cascade only reaches about its own radius above the ground it covers, and the near
/// cascade's radius is a few tens of units while a structure is well over a hundred tall — so tall
/// objects get clipped out of exactly the cascade a nearby receiver selects, and lose their shadow
/// when the camera approaches. This costs no resolution: an orthographic depth range does not
/// affect texel density, and the world-space bias is unaffected because it is derived from
/// `texel_world` and converted through `depth_range` at sample time.
const SHADOW_CASTER_HEADROOM: f32 = 600.0;
/// First cascade eligible for reuse across frames. Every caster in the MAP scene is static, so a
/// cascade whose fitted matrix is unchanged since the previous frame already holds the correct
/// depth and its pass can be skipped entirely. Texel snapping makes this common: a cascade's matrix
/// only changes when its snapped centre crosses a texel, and the outer cascades have the largest
/// texels, so they are both the most expensive to redraw and the least likely to need it.
///
/// The inner cascades are deliberately excluded. Tree sway is the one thing that animates, and it
/// is not part of the matrix, so a reused cascade freezes the sway in its shadows. In the outer
/// cascades that motion is close to a texel and reads as still; in the inner ones it would be
/// obvious. Raise this to 3 if frozen sway is visible at mid distance.
const SHADOW_CACHED_CASCADE_START: usize = 2;
/// Occlusion is a single visibility scalar, so one 8-bit channel is enough and keeps the extra
/// full-resolution target plus its blur ping-pong cheap.
const AO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;
// Per-cascade extent. Five 3072-square layers cost 180 MiB as `Depth32Float`, which is the price of
// shadows reaching far enough not to cut off visibly while the outermost cascade stays near the
// density a single whole-map slice managed. Reuse keeps the bandwidth cost far below the memory
// cost, since the outer cascades redraw only when their fit moves. Each cascade derives its own bias
// from its own `texel_world`, so the near cascade is biased far more tightly than the far one.
const SHADOW_MAP_EXTENT: u32 = 3_072;
/// Hardware MSAA sample count for the opaque G-buffer geometry pass.
const GBUFFER_SAMPLE_COUNT: u32 = 4;
/// Third G-buffer target: geometry coverage, carrying emissive strength above 1.0. One channel is
/// all it needs, and a half float resolves emissive strengths far finer than a material declares.
const GBUFFER_COVERAGE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;
/// Resolved scene depth, kept as a colour target as well as a depth attachment so the deferred and
/// forward passes can sample it. Full float because it is what world position is reconstructed from.
const SCENE_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
const MAX_FRAME_SECONDS: f32 = 0.1;
const CAMERA_VERTICAL_FOV: f32 = std::f32::consts::PI / 3.0;
const TERRAIN_CELL_WORLD_SIZE: f32 = 10.0;
const DETAIL_SCREEN_OVERSAMPLE: f32 = 1.75;
const DETAIL_FADE_START_RATIO: f32 = 0.78;
const SOURCE_ROAD_MIP_LEVELS: usize = 3;
const ROAD_DEPTH_BIAS: wgpu::DepthBiasState = wgpu::DepthBiasState {
    constant: -2,
    slope_scale: -1.0,
    clamp: 0.0,
};

/// One immutable staged MAP scene, complete enough to present or capture.
///
/// Bundled rather than passed as loose arguments because the interactive viewer and the
/// deterministic capture take exactly the same scene, and a single type is what keeps them unable
/// to drift apart.
#[derive(Debug)]
pub struct MapScene {
    pub terrain: StagedTerrain,
    pub roads: StagedRoads,
    pub boundary: StagedBoundaryFence,
    pub overlays: StagedMapOverlays,
    pub scenery: StagedStaticScenery,
    pub water: StagedWater,
    pub water_appearance: WaterAppearance,
    pub lighting: TerrainLighting,
}

impl MapScene {
    /// Builds a terrain-and-water-only scene, leaving every MAP overlay empty.
    #[must_use]
    pub fn terrain_only(
        terrain: StagedTerrain,
        water: StagedWater,
        water_appearance: WaterAppearance,
        lighting: TerrainLighting,
    ) -> Self {
        Self {
            terrain,
            roads: StagedRoads::empty(),
            boundary: StagedBoundaryFence::empty(),
            overlays: StagedMapOverlays::empty(),
            scenery: StagedStaticScenery::empty(),
            water,
            water_appearance,
            lighting,
        }
    }

    fn staged<'a>(
        &'a self,
        requests: &'a [TerrainDetailRequest],
        page_view: VirtualPageView,
    ) -> TerrainViewerScene<'a> {
        TerrainViewerScene {
            terrain: &self.terrain,
            roads: &self.roads,
            boundary: &self.boundary,
            overlays: &self.overlays,
            scenery: &self.scenery,
            requests,
            page_view,
            water: &self.water,
            water_appearance: &self.water_appearance,
            lighting: self.lighting,
        }
    }
}

/// Explicit placement for the MAP scene camera.
///
/// The interactive viewer treats it as the starting pose and the capture treats it as the only
/// pose, so a capture frames the scene exactly as the window would before any input arrives. Pitch
/// is deliberately absent: the source real-time-strategy camera has a fixed tilt and the viewer
/// offers no way to change it, so a capture that could would no longer be showing what `map-view`
/// shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapViewCamera {
    /// Ground point the view is centred on, or `None` for the centre of the terrain bounds.
    focus_xy: Option<[f32; 2]>,
    yaw: f32,
    height: f32,
}

impl MapViewCamera {
    /// The pose `map-view` opens on: centred on the terrain, at the source default yaw and height.
    pub const CENTERED: Self = Self {
        focus_xy: None,
        yaw: RtsCameraProfile::GENERALS_DEFAULT.yaw,
        height: RtsCameraProfile::GENERALS_DEFAULT.height,
    };

    /// Returns a placement at an explicit yaw in radians and height in world units.
    ///
    /// The height is clamped by the camera profile's own limits, exactly as a zoom would be, so a
    /// capture cannot be framed from somewhere the viewer could not reach.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::InvalidCameraPlacement`] for a non-finite yaw or height.
    pub fn new(yaw: f32, height: f32) -> Result<Self, RenderError> {
        if !yaw.is_finite() || !height.is_finite() {
            return Err(RenderError::InvalidCameraPlacement);
        }
        Ok(Self {
            focus_xy: None,
            yaw,
            height,
        })
    }

    /// Centres the view on an explicit ground point instead of the terrain centre.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::InvalidCameraPlacement`] for a non-finite coordinate.
    pub fn with_focus(mut self, focus_xy: [f32; 2]) -> Result<Self, RenderError> {
        if !focus_xy.iter().all(|value| value.is_finite()) {
            return Err(RenderError::InvalidCameraPlacement);
        }
        self.focus_xy = Some(focus_xy);
        Ok(self)
    }

    #[must_use]
    pub const fn yaw(self) -> f32 {
        self.yaw
    }

    #[must_use]
    pub const fn height(self) -> f32 {
        self.height
    }

    #[must_use]
    pub const fn focus_xy(self) -> Option<[f32; 2]> {
        self.focus_xy
    }

    /// Resolves the placement into the camera model the viewer drives interactively.
    fn rts_camera(self, terrain: &StagedTerrain) -> RtsCamera {
        let focus = self.focus_xy.unwrap_or_else(|| {
            let (minimum, maximum) = terrain.bounds();
            [
                (minimum[0] + maximum[0]) * 0.5,
                (minimum[1] + maximum[1]) * 0.5,
            ]
        });
        let profile = RtsCameraProfile {
            yaw: self.yaw,
            height: self.height,
            ..RtsCameraProfile::GENERALS_DEFAULT
        };
        RtsCamera::new(profile, focus, &StagedGround(terrain))
    }

    /// Resolves the placement into the renderer's view pose.
    fn resolve(self, terrain: &StagedTerrain) -> TerrainCamera {
        TerrainCamera::from_pose(
            self.rts_camera(terrain).pose(),
            TerrainCamera::far_plane_for(terrain),
        )
    }
}

impl Default for MapViewCamera {
    fn default() -> Self {
        Self::CENTERED
    }
}

/// Which optional lighting contributions a frame records.
///
/// Skipping one leaves its target at the neutral clear value the pass already writes — a shadow
/// cascade cleared to the far plane shadows nothing, an occlusion target cleared to white occludes
/// nothing — so a term can be isolated without a shader variant or a uniform flag. Differencing two
/// captures that differ in one flag attributes a suspicious region to a specific term, which is the
/// only way to tell coarse terrain self-shadowing from an occlusion artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapViewPasses {
    /// Whether casters are drawn into the shadow cascades.
    pub shadows: bool,
    /// Whether the ambient occlusion pass and its blur run.
    pub occlusion: bool,
}

impl MapViewPasses {
    /// Everything on, as the interactive viewer always presents it.
    pub const ALL: Self = Self {
        shadows: true,
        occlusion: true,
    };
}

impl Default for MapViewPasses {
    fn default() -> Self {
        Self::ALL
    }
}

/// Presents a staged MAP scene in an interactive window.
///
/// WASD or the arrow keys scroll, the wheel zooms, the middle button rotates and a middle click
/// faces the camera back to its starting yaw, R resets, M toggles wireframe when the adapter
/// supports it, and Escape closes. A `frame` freezes water and ambient presentation at an explicit
/// diagnostic time; camera motion and detail streaming stay live either way.
///
/// # Errors
///
/// Returns a structured window, surface, adapter, device, shader, or terrain-resource failure.
pub fn run_map_view(
    scene: MapScene,
    camera: MapViewCamera,
    title: String,
    frame: Option<MapPresentationFrame>,
) -> Result<(), ViewerError> {
    let event_loop = EventLoop::new().map_err(ViewerError::EventLoop)?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let display = event_loop.owned_display_handle();
    let mut application = TerrainViewerApplication::new(scene, camera, title, display, frame)?;
    event_loop
        .run_app(&mut application)
        .map_err(ViewerError::EventLoop)?;
    application.error.map_or(Ok(()), Err)
}

/// Renders one frame of a staged MAP scene offscreen, through the same GPU path `run_map_view`
/// presents: the shadow cascade passes, the multisampled G-buffer, ambient occlusion, deferred
/// lighting, the composite, and the forward diagnostics and water passes over it.
///
/// Every input is explicit — target size, camera placement, and presentation time — and nothing in
/// the path consults a clock or an RNG, so identical inputs produce an identical image on a given
/// adapter.
///
/// # Errors
///
/// Returns a structured capture-dimension, adapter-resource, submission, or readback failure.
pub(crate) fn capture_map_view(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    [width, height]: [u32; 2],
    scene: &MapScene,
    camera: MapViewCamera,
    frame: MapPresentationFrame,
    passes: MapViewPasses,
) -> Result<Capture, ViewerError> {
    let (unpadded_row, padded_row, buffer_size) = capture_layout(width, height)?;
    let size = nonzero_size(PhysicalSize::new(width, height));
    let terrain_camera = camera.resolve(&scene.terrain);
    let viewport = [size.width, size.height];
    let requests = terrain_camera.detail_requests(&scene.terrain, viewport)?;
    let page_view = terrain_camera.virtual_page_view(&scene.terrain, viewport);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("cic-render MAP scene capture target"),
        size: wgpu::Extent3d {
            width: size.width,
            height: size.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: crate::SCENE_CAPTURE_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cic-render MAP scene capture readback"),
        size: buffer_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut gpu = TerrainViewerGpu::offscreen(
        device,
        queue,
        target.clone(),
        &scene.staged(&requests, page_view),
    )?;
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = gpu.encode_frame(terrain_camera, frame.seconds(), false, passes, &view)?;
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_row),
                rows_per_image: Some(size.height),
            },
        },
        target.size(),
    );
    let capture = read_back_capture(
        device,
        queue,
        encoder,
        &readback,
        size.width,
        size.height,
        unpadded_row,
        padded_row,
    )?;
    // `gpu` owns every render attachment the recorded passes write, and drops only here, after the
    // readback above has submitted and waited.
    Ok(capture)
}

#[allow(clippy::struct_excessive_bools)]
struct TerrainViewerApplication {
    scene: Arc<MapScene>,
    title: String,
    display: OwnedDisplayHandle,
    window: Option<Arc<Window>>,
    gpu: Option<TerrainViewerGpu>,
    /// Owns the movement model; the renderer never sees it directly.
    rts_camera: RtsCamera,
    /// Pose derived from `rts_camera` each frame, for the renderer and detail selection.
    camera: TerrainCamera,
    detail_requests: Vec<TerrainDetailRequest>,
    input: TerrainInput,
    /// When the current middle-button hold began, if one is active. Middle-button rotate, as the
    /// original game does it; the press time is kept because a release only faces the camera back
    /// to its starting yaw when the hold was a brief tap.
    rotate_press: Option<Instant>,
    /// Pointer travel summed over the current middle-button hold, in pixels.
    rotate_travel: f32,
    /// Cursor position where a right-button scroll was anchored, if one is active.
    ///
    /// The original scrolls from an anchor rather than dragging the map: pressing plants an anchor,
    /// and the cursor's offset from it is a continuous scroll velocity, so holding the cursor still
    /// away from the anchor keeps scrolling. `DrawScrollAnchor` and `MoveScrollAnchor` in the source
    /// options exist for exactly this gesture.
    scroll_anchor: Option<PhysicalPosition<f64>>,
    scroll_pressed: bool,
    /// Rotation and zoom arrive as discrete events but are consumed once per frame, so they
    /// accumulate here instead of being applied the moment they arrive.
    pending_rotate: f32,
    pending_zoom: f32,
    reset_camera: bool,
    reset_rotation: bool,
    cursor: Option<PhysicalPosition<f64>>,
    previous_frame: Instant,
    presentation_seconds: f32,
    fixed_frame: Option<MapPresentationFrame>,
    wireframe: bool,
    error: Option<ViewerError>,
}

impl TerrainViewerApplication {
    fn new(
        scene: MapScene,
        placement: MapViewCamera,
        title: String,
        display: OwnedDisplayHandle,
        fixed_frame: Option<MapPresentationFrame>,
    ) -> Result<Self, ViewerError> {
        let scene = Arc::new(scene);
        let rts_camera = placement.rts_camera(&scene.terrain);
        let camera = placement.resolve(&scene.terrain);
        let detail_requests =
            camera.detail_requests(&scene.terrain, [WINDOW_WIDTH, WINDOW_HEIGHT])?;
        Ok(Self {
            scene,
            title,
            display,
            window: None,
            gpu: None,
            rts_camera,
            camera,
            detail_requests,
            input: TerrainInput::default(),
            rotate_press: None,
            rotate_travel: 0.0,
            scroll_anchor: None,
            scroll_pressed: false,
            pending_rotate: 0.0,
            pending_zoom: 0.0,
            reset_camera: false,
            reset_rotation: false,
            cursor: None,
            previous_frame: Instant::now(),
            presentation_seconds: fixed_frame.map_or(0.0, MapPresentationFrame::seconds),
            fixed_frame,
            wireframe: false,
            error: None,
        })
    }

    fn window_title(&self, wireframe_available: bool) -> String {
        terrain_viewer_title(&self.title, self.wireframe, wireframe_available)
    }

    fn initialize(&mut self, event_loop: &ActiveEventLoop) -> Result<(), ViewerError> {
        let attributes = Window::default_attributes()
            .with_title(self.window_title(true))
            .with_inner_size(LogicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT));
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .map_err(ViewerError::Window)?,
        );
        let size = nonzero_size(window.inner_size());
        let gpu = pollster::block_on(TerrainViewerGpu::new(
            window.clone(),
            self.display.clone(),
            &self.scene.staged(
                &self.detail_requests,
                self.camera
                    .virtual_page_view(&self.scene.terrain, [size.width, size.height]),
            ),
        ))?;
        window.set_title(&self.window_title(gpu.wireframe_available()));
        self.window = Some(window);
        self.gpu = Some(gpu);
        self.previous_frame = Instant::now();
        Ok(())
    }

    /// Middle button rotates and, on a brief click that stayed put, faces the camera back to its
    /// starting yaw. Right button drags the view, which is the original's other way to scroll.
    fn mouse_button(&mut self, state: ElementState, button: MouseButton) {
        let pressed = state == ElementState::Pressed;
        match button {
            MouseButton::Middle => {
                if pressed {
                    self.rotate_press = Some(Instant::now());
                    self.rotate_travel = 0.0;
                } else if let Some(press) = self.rotate_press.take()
                    && rotate_release_is_click(press.elapsed(), self.rotate_travel)
                {
                    self.reset_rotation = true;
                }
            }
            MouseButton::Right => {
                // Plant the anchor where the press landed and keep it until release. `self.cursor`
                // is cleared below, so seed the anchor from the press position on the next move.
                self.scroll_anchor = None;
                self.scroll_pressed = pressed;
            }
            _ => return,
        }
        self.cursor = None;
    }

    /// Scroll request from the anchor offset, as a unit-capped direction and magnitude.
    ///
    /// Velocity rather than displacement: the offset from the anchor is sustained every frame while
    /// the button is held, so holding the cursor still away from the anchor keeps scrolling.
    /// [`anchor_scroll_request`] shapes the offset into a rate.
    fn scroll_request(&self) -> [f32; 2] {
        let (Some(anchor), Some(cursor)) = (self.scroll_anchor, self.cursor) else {
            return [0.0; 2];
        };
        #[allow(clippy::cast_possible_truncation)]
        let offset = [(cursor.x - anchor.x) as f32, (cursor.y - anchor.y) as f32];
        anchor_scroll_request(offset)
    }

    fn cursor_moved(&mut self, position: PhysicalPosition<f64>) {
        if let Some(previous) = self.cursor {
            #[allow(clippy::cast_possible_truncation)]
            let motion = [
                (position.x - previous.x) as f32,
                (position.y - previous.y) as f32,
            ];
            if self.rotate_press.is_some() {
                // Summed over the hold, not tested per event: a deliberate rotation arrives as many
                // small steps and no single one of them clears the click slop.
                self.rotate_travel += motion[0].abs() + motion[1].abs();
                self.pending_rotate -= motion[0];
            }
        }
        if self.scroll_pressed && self.scroll_anchor.is_none() {
            self.scroll_anchor = Some(position);
        }
        self.cursor = Some(position);
    }

    /// Feeds one frame of accumulated input to the camera model and republishes the pose.
    ///
    /// Rotation and zoom arrive as discrete events that can fire several times between frames, so
    /// they are accumulated and consumed here rather than applied on arrival; that keeps a fast
    /// scroll wheel from outrunning the frame rate.
    fn advance_camera(&mut self, seconds: f32) {
        let intent = CameraIntent {
            pan: self.input.pan(),
            drag: self.scroll_request(),
            zoom: self.pending_zoom,
            rotate: self.pending_rotate,
            reset: self.reset_camera,
            reset_rotation: self.reset_rotation,
        };
        self.pending_zoom = 0.0;
        self.pending_rotate = 0.0;
        self.reset_camera = false;
        self.reset_rotation = false;
        self.rts_camera
            .update(intent, seconds, &StagedGround(&self.scene.terrain));
        self.camera = TerrainCamera::from_pose(self.rts_camera.pose(), self.camera.far_plane);
    }

    fn refresh_detail(&mut self) -> Result<(), ViewerError> {
        if let Some(window) = &self.window {
            let size = nonzero_size(window.inner_size());
            let requests = self
                .camera
                .detail_requests(&self.scene.terrain, [size.width, size.height])?;
            if let Some(gpu) = &mut self.gpu {
                gpu.update_virtual_residency(
                    &requests,
                    self.camera
                        .virtual_page_view(&self.scene.terrain, [size.width, size.height]),
                );
            }
            self.detail_requests = requests;
        }
        Ok(())
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: ViewerError) {
        self.error = Some(error);
        event_loop.exit();
    }
}

impl ApplicationHandler for TerrainViewerApplication {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none()
            && let Err(error) = self.initialize(event_loop)
        {
            self.fail(event_loop, error);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self
            .window
            .as_ref()
            .is_none_or(|window| window.id() != window_id)
        {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.resize(size);
                }
            }
            WindowEvent::Focused(false) => {
                self.input = TerrainInput::default();
                self.rotate_press = None;
                self.rotate_travel = 0.0;
                self.scroll_anchor = None;
                self.pending_rotate = 0.0;
                self.pending_zoom = 0.0;
                self.cursor = None;
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                let pressed = event.state == ElementState::Pressed;
                self.input.set(code, pressed);
                if pressed && !event.repeat {
                    match code {
                        KeyCode::Escape => event_loop.exit(),
                        KeyCode::KeyR => self.reset_camera = true,
                        KeyCode::KeyM
                            if self
                                .gpu
                                .as_ref()
                                .is_some_and(TerrainViewerGpu::wireframe_available) =>
                        {
                            self.wireframe = !self.wireframe;
                            if let Some(window) = &self.window {
                                window.set_title(&self.window_title(true));
                            }
                        }
                        _ => {}
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => self.mouse_button(state, button),
            WindowEvent::CursorMoved { position, .. } => self.cursor_moved(position),
            WindowEvent::MouseWheel { delta, .. } => {
                let amount = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(position) => {
                        #[allow(clippy::cast_possible_truncation)]
                        let y = position.y as f32;
                        y / 80.0
                    }
                };
                self.pending_zoom += amount;
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let seconds = now
                    .duration_since(self.previous_frame)
                    .as_secs_f32()
                    .min(MAX_FRAME_SECONDS);
                self.previous_frame = now;
                if self.fixed_frame.is_none() {
                    self.presentation_seconds += seconds;
                }
                self.advance_camera(seconds);
                let result = self.refresh_detail().and_then(|()| {
                    self.gpu.as_mut().map_or(Ok(()), |gpu| {
                        gpu.render(self.camera, self.presentation_seconds, self.wireframe)
                    })
                });
                if let Err(error) = result {
                    self.fail(event_loop, error);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

/// Shapes a cursor offset from the scroll anchor into a pan request: the offset's direction, with a
/// magnitude in `0..=1` of the camera's scroll rate.
///
/// The magnitude follows the square root of the offset rather than the offset itself. A linear ramp
/// spends most of its travel below a usefully brisk rate, which reads as the view responding late
/// and then drifting; the square root puts a usable share of full rate within a short nudge of the
/// dead zone and still leaves the outer travel for fine control. A dead zone keeps a press that
/// barely moves from creeping.
fn anchor_scroll_request(offset: [f32; 2]) -> [f32; 2] {
    let distance = offset[0].hypot(offset[1]);
    if !distance.is_finite() || distance <= SCROLL_ANCHOR_DEAD_ZONE_PIXELS {
        return [0.0; 2];
    }
    let span = (SCROLL_ANCHOR_FULL_PIXELS - SCROLL_ANCHOR_DEAD_ZONE_PIXELS).max(1.0);
    let ramp = ((distance - SCROLL_ANCHOR_DEAD_ZONE_PIXELS) / span).clamp(0.0, 1.0);
    let scale = ramp.sqrt() / distance;
    // Screen up is negative while the camera's forward pan axis is positive.
    [offset[0] * scale, -offset[1] * scale]
}

/// Whether a middle-button release counts as a click, which faces the camera back to its starting
/// yaw, rather than the end of a rotation.
///
/// Both gates bind. Travel alone used to decide it and reset far too readily: pointer motion arrives
/// in small per-event steps, so a rotation that never jumped far within one event released as a
/// click. Summing the travel fixes that half, and the hold limit covers the rest, because a hold long
/// enough to aim a rotation is not a tap however still the cursor stayed.
fn rotate_release_is_click(held: Duration, travel_pixels: f32) -> bool {
    held <= ROTATE_CLICK_MAX_HOLD && travel_pixels <= ROTATE_CLICK_SLOP_PIXELS
}

fn terrain_viewer_title(title: &str, wireframe: bool, wireframe_available: bool) -> String {
    let mode = if wireframe { " [wireframe]" } else { "" };
    let wireframe_help = if wireframe_available {
        "M wireframe, "
    } else {
        ""
    };
    format!(
        "{title}{mode} | WASD/arrows scroll, RMB hold to scroll, wheel zoom, MMB rotate or click to face north, R reset, {wireframe_help}Esc close"
    )
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct TerrainInput(u8);

impl TerrainInput {
    const FORWARD: u8 = 1 << 0;
    const BACKWARD: u8 = 1 << 1;
    const LEFT: u8 = 1 << 2;
    const RIGHT: u8 = 1 << 3;

    /// Scroll keys only. The camera holds a fixed tilt at a height above the terrain, so there is
    /// nothing for the old free-flight vertical and boost keys to do.
    fn set(&mut self, code: KeyCode, pressed: bool) {
        let mask = match code {
            KeyCode::KeyW | KeyCode::ArrowUp => Self::FORWARD,
            KeyCode::KeyS | KeyCode::ArrowDown => Self::BACKWARD,
            KeyCode::KeyA | KeyCode::ArrowLeft => Self::LEFT,
            KeyCode::KeyD | KeyCode::ArrowRight => Self::RIGHT,
            _ => return,
        };
        if pressed {
            self.0 |= mask;
        } else {
            self.0 &= !mask;
        }
    }

    const fn active(self, mask: u8) -> bool {
        self.0 & mask != 0
    }

    /// Held scroll keys as a camera pan request, `x` right and `y` forward.
    fn pan(self) -> [f32; 2] {
        let axis = |negative: u8, positive: u8| match (self.active(negative), self.active(positive))
        {
            (true, false) => -1.0,
            (false, true) => 1.0,
            _ => 0.0,
        };
        [
            axis(Self::LEFT, Self::RIGHT),
            axis(Self::BACKWARD, Self::FORWARD),
        ]
    }
}

/// Ground elevation for [`cic_camera`], backed by the staged heightfield.
///
/// The camera holds its height above the terrain beneath it, so it needs elevation lookups without
/// depending on this crate's terrain type. `None` outside the map keeps the camera at its last known
/// elevation rather than diving.
struct StagedGround<'a>(&'a StagedTerrain);

impl GroundHeight for StagedGround<'_> {
    fn height_at(&self, x: f32, y: f32) -> Option<f32> {
        self.0.height_at_world([x, y])
    }
}

/// A resolved camera pose plus the projection depth the viewer draws with.
///
/// Controls no longer live here: [`cic_camera::RtsCamera`] owns the movement model so the game and a
/// future editor share it, and this type is only what the renderer, the shadow cascades, and the
/// terrain detail selection read.
#[derive(Debug, Clone, Copy, PartialEq)]
struct TerrainCamera {
    position: [f32; 3],
    yaw: f32,
    pitch: f32,
    far_plane: f32,
}

impl TerrainCamera {
    /// Derives the pose the renderer needs from a camera pose.
    ///
    /// Yaw and pitch are recovered from the forward vector rather than carried alongside it, so
    /// [`Self::forward`] reproduces the pose exactly and the detail-selection code that reasons
    /// about them keeps working unchanged.
    fn from_pose(pose: CameraPose, far_plane: f32) -> Self {
        let forward = pose.forward;
        let horizontal = forward[0].hypot(forward[1]);
        Self {
            position: pose.eye,
            yaw: forward[1].atan2(forward[0]),
            pitch: forward[2].atan2(horizontal),
            far_plane,
        }
    }

    /// Far plane for a map, generous enough that the horizon never clips.
    fn far_plane_for(terrain: &StagedTerrain) -> f32 {
        let (minimum, maximum) = terrain.bounds();
        let horizontal_span = (maximum[0] - minimum[0])
            .max(maximum[1] - minimum[1])
            .max(100.0);
        (horizontal_span * 20.0).max(10_000.0)
    }

    fn forward(self) -> [f32; 3] {
        let pitch_cosine = self.pitch.cos();
        [
            pitch_cosine * self.yaw.cos(),
            pitch_cosine * self.yaw.sin(),
            self.pitch.sin(),
        ]
    }

    #[allow(clippy::cast_precision_loss)]
    fn detail_requests(
        self,
        terrain: &StagedTerrain,
        viewport: [u32; 2],
    ) -> Result<Vec<TerrainDetailRequest>, crate::TerrainError> {
        let aspect = viewport[0] as f32 / viewport[1].max(1) as f32;
        let terrain_bounds = terrain.bounds();
        let Some((full_minimum, full_maximum)) =
            self.viewport_ground_bounds(terrain_bounds, aspect)
        else {
            return Ok(Vec::new());
        };
        let fallback = [
            (full_minimum[0] + full_maximum[0]) * 0.5,
            (full_minimum[1] + full_maximum[1]) * 0.5,
        ];
        let projection_scale = detail_projection_scale(viewport[1].max(1) as f32);
        let mut requests = Vec::with_capacity(2);
        for (pixels_per_cell, outer_screen_pixels) in [(16, 8.0_f32), (32, 16.0)] {
            let maximum_distance = projection_scale / outer_screen_pixels;
            let (minimum, maximum) = self
                .viewport_ground_bounds_limited(terrain_bounds, aspect, maximum_distance)
                .unwrap_or((fallback, fallback));
            requests.push(terrain.detail_request_at_density(minimum, maximum, pixels_per_cell)?);
        }
        Ok(requests)
    }

    #[allow(clippy::cast_precision_loss)]
    fn virtual_page_view(self, terrain: &StagedTerrain, viewport: [u32; 2]) -> VirtualPageView {
        let forward = self.forward();
        let right = normalize(cross(forward, [0.0, 0.0, 1.0]));
        let up = cross(right, forward);
        VirtualPageView::new(
            self.position,
            forward,
            right,
            up,
            terrain.bounds(),
            (CAMERA_VERTICAL_FOV * 0.5).tan(),
            viewport[0] as f32 / viewport[1].max(1) as f32,
            TERRAIN_CELL_WORLD_SIZE,
        )
    }

    fn viewport_ground_bounds(
        self,
        terrain_bounds: ([f32; 3], [f32; 3]),
        aspect: f32,
    ) -> Option<([f32; 2], [f32; 2])> {
        self.viewport_ground_bounds_limited(terrain_bounds, aspect, self.far_plane)
    }

    fn viewport_ground_bounds_limited(
        self,
        terrain_bounds: ([f32; 3], [f32; 3]),
        aspect: f32,
        maximum_distance: f32,
    ) -> Option<([f32; 2], [f32; 2])> {
        let (terrain_minimum, terrain_maximum) = terrain_bounds;
        let forward = self.forward();
        let right = normalize(cross(forward, [0.0, 0.0, 1.0]));
        let camera_up = cross(right, forward);
        let tangent = (CAMERA_VERTICAL_FOV * 0.5).tan();
        let mut footprint_minimum = [f32::INFINITY; 2];
        let mut footprint_maximum = [f32::NEG_INFINITY; 2];
        let mut found = false;
        let direction_for = |x: f32, y: f32| {
            let mut direction = forward;
            add_scaled(&mut direction, right, x * tangent * aspect);
            add_scaled(&mut direction, camera_up, y * tangent);
            direction
        };
        let maximum_depth = maximum_distance.min(self.far_plane);
        for x in [-1.0, 0.0, 1.0] {
            let lower = direction_for(x, -1.0);
            let upper = direction_for(x, 1.0);
            for y in [-1.0, -0.5, 0.0, 0.5, 1.0] {
                let direction = direction_for(x, y);
                let direction = normalize(direction);
                if direction[2].abs() <= f32::EPSILON {
                    continue;
                }
                let Some(maximum_ray_distance) =
                    ray_distance_for_view_depth(direction, forward, maximum_depth)
                else {
                    continue;
                };
                for height in [terrain_minimum[2], terrain_maximum[2]] {
                    let distance = (height - self.position[2]) / direction[2];
                    if !distance.is_finite() || distance <= 0.0 {
                        continue;
                    }
                    let distance = distance.min(maximum_ray_distance);
                    for axis in 0..2 {
                        let coordinate = self.position[axis] + direction[axis] * distance;
                        footprint_minimum[axis] = footprint_minimum[axis].min(coordinate);
                        footprint_maximum[axis] = footprint_maximum[axis].max(coordinate);
                    }
                    found = true;
                }
            }
            let vertical_delta = upper[2] - lower[2];
            if vertical_delta.abs() > f32::EPSILON {
                let horizon_ratio = -lower[2] / vertical_delta;
                if (0.0..=1.0).contains(&horizon_ratio) {
                    let horizon = normalize([
                        lower[0] + (upper[0] - lower[0]) * horizon_ratio,
                        lower[1] + (upper[1] - lower[1]) * horizon_ratio,
                        0.0,
                    ]);
                    let horizon_forward_scale = dot(horizon, forward);
                    if horizon_forward_scale <= f32::EPSILON {
                        continue;
                    }
                    for axis in 0..2 {
                        let coordinate = self.position[axis]
                            + horizon[axis] * maximum_depth / horizon_forward_scale;
                        footprint_minimum[axis] = footprint_minimum[axis].min(coordinate);
                        footprint_maximum[axis] = footprint_maximum[axis].max(coordinate);
                    }
                    found = true;
                }
            }
        }
        if !found {
            return None;
        }
        let minimum = [
            footprint_minimum[0].max(terrain_minimum[0]),
            footprint_minimum[1].max(terrain_minimum[1]),
        ];
        let maximum = [
            footprint_maximum[0].min(terrain_maximum[0]),
            footprint_maximum[1].min(terrain_maximum[1]),
        ];
        (minimum[0] <= maximum[0] && minimum[1] <= maximum[1]).then_some((minimum, maximum))
    }

    fn view_projection(self, aspect: f32) -> [[f32; 4]; 4] {
        multiply_matrix(
            perspective(CAMERA_VERTICAL_FOV, aspect, 1.0, self.far_plane),
            look_to(self.position, self.forward(), [0.0, 0.0, 1.0]),
        )
    }
}

/// Where a composed frame lands.
///
/// The two arms differ only in how the colour target is obtained and released; every pass between
/// the shadow cascades and the composite is identical, which is the point. Without this split the
/// whole deferred path was reachable only through a winit window, so no test could reach the
/// resource construction below and several wgpu validation failures in it shipped as runtime panics.
enum ViewerOutput {
    Window(WindowOutput),
    Capture(CaptureOutput),
}

struct WindowOutput {
    /// Held only to outlive the surface it created.
    _instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    window: Arc<Window>,
}

struct CaptureOutput {
    texture: wgpu::Texture,
    size: PhysicalSize<u32>,
}

/// A colour target acquired for one frame, plus whatever releasing it needs.
struct AcquiredFrame {
    view: wgpu::TextureView,
    surface: Option<wgpu::SurfaceTexture>,
    suboptimal: bool,
}

impl ViewerOutput {
    fn format(&self) -> wgpu::TextureFormat {
        match self {
            Self::Window(output) => output.config.format,
            Self::Capture(output) => output.texture.format(),
        }
    }

    fn size(&self) -> PhysicalSize<u32> {
        match self {
            Self::Window(output) => PhysicalSize::new(output.config.width, output.config.height),
            Self::Capture(output) => output.size,
        }
    }

    /// Returns the target for this frame, or `None` when the frame should be skipped entirely.
    fn acquire(&mut self, device: &wgpu::Device) -> Result<Option<AcquiredFrame>, ViewerError> {
        match self {
            Self::Window(output) => {
                let size = output.window.inner_size();
                if size.width == 0 || size.height == 0 {
                    return Ok(None);
                }
                let (surface_texture, suboptimal) = match output.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(texture) => (texture, false),
                    wgpu::CurrentSurfaceTexture::Suboptimal(texture) => (texture, true),
                    wgpu::CurrentSurfaceTexture::Timeout
                    | wgpu::CurrentSurfaceTexture::Occluded => return Ok(None),
                    wgpu::CurrentSurfaceTexture::Outdated => {
                        output.surface.configure(device, &output.config);
                        return Ok(None);
                    }
                    wgpu::CurrentSurfaceTexture::Lost => return Err(ViewerError::SurfaceLost),
                    wgpu::CurrentSurfaceTexture::Validation => {
                        return Err(ViewerError::SurfaceValidation);
                    }
                };
                let view = surface_texture
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());
                Ok(Some(AcquiredFrame {
                    view,
                    surface: Some(surface_texture),
                    suboptimal,
                }))
            }
            Self::Capture(output) => Ok(Some(AcquiredFrame {
                view: output
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default()),
                surface: None,
                suboptimal: false,
            })),
        }
    }

    /// Releases a frame acquired from `acquire`, after its commands have been submitted.
    fn present(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: AcquiredFrame) {
        let Self::Window(output) = self else {
            return;
        };
        if let Some(surface_texture) = frame.surface {
            output.window.pre_present_notify();
            queue.present(surface_texture);
        }
        if frame.suboptimal {
            output.surface.configure(device, &output.config);
        }
    }

    /// Reconfigures a window surface to a new size. Capture targets are fixed at construction.
    fn reconfigure(&mut self, device: &wgpu::Device, size: PhysicalSize<u32>) {
        if let Self::Window(output) = self {
            output.config.width = size.width;
            output.config.height = size.height;
            output.surface.configure(device, &output.config);
        }
    }
}

struct TerrainViewerGpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    edge_pipeline: wgpu::RenderPipeline,
    road_pipeline: wgpu::RenderPipeline,
    static_pipelines: StaticSceneryPipelines,
    terrain_shadow_pipeline: wgpu::RenderPipeline,
    scenery_shadow_pipeline: wgpu::RenderPipeline,
    boundary_pipeline: wgpu::RenderPipeline,
    lighting_pipeline: wgpu::RenderPipeline,
    composite_pipeline: wgpu::RenderPipeline,
    water_pipeline: wgpu::RenderPipeline,
    depth_resolve_pipeline: wgpu::RenderPipeline,
    wireframe_pipelines: Option<WireframePipelines>,
    lighting_layout: wgpu::BindGroupLayout,
    composite_layout: wgpu::BindGroupLayout,
    water_layout: wgpu::BindGroupLayout,
    depth_resolve_layout: wgpu::BindGroupLayout,
    ao_layout: wgpu::BindGroupLayout,
    ao_blur_layout: wgpu::BindGroupLayout,
    ao_pipeline: wgpu::RenderPipeline,
    ao_blur_pipeline: wgpu::RenderPipeline,
    _texture: wgpu::Texture,
    _edge_texture: wgpu::Texture,
    camera_uniform: wgpu::Buffer,
    shadow_uniform: wgpu::Buffer,
    cascade_uniforms: Vec<wgpu::Buffer>,
    cascade_bind_groups: Vec<wgpu::BindGroup>,
    /// Matrix bits each cascade layer was last rendered with, or `None` when its contents are not
    /// known to be valid. Reset whenever the shadow texture is recreated.
    cascade_cache: Vec<Option<[u32; 16]>>,
    bind_group: wgpu::BindGroup,
    edge_bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    edge_index_buffer: Option<wgpu::Buffer>,
    index_count: u32,
    edge_index_count: u32,
    virtual_terrain: VirtualTerrainGpu,
    roads: Option<RoadGpu>,
    scenery: Option<StaticSceneryGpu>,
    boundary: Option<BoundaryGpu>,
    overlays: Option<BoundaryGpu>,
    water: Option<WaterGpu>,
    water_appearance: WaterAppearanceGpu,
    lighting: TerrainLighting,
    deferred: DeferredTargets,
    output: ViewerOutput,
}

struct WireframePipelines {
    terrain: wgpu::RenderPipeline,
    edge: wgpu::RenderPipeline,
    road: wgpu::RenderPipeline,
    scenery: StaticSceneryPipelines,
    boundary: wgpu::RenderPipeline,
    water: wgpu::RenderPipeline,
}

#[derive(Clone, Copy)]
struct TerrainViewerScene<'a> {
    terrain: &'a StagedTerrain,
    roads: &'a StagedRoads,
    scenery: &'a StagedStaticScenery,
    boundary: &'a StagedBoundaryFence,
    overlays: &'a StagedMapOverlays,
    requests: &'a [TerrainDetailRequest],
    page_view: VirtualPageView,
    water: &'a StagedWater,
    water_appearance: &'a WaterAppearance,
    lighting: TerrainLighting,
}

struct RoadGpu {
    _textures: Vec<wgpu::Texture>,
    bind_groups: Vec<wgpu::BindGroup>,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    draws: Vec<RoadDrawGpu>,
}

struct BoundaryGpu {
    bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
}

struct StaticSceneryGpu {
    camera_bind_group: wgpu::BindGroup,
    models: Vec<StaticSceneryModelGpu>,
}

struct StaticSceneryModelGpu {
    resources: GpuResourceManager,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    instance_count: u32,
    draws: Vec<StaticSceneryDrawGpu>,
    /// Every instance's world-space bounding sphere, in the same order as `instance_buffer`, for
    /// deciding per cascade which of them are worth drawing.
    casters: Vec<CasterSphere>,
    /// The packed instance records, kept on the CPU so a cascade's subset can be gathered without
    /// reading back from the GPU. At eighty bytes an instance this is tens of kilobytes per scene.
    packed_instances: Vec<u8>,
    instance_stride: usize,
    /// Packed instance data per cascade, rebuilt when that cascade's own pass is about to run.
    ///
    /// One buffer per cascade rather than one shared buffer, because a cascade whose fit has not
    /// moved skips its pass entirely and keeps both its depth and the instances that produced it; a
    /// shared buffer would be overwritten by whichever cascade rebuilt last.
    cascade_instance_buffers: Vec<wgpu::Buffer>,
    /// How many instances each cascade's buffer currently holds.
    cascade_instance_counts: Vec<u32>,
}

/// One caster instance's world-space bounding sphere.
#[derive(Clone, Copy)]
struct CasterSphere {
    center: [f32; 3],
    radius: f32,
}

#[derive(Debug, Clone, Copy)]
struct StaticSceneryDrawGpu {
    material: usize,
    first_index: u32,
    index_count: u32,
}

struct StaticSceneryPipelines {
    opaque: [wgpu::RenderPipeline; 2],
    overlay: [wgpu::RenderPipeline; 2],
    alpha: [wgpu::RenderPipeline; 2],
    additive: [wgpu::RenderPipeline; 2],
    multiply: [wgpu::RenderPipeline; 2],
}

impl StaticSceneryPipelines {
    fn get(&self, blend: BlendMode, depth_write: bool, two_sided: bool) -> &wgpu::RenderPipeline {
        let pair = match (blend, depth_write) {
            (BlendMode::Opaque, true) => &self.opaque,
            (BlendMode::Opaque, false) => &self.overlay,
            (BlendMode::Alpha, _) => &self.alpha,
            (BlendMode::Additive, _) => &self.additive,
            (BlendMode::Multiply, _) => &self.multiply,
        };
        &pair[usize::from(two_sided)]
    }
}

#[derive(Debug, Clone, Copy)]
struct RoadDrawGpu {
    material_index: u32,
    first_index: u32,
    index_count: u32,
}

struct WaterGpu {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
}

struct WaterAppearanceGpu {
    _caustics: wgpu::Texture,
    caustic_view: wgpu::TextureView,
    caustic_sampler: wgpu::Sampler,
    _surface: wgpu::Texture,
    surface_view: wgpu::TextureView,
    surface_sampler: wgpu::Sampler,
    _sky: wgpu::Texture,
    sky_view: wgpu::TextureView,
    sky_sampler: wgpu::Sampler,
    _environment: wgpu::Texture,
    environment_view: wgpu::TextureView,
    environment_sampler: wgpu::Sampler,
    frame_count: u32,
    frames_per_second: u32,
    minimum_opacity: f32,
    opaque_depth: f32,
    source_surface_rgba: Option<[f32; 4]>,
    source_scroll_per_ms: [f32; 2],
    presentation: WaterPresentationPolicy,
}

impl WaterAppearanceGpu {
    #[allow(clippy::too_many_lines)]
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        appearance: &WaterAppearance,
    ) -> Result<Self, ViewerError> {
        let fallback = vec![0_u8];
        let (width, height, frame_count, frames_per_second, frames): (_, _, _, _, &[Vec<u8>]) =
            match appearance.caustics() {
                Some(sequence) => (
                    sequence.width(),
                    sequence.height(),
                    u32::try_from(sequence.frames().len())
                        .map_err(|_| RenderError::TextureTooLarge)?,
                    sequence.frames_per_second(),
                    sequence.frames(),
                ),
                None => (1, 1, 1, 1, std::slice::from_ref(&fallback)),
            };
        let mip_level_count = width.max(height).ilog2() + 1;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cic-render water caustic array"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: frame_count,
            },
            mip_level_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (layer, frame) in frames.iter().enumerate() {
            let layer = u32::try_from(layer).map_err(|_| RenderError::TextureTooLarge)?;
            let mut level_width = width;
            let mut level_height = height;
            let mut level = frame.clone();
            for mip_level in 0..mip_level_count {
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: layer,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    &level,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(level_width),
                        rows_per_image: Some(level_height),
                    },
                    wgpu::Extent3d {
                        width: level_width,
                        height: level_height,
                        depth_or_array_layers: 1,
                    },
                );
                if mip_level + 1 < mip_level_count {
                    let (next_width, next_height, next) =
                        gray_mip(level_width, level_height, &level)?;
                    level_width = next_width;
                    level_height = next_height;
                    level = next;
                }
            }
        }
        let caustic_view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("cic-render water caustic array view"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let caustic_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("cic-render water caustic sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 4,
            ..Default::default()
        });
        let (surface, surface_view, surface_sampler) =
            upload_standing_water_texture(device, queue, appearance)?;
        let (sky, sky_view, sky_sampler) = upload_water_texture(
            device,
            queue,
            appearance.sky_texture(),
            "cic-render water sky texture",
            [48, 92, 132, 255],
        )?;
        let (environment, environment_view, environment_sampler) = upload_water_texture(
            device,
            queue,
            appearance.environment_texture(),
            "cic-render water environment texture",
            [128, 128, 255, 255],
        )?;
        Ok(Self {
            _caustics: texture,
            caustic_view,
            caustic_sampler,
            _surface: surface,
            surface_view,
            surface_sampler,
            _sky: sky,
            sky_view,
            sky_sampler,
            _environment: environment,
            environment_view,
            environment_sampler,
            frame_count,
            frames_per_second,
            minimum_opacity: appearance.minimum_opacity(),
            opaque_depth: appearance.opaque_depth(),
            source_surface_rgba: appearance.source_surface_rgba(),
            source_scroll_per_ms: appearance.source_scroll_per_ms(),
            presentation: appearance.presentation(),
        })
    }
}

fn upload_standing_water_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    appearance: &WaterAppearance,
) -> Result<(wgpu::Texture, wgpu::TextureView, wgpu::Sampler), ViewerError> {
    upload_water_texture(
        device,
        queue,
        appearance.surface_texture(),
        "cic-render standing water texture",
        [255; 4],
    )
}

fn upload_water_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: Option<&crate::WaterSurfaceTexture>,
    label: &'static str,
    fallback: [u8; 4],
) -> Result<(wgpu::Texture, wgpu::TextureView, wgpu::Sampler), ViewerError> {
    let (width, height, rgba) = texture.map_or((1, 1, fallback.as_slice()), |texture| {
        (texture.width(), texture.height(), texture.rgba())
    });
    let mips = generate_srgb_mips(width, height, rgba)?;
    let texture =
        upload_mipmapped_terrain_texture(device, queue, label, width, height, rgba, &mips)?;
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some(label),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        anisotropy_clamp: 8,
        ..Default::default()
    });
    Ok((texture, view, sampler))
}

fn gray_mip(width: u32, height: u32, source: &[u8]) -> Result<(u32, u32, Vec<u8>), RenderError> {
    let target_width = (width / 2).max(1);
    let target_height = (height / 2).max(1);
    let target_len = usize::try_from(u64::from(target_width) * u64::from(target_height))
        .map_err(|_| RenderError::TextureTooLarge)?;
    let mut target = vec![0_u8; target_len];
    for target_y in 0..target_height {
        let row_start = target_y * height / target_height;
        let row_end = (target_y + 1) * height / target_height;
        for target_x in 0..target_width {
            let column_start = target_x * width / target_width;
            let column_end = (target_x + 1) * width / target_width;
            let mut sum = 0_u32;
            let mut count = 0_u32;
            for source_y in row_start..row_end {
                for source_x in column_start..column_end {
                    let index = usize::try_from(
                        u64::from(source_y) * u64::from(width) + u64::from(source_x),
                    )
                    .map_err(|_| RenderError::TextureTooLarge)?;
                    sum = sum.saturating_add(u32::from(source[index]));
                    count += 1;
                }
            }
            let target_index = usize::try_from(
                u64::from(target_y) * u64::from(target_width) + u64::from(target_x),
            )
            .map_err(|_| RenderError::TextureTooLarge)?;
            target[target_index] = u8::try_from((sum + count / 2) / count)
                .expect("averaged caustic luminance fits u8");
        }
    }
    Ok((target_width, target_height, target))
}

struct DeferredTargets {
    _albedo: wgpu::Texture,
    _normal: wgpu::Texture,
    _coverage: wgpu::Texture,
    _scene_depth: wgpu::Texture,
    _scene: wgpu::Texture,
    _shadow: wgpu::Texture,
    _ao: wgpu::Texture,
    _ao_blurred: wgpu::Texture,
    _albedo_ms: wgpu::Texture,
    _normal_ms: wgpu::Texture,
    _coverage_ms: wgpu::Texture,
    _depth_ms: wgpu::Texture,
    depth: wgpu::Texture,
    shadow_layer_views: Vec<wgpu::TextureView>,
    albedo_view: wgpu::TextureView,
    normal_view: wgpu::TextureView,
    coverage_view: wgpu::TextureView,
    scene_depth_view: wgpu::TextureView,
    scene_view: wgpu::TextureView,
    ao_view: wgpu::TextureView,
    ao_blurred_view: wgpu::TextureView,
    albedo_ms_view: wgpu::TextureView,
    normal_ms_view: wgpu::TextureView,
    coverage_ms_view: wgpu::TextureView,
    depth_ms_view: wgpu::TextureView,
    lighting_bind_group: wgpu::BindGroup,
    composite_bind_group: wgpu::BindGroup,
    water_bind_group: wgpu::BindGroup,
    depth_resolve_bind_group: wgpu::BindGroup,
    ao_bind_group: wgpu::BindGroup,
    ao_source_bind_group: wgpu::BindGroup,
}

#[derive(Clone, Copy)]
struct DeferredTargetResources<'a> {
    lighting_layout: &'a wgpu::BindGroupLayout,
    composite_layout: &'a wgpu::BindGroupLayout,
    water_layout: &'a wgpu::BindGroupLayout,
    depth_resolve_layout: &'a wgpu::BindGroupLayout,
    ao_layout: &'a wgpu::BindGroupLayout,
    ao_blur_layout: &'a wgpu::BindGroupLayout,
    camera_uniform: &'a wgpu::Buffer,
    shadow_uniform: &'a wgpu::Buffer,
    water_appearance: &'a WaterAppearanceGpu,
}

struct VirtualTerrainGpu {
    cache: VirtualPageCache,
    pending_jobs: Vec<VirtualPageJob>,
    compose_pipeline: wgpu::ComputePipeline,
    compose_bind_group: wgpu::BindGroup,
    mip_pipeline: wgpu::ComputePipeline,
    mip_bind_groups: Vec<wgpu::BindGroup>,
    job_buffer: wgpu::Buffer,
    _source_tiles: wgpu::Texture,
    _edge_tiles: wgpu::Texture,
    _macro_lattice: wgpu::Texture,
    _cell_buffer: wgpu::Buffer,
    _color_cache: wgpu::Texture,
    _edge_cache: wgpu::Texture,
    color_view: wgpu::TextureView,
    edge_view: wgpu::TextureView,
    page_tables: [wgpu::Texture; 2],
    page_table_views: [wgpu::TextureView; 2],
    config_buffer: wgpu::Buffer,
}

fn upload_mipmapped_terrain_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    width: u32,
    height: u32,
    base_rgba: &[u8],
    mips: &[TerrainMipLevel],
) -> Result<wgpu::Texture, ViewerError> {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: u32::try_from(mips.len())
            .map_err(|_| RenderError::TextureTooLarge)?
            .checked_add(1)
            .ok_or(RenderError::TextureTooLarge)?,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    write_texture_mip(queue, &texture, 0, width, height, base_rgba)?;
    for (index, mip) in mips.iter().enumerate() {
        let level = u32::try_from(index)
            .map_err(|_| RenderError::TextureTooLarge)?
            .checked_add(1)
            .ok_or(RenderError::TextureTooLarge)?;
        write_texture_mip(queue, &texture, level, mip.width, mip.height, &mip.rgba)?;
    }
    Ok(texture)
}

fn write_texture_mip(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    mip_level: u32,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<(), RenderError> {
    let expected = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|texels| texels.checked_mul(4))
        .and_then(|bytes| usize::try_from(bytes).ok())
        .ok_or(RenderError::TextureTooLarge)?;
    if rgba.len() != expected {
        return Err(RenderError::InvalidTexture);
    }
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width.checked_mul(4).ok_or(RenderError::TextureTooLarge)?),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    Ok(())
}

impl VirtualTerrainGpu {
    #[allow(clippy::too_many_lines)]
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        terrain: &StagedTerrain,
        requests: &[TerrainDetailRequest],
        view: VirtualPageView,
    ) -> Result<Self, ViewerError> {
        let source = terrain.virtual_source()?;
        let source_extent = source
            .source_tile_grid_width()
            .checked_mul(64)
            .ok_or(RenderError::TextureTooLarge)?;
        let source_tiles = upload_rgba_texture(
            device,
            queue,
            "cic-render virtual terrain source tiles",
            source_extent,
            source_extent,
            wgpu::TextureFormat::Rgba8Unorm,
            source.source_tile_atlas_rgba(),
        )?;
        let edge_extent = source
            .edge_tile_grid_width()
            .checked_mul(32)
            .ok_or(RenderError::TextureTooLarge)?;
        let edge_tiles = upload_rgba_texture(
            device,
            queue,
            "cic-render virtual terrain edge tiles",
            edge_extent,
            edge_extent,
            wgpu::TextureFormat::Rgba8Unorm,
            source.edge_tile_atlas_rgba(),
        )?;
        let macro_size = source.macro_lattice_size();
        let macro_lattice = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cic-render virtual terrain macro lattice"),
            size: wgpu::Extent3d {
                width: macro_size[0],
                height: macro_size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            macro_lattice.as_image_copy(),
            source.macro_lattice(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(macro_size[0]),
                rows_per_image: Some(macro_size[1]),
            },
            wgpu::Extent3d {
                width: macro_size[0],
                height: macro_size[1],
                depth_or_array_layers: 1,
            },
        );
        let cell_buffer = upload_buffer(
            device,
            queue,
            "cic-render virtual terrain cells",
            source.cell_bytes(),
            wgpu::BufferUsages::STORAGE,
        )?;
        let job_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cic-render virtual terrain page jobs"),
            size: u64::try_from(VIRTUAL_PAGE_LAYERS * 32)
                .map_err(|_| RenderError::TextureTooLarge)?,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let page_texture = |label| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: VIRTUAL_PAGE_EXTENT,
                    height: VIRTUAL_PAGE_EXTENT,
                    depth_or_array_layers: u32::try_from(VIRTUAL_PAGE_LAYERS).unwrap_or(64),
                },
                mip_level_count: VIRTUAL_PAGE_MIPS,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
                view_formats: &[],
            })
        };
        let color_cache = page_texture("cic-render virtual terrain color pages");
        let edge_cache = page_texture("cic-render virtual terrain edge pages");
        let color_view = color_cache.create_view(&wgpu::TextureViewDescriptor {
            label: Some("cic-render virtual terrain color page view"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let edge_view = edge_cache.create_view(&wgpu::TextureViewDescriptor {
            label: Some("cic-render virtual terrain edge page view"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let config_values = [
            source.cell_size()[0],
            source.cell_size()[1],
            source.source_tile_grid_width(),
            source.edge_tile_grid_width(),
            u32::from(source.modern()),
            VIRTUAL_PAGE_EXTENT,
            VIRTUAL_PAGE_BORDER,
            VIRTUAL_PAGE_INTERIOR,
        ];
        let config_bytes = config_values
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>();
        let config_buffer = upload_buffer(
            device,
            queue,
            "cic-render virtual terrain config",
            &config_bytes,
            wgpu::BufferUsages::UNIFORM,
        )?;

        let compose_layout = create_virtual_compose_layout(device);
        let mip_layout = create_virtual_mip_layout(device);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cic-render virtual terrain compute shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("terrain_virtual.wgsl").into()),
        });
        let compose_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("cic-render virtual terrain compose pipeline layout"),
                bind_group_layouts: &[Some(&compose_layout)],
                immediate_size: 0,
            });
        let compose_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("cic-render virtual terrain compose pipeline"),
            layout: Some(&compose_pipeline_layout),
            module: &shader,
            entry_point: Some("compose_page"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let color_base_view = mip_view(&color_cache, 0, "virtual color compose target");
        let edge_base_view = mip_view(&edge_cache, 0, "virtual edge compose target");
        let source_view = source_tiles.create_view(&wgpu::TextureViewDescriptor::default());
        let source_edge_view = edge_tiles.create_view(&wgpu::TextureViewDescriptor::default());
        let macro_view = macro_lattice.create_view(&wgpu::TextureViewDescriptor::default());
        let compose_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cic-render virtual terrain compose bind group"),
            layout: &compose_layout,
            entries: &[
                texture_binding(0, &source_view),
                texture_binding(1, &source_edge_view),
                texture_binding(2, &macro_view),
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: cell_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: job_buffer.as_entire_binding(),
                },
                texture_binding(5, &color_base_view),
                texture_binding(6, &edge_base_view),
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: config_buffer.as_entire_binding(),
                },
            ],
        });
        let empty_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cic-render virtual terrain empty group"),
            entries: &[],
        });
        let mip_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cic-render virtual terrain mip pipeline layout"),
            bind_group_layouts: &[Some(&empty_layout), Some(&mip_layout)],
            immediate_size: 0,
        });
        let mip_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("cic-render virtual terrain mip pipeline"),
            layout: Some(&mip_pipeline_layout),
            module: &shader,
            entry_point: Some("downsample_page"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let mut mip_bind_groups = Vec::new();
        for mip in 1..VIRTUAL_PAGE_MIPS {
            let previous_color = mip_view(&color_cache, mip - 1, "virtual color mip source");
            let previous_edge = mip_view(&edge_cache, mip - 1, "virtual edge mip source");
            let target_color = mip_view(&color_cache, mip, "virtual color mip target");
            let target_edge = mip_view(&edge_cache, mip, "virtual edge mip target");
            mip_bind_groups.push(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("cic-render virtual terrain mip bind group"),
                layout: &mip_layout,
                entries: &[
                    texture_binding(0, &previous_color),
                    texture_binding(1, &previous_edge),
                    texture_binding(2, &target_color),
                    texture_binding(3, &target_edge),
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: job_buffer.as_entire_binding(),
                    },
                ],
            }));
        }

        let mut cache = VirtualPageCache::new(source.cell_size());
        let page_table = |level: usize| {
            let size = cache.table_size(level);
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("cic-render virtual terrain page table"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R32Uint,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let page_tables = [page_table(0), page_table(1)];
        let page_table_views = [
            page_tables[0].create_view(&wgpu::TextureViewDescriptor::default()),
            page_tables[1].create_view(&wgpu::TextureViewDescriptor::default()),
        ];
        let update = cache.update(requests, view);
        write_page_tables(queue, &cache, &page_tables);
        let mut virtual_terrain = Self {
            cache,
            pending_jobs: update.jobs,
            compose_pipeline,
            compose_bind_group,
            mip_pipeline,
            mip_bind_groups,
            job_buffer,
            _source_tiles: source_tiles,
            _edge_tiles: edge_tiles,
            _macro_lattice: macro_lattice,
            _cell_buffer: cell_buffer,
            _color_cache: color_cache,
            _edge_cache: edge_cache,
            color_view,
            edge_view,
            page_tables,
            page_table_views,
            config_buffer,
        };
        virtual_terrain.write_jobs(queue);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("cic-render initial virtual terrain pages"),
        });
        virtual_terrain.encode(&mut encoder);
        queue.submit([encoder.finish()]);
        Ok(virtual_terrain)
    }

    fn update_residency(
        &mut self,
        queue: &wgpu::Queue,
        requests: &[TerrainDetailRequest],
        view: VirtualPageView,
    ) {
        let update = self.cache.update(requests, view);
        if update.tables_changed {
            write_page_tables(queue, &self.cache, &self.page_tables);
        }
        if !update.jobs.is_empty() {
            self.pending_jobs = update.jobs;
            self.write_jobs(queue);
        }
    }

    fn write_jobs(&self, queue: &wgpu::Queue) {
        if self.pending_jobs.is_empty() {
            return;
        }
        let mut bytes = Vec::with_capacity(self.pending_jobs.len() * 32);
        for job in &self.pending_jobs {
            job.write_bytes(&mut bytes);
        }
        queue.write_buffer(&self.job_buffer, 0, &bytes);
    }

    fn encode(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let Ok(job_count) = u32::try_from(self.pending_jobs.len()) else {
            return;
        };
        if job_count == 0 {
            return;
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("cic-render virtual terrain compose pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.compose_pipeline);
            pass.set_bind_group(0, &self.compose_bind_group, &[]);
            pass.dispatch_workgroups(
                VIRTUAL_PAGE_EXTENT.div_ceil(8),
                VIRTUAL_PAGE_EXTENT.div_ceil(8),
                job_count,
            );
        }
        for (index, bind_group) in self.mip_bind_groups.iter().enumerate() {
            let mip = u32::try_from(index).unwrap_or(0) + 1;
            let extent = (VIRTUAL_PAGE_EXTENT >> mip).max(1);
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("cic-render virtual terrain mip pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.mip_pipeline);
            pass.set_bind_group(1, bind_group, &[]);
            pass.dispatch_workgroups(extent.div_ceil(8), extent.div_ceil(8), job_count);
        }
        self.pending_jobs.clear();
    }
}

fn upload_rgba_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    rgba: &[u8],
) -> Result<wgpu::Texture, RenderError> {
    let expected = usize::try_from(u64::from(width) * u64::from(height) * 4)
        .map_err(|_| RenderError::TextureTooLarge)?;
    if rgba.len() != expected {
        return Err(RenderError::InvalidTexture);
    }
    let texture = device.create_texture(&wgpu::TextureDescriptor {
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
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    write_texture_mip(queue, &texture, 0, width, height, rgba)?;
    Ok(texture)
}

fn mip_view(texture: &wgpu::Texture, mip: u32, label: &'static str) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some(label),
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        base_mip_level: mip,
        mip_level_count: Some(1),
        ..Default::default()
    })
}

fn write_page_tables(queue: &wgpu::Queue, cache: &VirtualPageCache, textures: &[wgpu::Texture; 2]) {
    for (level, texture) in textures.iter().enumerate() {
        let size = cache.table_size(level);
        let bytes = cache
            .table(level)
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        queue.write_texture(
            texture.as_image_copy(),
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size[0] * 4),
                rows_per_image: Some(size[1]),
            },
            wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
        );
    }
}

fn upload_buffer(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    bytes: &[u8],
    usage: wgpu::BufferUsages,
) -> Result<wgpu::Buffer, RenderError> {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: u64::try_from(bytes.len()).map_err(|_| RenderError::GeometryTooLarge)?,
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&buffer, 0, bytes);
    Ok(buffer)
}

fn create_road_gpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    camera_uniform: &wgpu::Buffer,
    virtual_terrain: &VirtualTerrainGpu,
    roads: &StagedRoads,
) -> Result<Option<RoadGpu>, ViewerError> {
    if roads.indices().is_empty() || roads.draws().is_empty() {
        return Ok(None);
    }
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("cic-render road texture sampler"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        anisotropy_clamp: 16,
        ..Default::default()
    });
    let mut textures = Vec::with_capacity(roads.materials().len());
    let mut bind_groups = Vec::with_capacity(roads.materials().len());
    for material in roads.materials() {
        let source = material.texture();
        let mips = source_road_mips(source.width(), source.height(), source.rgba())?;
        let texture = upload_mipmapped_terrain_texture(
            device,
            queue,
            "cic-render road texture",
            source.width(),
            source.height(),
            source.rgba(),
            &mips,
        )?;
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cic-render road bind group"),
            layout,
            entries: &[
                texture_binding(0, &view),
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: camera_uniform.as_entire_binding(),
                },
                texture_binding(3, &virtual_terrain.color_view),
                texture_binding(4, &virtual_terrain.page_table_views[0]),
                texture_binding(5, &virtual_terrain.page_table_views[1]),
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: virtual_terrain.config_buffer.as_entire_binding(),
                },
            ],
        });
        textures.push(texture);
        bind_groups.push(bind_group);
    }
    let draws = roads
        .draws()
        .iter()
        .map(|draw| RoadDrawGpu {
            material_index: draw.material_index(),
            first_index: draw.first_index(),
            index_count: draw.index_count(),
        })
        .collect::<Vec<_>>();
    if draws.iter().any(|draw| {
        usize::try_from(draw.material_index).map_or(true, |index| index >= bind_groups.len())
    }) {
        return Err(RenderError::InvalidTexture.into());
    }
    Ok(Some(RoadGpu {
        _textures: textures,
        bind_groups,
        vertex_buffer: upload_buffer(
            device,
            queue,
            "cic-render road vertices",
            &roads.vertex_bytes(),
            wgpu::BufferUsages::VERTEX,
        )?,
        index_buffer: upload_buffer(
            device,
            queue,
            "cic-render road indices",
            &roads.index_bytes(),
            wgpu::BufferUsages::INDEX,
        )?,
        draws,
    }))
}

fn source_road_mips(
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<Vec<TerrainMipLevel>, ViewerError> {
    let mut mips = generate_srgb_mips(width, height, rgba)?;
    mips.truncate(SOURCE_ROAD_MIP_LEVELS.saturating_sub(1));
    Ok(mips)
}

fn create_boundary_gpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    camera_uniform: &wgpu::Buffer,
    boundary: &StagedBoundaryFence,
) -> Result<Option<BoundaryGpu>, ViewerError> {
    if boundary.indices().is_empty() {
        return Ok(None);
    }
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("cic-render boundary fence bind group"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: camera_uniform.as_entire_binding(),
        }],
    });
    Ok(Some(BoundaryGpu {
        bind_group,
        vertex_buffer: upload_buffer(
            device,
            queue,
            "cic-render boundary fence vertices",
            &boundary.vertex_bytes(),
            wgpu::BufferUsages::VERTEX,
        )?,
        index_buffer: upload_buffer(
            device,
            queue,
            "cic-render boundary fence indices",
            &boundary.index_bytes(),
            wgpu::BufferUsages::INDEX,
        )?,
        index_count: u32::try_from(boundary.indices().len())
            .map_err(|_| RenderError::GeometryTooLarge)?,
    }))
}

fn create_map_overlay_gpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    camera_uniform: &wgpu::Buffer,
    overlays: &StagedMapOverlays,
) -> Result<Option<BoundaryGpu>, ViewerError> {
    if overlays.indices().is_empty() {
        return Ok(None);
    }
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("cic-render MAP diagnostic overlay bind group"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: camera_uniform.as_entire_binding(),
        }],
    });
    Ok(Some(BoundaryGpu {
        bind_group,
        vertex_buffer: upload_buffer(
            device,
            queue,
            "cic-render MAP diagnostic overlay vertices",
            &overlays.vertex_bytes(),
            wgpu::BufferUsages::VERTEX,
        )?,
        index_buffer: upload_buffer(
            device,
            queue,
            "cic-render MAP diagnostic overlay indices",
            &overlays.index_bytes(),
            wgpu::BufferUsages::INDEX,
        )?,
        index_count: u32::try_from(overlays.indices().len())
            .map_err(|_| RenderError::GeometryTooLarge)?,
    }))
}

fn create_static_scenery_gpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    material_layout: &wgpu::BindGroupLayout,
    camera_layout: &wgpu::BindGroupLayout,
    camera_uniform: &wgpu::Buffer,
    scenery: &StagedStaticScenery,
) -> Result<Option<StaticSceneryGpu>, ViewerError> {
    if scenery.models().is_empty() {
        return Ok(None);
    }
    let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("cic-render static scenery camera bind group"),
        layout: camera_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: camera_uniform.as_entire_binding(),
        }],
    });
    let mut models = Vec::with_capacity(scenery.models().len());
    for staged in scenery.models() {
        let model = staged.model();
        let resources = GpuResourceManager::new(device, queue, model, material_layout)?;
        let draws = model
            .draws()
            .iter()
            .map(|draw| StaticSceneryDrawGpu {
                material: draw.material,
                first_index: draw.first_index,
                index_count: draw.index_count,
            })
            .collect();
        let instance_bytes = staged.instance_bytes();
        let instance_count =
            u32::try_from(staged.instances().len()).map_err(|_| RenderError::GeometryTooLarge)?;
        let stride = if staged.instances().is_empty() {
            0
        } else {
            instance_bytes.len() / staged.instances().len()
        };
        let casters = staged
            .instances()
            .iter()
            .zip(staged.caster_radii()?)
            .map(|(instance, radius)| CasterSphere {
                center: instance.position(),
                radius,
            })
            .collect::<Vec<_>>();
        // Each cascade's buffer is sized for every instance, since the worst case is that all of
        // them fall inside it. That is the same total as one full buffer per cascade, which for the
        // largest MAP scenes is well under a megabyte against the 180 MiB the cascades already cost.
        let caster_buffer_size = u64::try_from(instance_bytes.len().max(1))
            .map_err(|_| RenderError::GeometryTooLarge)?;
        let cascade_instance_buffers = (0..SHADOW_CASCADE_COUNT)
            .map(|cascade| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(&format!(
                        "cic-render static scenery casters for cascade {cascade}"
                    )),
                    size: caster_buffer_size,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect::<Vec<_>>();
        models.push(StaticSceneryModelGpu {
            resources,
            vertex_buffer: upload_buffer(
                device,
                queue,
                "cic-render static scenery vertices",
                &model.bind_pose_vertex_bytes()?,
                wgpu::BufferUsages::VERTEX,
            )?,
            index_buffer: upload_buffer(
                device,
                queue,
                "cic-render static scenery indices",
                &model.index_bytes(),
                wgpu::BufferUsages::INDEX,
            )?,
            instance_buffer: upload_buffer(
                device,
                queue,
                "cic-render static scenery instances",
                &instance_bytes,
                wgpu::BufferUsages::VERTEX,
            )?,
            instance_count,
            draws,
            casters,
            packed_instances: instance_bytes,
            instance_stride: stride,
            cascade_instance_buffers,
            cascade_instance_counts: vec![0; SHADOW_CASCADE_COUNT],
        });
    }
    Ok(Some(StaticSceneryGpu {
        camera_bind_group,
        models,
    }))
}

impl TerrainViewerGpu {
    /// Builds the presentation path against a winit window's surface.
    async fn new(
        window: Arc<Window>,
        display: OwnedDisplayHandle,
        scene: &TerrainViewerScene<'_>,
    ) -> Result<Self, ViewerError> {
        let descriptor = wgpu::InstanceDescriptor::new_with_display_handle(Box::new(display));
        let instance = wgpu::Instance::new(descriptor);
        let surface = instance
            .create_surface(window.clone())
            .map_err(ViewerError::CreateSurface)?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .map_err(RenderError::RequestAdapter)?;
        let wireframe_available = adapter
            .features()
            .contains(wgpu::Features::POLYGON_MODE_LINE);
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("cic-render terrain viewer device"),
                required_features: if wireframe_available {
                    wgpu::Features::POLYGON_MODE_LINE
                } else {
                    wgpu::Features::empty()
                },
                ..Default::default()
            })
            .await
            .map_err(RenderError::RequestDevice)?;
        let size = nonzero_size(window.inner_size());
        let mut config = surface
            .get_default_config(&adapter, size.width, size.height)
            .ok_or(ViewerError::UnsupportedSurface)?;
        config.present_mode = wgpu::PresentMode::Fifo;
        // The deferred composite tonemaps and returns the result unencoded, relying on the colour
        // target to apply the sRGB transfer function in hardware. `get_default_config` just takes
        // whichever format the backend lists first, which happens to be an sRGB one on the backends
        // in use but is not promised to be. On a backend that listed a linear format first the whole
        // scene would present several times too dark, so prefer the sRGB pair of whatever was chosen.
        if !config.format.is_srgb() {
            let encoded = config.format.add_srgb_suffix();
            if surface
                .get_capabilities(&adapter)
                .formats
                .contains(&encoded)
            {
                config.format = encoded;
            }
        }
        surface.configure(&device, &config);
        Self::assemble(
            device,
            queue,
            wireframe_available,
            ViewerOutput::Window(WindowOutput {
                _instance: instance,
                surface,
                config,
                window,
            }),
            scene,
        )
    }

    /// Builds the same resources against an offscreen colour target on a caller-supplied device.
    ///
    /// Wireframe pipelines are skipped: they need an optional adapter feature and no capture asks
    /// for them, so requiring them would make the capture path unavailable on adapters the
    /// presentation path runs on.
    fn offscreen(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: wgpu::Texture,
        scene: &TerrainViewerScene<'_>,
    ) -> Result<Self, ViewerError> {
        let size = PhysicalSize::new(texture.width(), texture.height());
        Self::assemble(
            device.clone(),
            queue.clone(),
            false,
            ViewerOutput::Capture(CaptureOutput { texture, size }),
            scene,
        )
    }

    #[allow(clippy::too_many_lines)]
    fn assemble(
        device: wgpu::Device,
        queue: wgpu::Queue,
        wireframe_available: bool,
        output: ViewerOutput,
        scene: &TerrainViewerScene<'_>,
    ) -> Result<Self, ViewerError> {
        let &TerrainViewerScene {
            terrain,
            roads,
            boundary,
            overlays,
            scenery,
            requests,
            page_view,
            water,
            water_appearance,
            lighting,
        } = scene;
        let size = output.size();
        let output_format = output.format();
        let layout = create_terrain_layout(&device);
        let lighting_layout = create_lighting_layout(&device);
        let composite_layout = create_composite_layout(&device);
        let water_layout = create_water_layout(&device);
        let depth_resolve_layout = create_depth_resolve_layout(&device);
        let ao_layout = create_ao_layout(&device);
        let ao_blur_layout = create_ao_blur_layout(&device);
        let boundary_layout = create_boundary_layout(&device);
        let shadow_layout = create_shadow_layout(&device);
        let material_layout = create_material_layout(&device);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cic-render terrain viewer shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("terrain_viewer.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cic-render terrain viewer pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = create_terrain_pipeline(
            &device,
            &shader,
            &pipeline_layout,
            "cic-render terrain viewer pipeline",
            TerrainPipelineOptions {
                blend: None,
                depth_write: true,
                write_geometry: true,
                polygon_mode: wgpu::PolygonMode::Fill,
                depth_bias: wgpu::DepthBiasState::default(),
            },
        );
        let edge_pipeline = create_terrain_pipeline(
            &device,
            &shader,
            &pipeline_layout,
            "cic-render terrain viewer edge pipeline",
            TerrainPipelineOptions {
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                depth_write: false,
                write_geometry: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                depth_bias: wgpu::DepthBiasState::default(),
            },
        );
        let road_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cic-render road viewer shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("road_viewer.wgsl").into()),
        });
        let road_pipeline = create_terrain_pipeline(
            &device,
            &road_shader,
            &pipeline_layout,
            "cic-render terrain-fitted road pipeline",
            TerrainPipelineOptions {
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                depth_write: false,
                write_geometry: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                depth_bias: ROAD_DEPTH_BIAS,
            },
        );
        let boundary_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cic-render boundary fence shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("boundary_viewer.wgsl").into()),
        });
        let boundary_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("cic-render boundary fence pipeline layout"),
                bind_group_layouts: &[Some(&boundary_layout)],
                immediate_size: 0,
            });
        let boundary_pipeline = create_boundary_pipeline(
            &device,
            &boundary_shader,
            &boundary_pipeline_layout,
            output_format,
            "cic-render boundary fence pipeline",
            wgpu::PolygonMode::Fill,
        );
        let static_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cic-render static scenery shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("static_scenery.wgsl").into()),
        });
        let static_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("cic-render static scenery pipeline layout"),
                bind_group_layouts: &[Some(&material_layout), Some(&boundary_layout)],
                immediate_size: 0,
            });
        let static_pipelines = create_static_scenery_pipelines(
            &device,
            &static_shader,
            &static_pipeline_layout,
            wgpu::PolygonMode::Fill,
        );
        let terrain_shadow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cic-render terrain shadow shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("terrain_shadow.wgsl").into()),
        });
        let scenery_shadow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cic-render scenery shadow shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("scene_shadow.wgsl").into()),
        });
        let terrain_shadow_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("cic-render terrain shadow pipeline layout"),
                bind_group_layouts: &[Some(&shadow_layout)],
                immediate_size: 0,
            });
        let scenery_shadow_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("cic-render scenery shadow pipeline layout"),
                bind_group_layouts: &[Some(&material_layout), Some(&shadow_layout)],
                immediate_size: 0,
            });
        let terrain_shadow_pipeline = create_terrain_shadow_pipeline(
            &device,
            &terrain_shadow_shader,
            &terrain_shadow_pipeline_layout,
        );
        let scenery_shadow_pipeline = create_scenery_shadow_pipeline(
            &device,
            &scenery_shadow_shader,
            &scenery_shadow_pipeline_layout,
        );
        let deferred_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cic-render deferred resolve shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("terrain_deferred.wgsl").into()),
        });
        let lighting_pipeline = create_fullscreen_pipeline(
            &device,
            &deferred_shader,
            &[&lighting_layout],
            "lighting_fragment",
            wgpu::TextureFormat::Rgba16Float,
            "cic-render deferred lighting pipeline",
        );
        let composite_pipeline = create_fullscreen_pipeline(
            &device,
            &deferred_shader,
            &[&lighting_layout, &composite_layout],
            "composite_fragment",
            output_format,
            "cic-render deferred composite pipeline",
        );
        let depth_resolve_pipeline =
            create_depth_resolve_pipeline(&device, &deferred_shader, &depth_resolve_layout);
        let ao_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cic-render ambient occlusion shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("terrain_ao.wgsl").into()),
        });
        let ao_pipeline = create_fullscreen_pipeline(
            &device,
            &ao_shader,
            &[&ao_layout],
            "ao_fragment",
            AO_FORMAT,
            "cic-render ambient occlusion pipeline",
        );
        let ao_blur_pipeline = create_fullscreen_pipeline(
            &device,
            &ao_shader,
            &[&ao_layout, &ao_blur_layout],
            "ao_blur_fragment",
            AO_FORMAT,
            "cic-render ambient occlusion blur pipeline",
        );
        let water_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cic-render modern water shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("water_viewer.wgsl").into()),
        });
        let water_pipeline = create_water_pipeline(
            &device,
            &water_shader,
            &water_layout,
            output_format,
            scene.water_appearance.additive_blending(),
            "cic-render forward water pipeline",
            wgpu::PolygonMode::Fill,
        );
        let wireframe_pipelines = wireframe_available.then(|| WireframePipelines {
            terrain: create_terrain_pipeline(
                &device,
                &shader,
                &pipeline_layout,
                "cic-render terrain wireframe pipeline",
                TerrainPipelineOptions {
                    blend: None,
                    depth_write: true,
                    write_geometry: true,
                    polygon_mode: wgpu::PolygonMode::Line,
                    depth_bias: wgpu::DepthBiasState::default(),
                },
            ),
            edge: create_terrain_pipeline(
                &device,
                &shader,
                &pipeline_layout,
                "cic-render terrain edge wireframe pipeline",
                TerrainPipelineOptions {
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    depth_write: false,
                    write_geometry: false,
                    polygon_mode: wgpu::PolygonMode::Line,
                    depth_bias: wgpu::DepthBiasState::default(),
                },
            ),
            road: create_terrain_pipeline(
                &device,
                &road_shader,
                &pipeline_layout,
                "cic-render road wireframe pipeline",
                TerrainPipelineOptions {
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    depth_write: false,
                    write_geometry: false,
                    polygon_mode: wgpu::PolygonMode::Line,
                    depth_bias: ROAD_DEPTH_BIAS,
                },
            ),
            scenery: create_static_scenery_pipelines(
                &device,
                &static_shader,
                &static_pipeline_layout,
                wgpu::PolygonMode::Line,
            ),
            boundary: create_boundary_pipeline(
                &device,
                &boundary_shader,
                &boundary_pipeline_layout,
                output_format,
                "cic-render boundary wireframe pipeline",
                wgpu::PolygonMode::Line,
            ),
            water: create_water_pipeline(
                &device,
                &water_shader,
                &water_layout,
                output_format,
                scene.water_appearance.additive_blending(),
                "cic-render water wireframe pipeline",
                wgpu::PolygonMode::Line,
            ),
        });

        let texture_mips = generate_srgb_mips(
            terrain.texture_width(),
            terrain.texture_height(),
            terrain.texture_rgba(),
        )?;
        let texture = upload_mipmapped_terrain_texture(
            &device,
            &queue,
            "cic-render terrain viewer texture",
            terrain.texture_width(),
            terrain.texture_height(),
            terrain.texture_rgba(),
            &texture_mips,
        )?;
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("cic-render terrain viewer sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 16,
            ..Default::default()
        });
        let camera_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cic-render terrain viewer camera"),
            size: CAMERA_UNIFORM_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shadow_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cic-render scene shadow cascades"),
            size: SHADOW_UNIFORM_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // The caster passes render one cascade at a time and reuse the single-cascade shader
        // interface, so each gets its own small buffer rather than a dynamic offset into the
        // receiver array, whose 80-byte stride is finer than the uniform offset alignment allows.
        let cascade_uniforms = (0..SHADOW_CASCADE_COUNT)
            .map(|index| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(&format!("cic-render scene shadow cascade {index}")),
                    size: SHADOW_CASCADE_BYTES,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect::<Vec<_>>();
        let cascade_bind_groups = cascade_uniforms
            .iter()
            .map(|uniform| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("cic-render scene shadow cascade bind group"),
                    layout: &shadow_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    }],
                })
            })
            .collect::<Vec<_>>();
        let virtual_terrain =
            VirtualTerrainGpu::new(&device, &queue, terrain, requests, page_view)?;
        let roads = create_road_gpu(
            &device,
            &queue,
            &layout,
            &camera_uniform,
            &virtual_terrain,
            roads,
        )?;
        let boundary =
            create_boundary_gpu(&device, &queue, &boundary_layout, &camera_uniform, boundary)?;
        let overlays =
            create_map_overlay_gpu(&device, &queue, &boundary_layout, &camera_uniform, overlays)?;
        let scenery = create_static_scenery_gpu(
            &device,
            &queue,
            &material_layout,
            &boundary_layout,
            &camera_uniform,
            scenery,
        )?;
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cic-render terrain viewer bind group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: camera_uniform.as_entire_binding(),
                },
                texture_binding(3, &virtual_terrain.color_view),
                texture_binding(4, &virtual_terrain.page_table_views[0]),
                texture_binding(5, &virtual_terrain.page_table_views[1]),
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: virtual_terrain.config_buffer.as_entire_binding(),
                },
            ],
        });
        let edge_texture_mips = generate_srgb_mips(
            terrain.texture_width(),
            terrain.texture_height(),
            terrain.edge_texture_rgba(),
        )?;
        let edge_texture = upload_mipmapped_terrain_texture(
            &device,
            &queue,
            "cic-render terrain viewer edge texture",
            terrain.texture_width(),
            terrain.texture_height(),
            terrain.edge_texture_rgba(),
            &edge_texture_mips,
        )?;
        let edge_view = edge_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let edge_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cic-render terrain viewer edge bind group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&edge_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: camera_uniform.as_entire_binding(),
                },
                texture_binding(3, &virtual_terrain.edge_view),
                texture_binding(4, &virtual_terrain.page_table_views[0]),
                texture_binding(5, &virtual_terrain.page_table_views[1]),
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: virtual_terrain.config_buffer.as_entire_binding(),
                },
            ],
        });
        let vertices = terrain.viewer_vertex_bytes()?;
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cic-render terrain viewer vertices"),
            size: u64::try_from(vertices.len()).map_err(|_| RenderError::GeometryTooLarge)?,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&vertex_buffer, 0, &vertices);
        let indices = terrain.index_bytes();
        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cic-render terrain viewer indices"),
            size: u64::try_from(indices.len()).map_err(|_| RenderError::GeometryTooLarge)?,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&index_buffer, 0, &indices);
        let index_count =
            u32::try_from(terrain.indices().len()).map_err(|_| RenderError::GeometryTooLarge)?;
        let edge_index_count = u32::try_from(terrain.edge_indices().len())
            .map_err(|_| RenderError::GeometryTooLarge)?;
        let edge_index_buffer = if edge_index_count == 0 {
            None
        } else {
            let bytes = terrain.edge_index_bytes();
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("cic-render terrain viewer edge indices"),
                size: u64::try_from(bytes.len()).map_err(|_| RenderError::GeometryTooLarge)?,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&buffer, 0, &bytes);
            Some(buffer)
        };
        let water = if water.indices().is_empty() {
            None
        } else {
            Some(WaterGpu {
                vertex_buffer: upload_buffer(
                    &device,
                    &queue,
                    "cic-render water vertices",
                    &water.vertex_bytes(),
                    wgpu::BufferUsages::VERTEX,
                )?,
                index_buffer: upload_buffer(
                    &device,
                    &queue,
                    "cic-render water indices",
                    &water.index_bytes(),
                    wgpu::BufferUsages::INDEX,
                )?,
                index_count: u32::try_from(water.indices().len())
                    .map_err(|_| RenderError::GeometryTooLarge)?,
            })
        };
        let water_appearance = WaterAppearanceGpu::new(&device, &queue, water_appearance)?;
        let deferred = DeferredTargets::new(
            &device,
            size,
            DeferredTargetResources {
                lighting_layout: &lighting_layout,
                composite_layout: &composite_layout,
                water_layout: &water_layout,
                depth_resolve_layout: &depth_resolve_layout,
                ao_layout: &ao_layout,
                ao_blur_layout: &ao_blur_layout,
                camera_uniform: &camera_uniform,
                shadow_uniform: &shadow_uniform,
                water_appearance: &water_appearance,
            },
        );
        Ok(Self {
            device,
            queue,
            pipeline,
            edge_pipeline,
            road_pipeline,
            static_pipelines,
            terrain_shadow_pipeline,
            scenery_shadow_pipeline,
            boundary_pipeline,
            lighting_pipeline,
            composite_pipeline,
            water_pipeline,
            depth_resolve_pipeline,
            wireframe_pipelines,
            lighting_layout,
            composite_layout,
            water_layout,
            depth_resolve_layout,
            ao_layout,
            ao_blur_layout,
            ao_pipeline,
            ao_blur_pipeline,
            _texture: texture,
            _edge_texture: edge_texture,
            camera_uniform,
            shadow_uniform,
            cascade_uniforms,
            cascade_bind_groups,
            cascade_cache: vec![None; SHADOW_CASCADE_COUNT],
            bind_group,
            edge_bind_group,
            vertex_buffer,
            index_buffer,
            edge_index_buffer,
            index_count,
            edge_index_count,
            virtual_terrain,
            roads,
            scenery,
            boundary,
            overlays,
            water,
            water_appearance,
            lighting,
            deferred,
            output,
        })
    }

    fn wireframe_available(&self) -> bool {
        self.wireframe_pipelines.is_some()
    }

    fn update_virtual_residency(
        &mut self,
        requests: &[TerrainDetailRequest],
        view: VirtualPageView,
    ) {
        self.virtual_terrain
            .update_residency(&self.queue, requests, view);
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.output.reconfigure(&self.device, size);
        self.deferred = DeferredTargets::new(
            &self.device,
            size,
            DeferredTargetResources {
                lighting_layout: &self.lighting_layout,
                composite_layout: &self.composite_layout,
                water_layout: &self.water_layout,
                depth_resolve_layout: &self.depth_resolve_layout,
                ao_layout: &self.ao_layout,
                ao_blur_layout: &self.ao_blur_layout,
                camera_uniform: &self.camera_uniform,
                shadow_uniform: &self.shadow_uniform,
                water_appearance: &self.water_appearance,
            },
        );
        // Recreating the deferred targets allocates a new shadow texture, so no cascade layer holds
        // valid depth any more.
        self.cascade_cache.fill(None);
    }

    fn render(
        &mut self,
        camera: TerrainCamera,
        presentation_seconds: f32,
        wireframe: bool,
    ) -> Result<(), ViewerError> {
        let Some(frame) = self.output.acquire(&self.device)? else {
            return Ok(());
        };
        let encoder = self.encode_frame(
            camera,
            presentation_seconds,
            wireframe,
            MapViewPasses::ALL,
            &frame.view,
        )?;
        self.queue.submit([encoder.finish()]);
        self.output.present(&self.device, &self.queue, frame);
        Ok(())
    }

    /// Records every pass for one frame into a fresh encoder, without submitting it.
    ///
    /// Split from `render` so the capture path can append a readback copy to the same encoder and
    /// then submit once, rather than duplicating the pass sequence it is meant to be testing.
    #[allow(clippy::cast_precision_loss, clippy::too_many_lines)]
    fn encode_frame(
        &mut self,
        camera: TerrainCamera,
        presentation_seconds: f32,
        wireframe: bool,
        passes: MapViewPasses,
        view: &wgpu::TextureView,
    ) -> Result<wgpu::CommandEncoder, ViewerError> {
        let size = self.output.size();
        let aspect = size.width as f32 / size.height as f32;
        let viewport = [size.width as f32, size.height as f32];
        let matrix = camera.view_projection(aspect);
        let caustic_animation = [
            self.water_appearance.frame_count as f32,
            self.water_appearance.frames_per_second as f32,
        ];
        let water_material = [
            self.water_appearance.minimum_opacity,
            self.water_appearance.opaque_depth,
            0.58,
            0.06,
        ];
        let water_surface = self
            .water_appearance
            .source_surface_rgba
            .unwrap_or([0.0; 4]);
        let water_motion = [
            self.water_appearance.source_scroll_per_ms[0],
            self.water_appearance.source_scroll_per_ms[1],
            f32::from(self.water_appearance.source_surface_rgba.is_some()),
            f32::from(self.water_appearance.presentation == WaterPresentationPolicy::Modern),
        ];
        self.queue.write_buffer(
            &self.camera_uniform,
            0,
            &camera_bytes(&CameraUniformInput {
                matrix,
                inverse_matrix: invert_matrix(matrix),
                position: camera.position,
                time: presentation_seconds,
                viewport,
                detail_fade_uv: detail_fade_distances(viewport[1]),
                caustic_animation,
                water_material,
                water_surface,
                water_motion,
                lighting: self.lighting,
            }),
        );
        // Cascades follow the camera, so the fit is recomputed every frame rather than staged once.
        let shadow = scene_shadow(camera, aspect, self.lighting);
        self.queue.write_buffer(
            &self.shadow_uniform,
            0,
            &shadow_uniform_bytes(&shadow, presentation_seconds),
        );
        for (uniform, cascade) in self.cascade_uniforms.iter().zip(shadow.cascades) {
            self.queue.write_buffer(
                uniform,
                0,
                &cascade_uniform_bytes(cascade, presentation_seconds),
            );
        }
        let depth_view = self
            .deferred
            .depth
            .create_view(&wgpu::TextureViewDescriptor::default());
        let depth_ms_view = &self.deferred.depth_ms_view;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("cic-render terrain viewer encoder"),
            });
        self.virtual_terrain.encode(&mut encoder);
        // Cascades whose fitted matrix has not moved since the previous frame already hold correct
        // depth, because every caster is static. Their passes are skipped, which also leaves the
        // layer's existing contents untouched since no pass clears it.
        let cascade_keys = shadow.cascades.map(|cascade| matrix_bits(cascade.matrix));
        let mut redraw_cascade = [true; SHADOW_CASCADE_COUNT];
        for index in SHADOW_CACHED_CASCADE_START..SHADOW_CASCADE_COUNT {
            if self.cascade_cache.get(index).copied().flatten() == Some(cascade_keys[index]) {
                redraw_cascade[index] = false;
            }
        }
        // Gather each redrawing cascade's casters before recording any pass, so the pass loop can
        // borrow the scenery immutably.
        //
        // Only cascades about to redraw are rebuilt. A cached cascade keeps the depth it already
        // holds, and the instance list that produced that depth is a function of the fitted matrix
        // alone — the cache key — so an unchanged matrix means an unchanged list. Sway is the one
        // thing that animates and it lives in the shader, not in these bytes.
        let cull_scenery = if passes.shadows {
            self.scenery.as_mut()
        } else {
            None
        };
        if let Some(scenery) = cull_scenery {
            for model in &mut scenery.models {
                if model.instance_stride == 0 {
                    continue;
                }
                let mut packed: [Vec<u8>; SHADOW_CASCADE_COUNT] = Default::default();
                for (index, caster) in model.casters.iter().enumerate() {
                    // Projected once per caster rather than once per cascade, since every
                    // cascade's bounds are measured along the same light basis.
                    let light_space = [
                        dot(caster.center, shadow.light_basis[0]),
                        dot(caster.center, shadow.light_basis[1]),
                        dot(caster.center, shadow.light_basis[2]),
                    ];
                    let start = index * model.instance_stride;
                    let Some(record) = model
                        .packed_instances
                        .get(start..start + model.instance_stride)
                    else {
                        continue;
                    };
                    for (cascade_index, cascade) in shadow.cascades.iter().enumerate() {
                        if redraw_cascade[cascade_index]
                            && cascade.bounds.accepts(light_space, caster.radius)
                        {
                            packed[cascade_index].extend_from_slice(record);
                        }
                    }
                }
                for (cascade_index, bytes) in packed.iter().enumerate() {
                    if !redraw_cascade[cascade_index] {
                        continue;
                    }
                    model.cascade_instance_counts[cascade_index] =
                        u32::try_from(bytes.len() / model.instance_stride).unwrap_or(0);
                    if let Some(buffer) = model
                        .cascade_instance_buffers
                        .get(cascade_index)
                        .filter(|_| !bytes.is_empty())
                    {
                        self.queue.write_buffer(buffer, 0, bytes);
                    }
                }
            }
        }
        for (cascade_index, cascade_bind_group) in self.cascade_bind_groups.iter().enumerate() {
            let Some(layer_view) = self.deferred.shadow_layer_views.get(cascade_index) else {
                continue;
            };
            if !redraw_cascade.get(cascade_index).copied().unwrap_or(true) {
                continue;
            }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cic-render primary directional shadow cascade pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: layer_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if !passes.shadows {
                // The clear above already put the far plane in every texel, which reads as fully
                // lit, so isolating shadows out needs no shader variant.
                continue;
            }
            pass.set_pipeline(&self.terrain_shadow_pipeline);
            pass.set_bind_group(0, cascade_bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.index_count, 0, 0..1);
            if let Some(scenery) = &self.scenery {
                pass.set_pipeline(&self.scenery_shadow_pipeline);
                pass.set_bind_group(1, cascade_bind_group, &[]);
                for model in &scenery.models {
                    let instance_count = model
                        .cascade_instance_counts
                        .get(cascade_index)
                        .copied()
                        .unwrap_or(0);
                    let Some(instances) = model.cascade_instance_buffers.get(cascade_index) else {
                        continue;
                    };
                    // A model with nothing inside this cascade costs no draws at all, which is the
                    // point: every instance used to be submitted to all five cascades.
                    if instance_count == 0 {
                        continue;
                    }
                    pass.set_vertex_buffer(0, model.vertex_buffer.slice(..));
                    pass.set_vertex_buffer(1, instances.slice(..));
                    pass.set_index_buffer(model.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    for draw in &model.draws {
                        let material = model
                            .resources
                            .materials
                            .get(draw.material)
                            .ok_or(RenderError::InvalidMaterial)?;
                        // Additive draws are emissive decoration; giving them opaque depth makes
                        // a glow cast a solid silhouette. Multiply draws are the detail and
                        // lightmap stages layered over geometry whose base stage already wrote
                        // this depth, so rasterizing them again only costs draws. Neither belongs
                        // in a shadow map.
                        if matches!(material.blend, BlendMode::Additive | BlendMode::Multiply) {
                            continue;
                        }
                        let end = draw
                            .first_index
                            .checked_add(draw.index_count)
                            .ok_or(RenderError::GeometryTooLarge)?;
                        pass.set_bind_group(0, &material.bind_group, &[]);
                        pass.draw_indexed(draw.first_index..end, 0, 0..instance_count);
                    }
                }
            }
        }
        for (slot, key) in self.cascade_cache.iter_mut().zip(cascade_keys) {
            *slot = Some(key);
        }
        let wireframe_pipelines = wireframe
            .then_some(self.wireframe_pipelines.as_ref())
            .flatten();
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cic-render terrain G-buffer pass"),
                color_attachments: &[
                    Some(clear_attachment_resolved(
                        &self.deferred.albedo_ms_view,
                        &self.deferred.albedo_view,
                        wgpu::Color::TRANSPARENT,
                    )),
                    Some(clear_attachment_resolved(
                        &self.deferred.normal_ms_view,
                        &self.deferred.normal_view,
                        wgpu::Color::TRANSPARENT,
                    )),
                    Some(clear_attachment_resolved(
                        &self.deferred.coverage_ms_view,
                        &self.deferred.coverage_view,
                        wgpu::Color::TRANSPARENT,
                    )),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_ms_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(
                wireframe_pipelines.map_or(&self.pipeline, |pipelines| &pipelines.terrain),
            );
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.index_count, 0, 0..1);
            if let Some(edge_index_buffer) = &self.edge_index_buffer {
                pass.set_pipeline(
                    wireframe_pipelines.map_or(&self.edge_pipeline, |pipelines| &pipelines.edge),
                );
                pass.set_bind_group(0, &self.edge_bind_group, &[]);
                pass.set_index_buffer(edge_index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.edge_index_count, 0, 0..1);
            }
            if let Some(roads) = &self.roads {
                pass.set_pipeline(
                    wireframe_pipelines.map_or(&self.road_pipeline, |pipelines| &pipelines.road),
                );
                pass.set_vertex_buffer(0, roads.vertex_buffer.slice(..));
                pass.set_index_buffer(roads.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                for draw in &roads.draws {
                    let Some(bind_group) = usize::try_from(draw.material_index)
                        .ok()
                        .and_then(|index| roads.bind_groups.get(index))
                    else {
                        continue;
                    };
                    let Some(end) = draw.first_index.checked_add(draw.index_count) else {
                        continue;
                    };
                    pass.set_bind_group(0, bind_group, &[]);
                    pass.draw_indexed(draw.first_index..end, 0, 0..1);
                }
            }
            if let Some(scenery) = &self.scenery {
                pass.set_bind_group(1, &scenery.camera_bind_group, &[]);
                for model in &scenery.models {
                    pass.set_vertex_buffer(0, model.vertex_buffer.slice(..));
                    pass.set_vertex_buffer(1, model.instance_buffer.slice(..));
                    pass.set_index_buffer(model.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    for draw in &model.draws {
                        let material = model
                            .resources
                            .materials
                            .get(draw.material)
                            .ok_or(RenderError::InvalidMaterial)?;
                        let end = draw
                            .first_index
                            .checked_add(draw.index_count)
                            .ok_or(RenderError::GeometryTooLarge)?;
                        let scenery_pipelines = wireframe_pipelines
                            .map_or(&self.static_pipelines, |pipelines| &pipelines.scenery);
                        pass.set_pipeline(scenery_pipelines.get(
                            material.blend,
                            material.depth_write,
                            material.two_sided,
                        ));
                        pass.set_bind_group(0, &material.bind_group, &[]);
                        pass.draw_indexed(draw.first_index..end, 0, 0..model.instance_count);
                    }
                }
            }
        }
        {
            // wgpu has no automatic depth resolve, so the multisampled G-buffer depth is
            // manually resolved into the single-sample depth texture that the boundary,
            // overlay, and water passes below reuse for depth testing against the terrain.
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cic-render G-buffer depth resolve pass"),
                color_attachments: &[Some(clear_attachment(
                    &self.deferred.scene_depth_view,
                    wgpu::Color::WHITE,
                ))],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.depth_resolve_pipeline);
            pass.set_bind_group(0, &self.deferred.depth_resolve_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        {
            // Occlusion runs on the resolved geometry targets, before lighting consumes it as the
            // ambient visibility term.
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cic-render ambient occlusion pass"),
                color_attachments: &[Some(clear_attachment(
                    &self.deferred.ao_view,
                    wgpu::Color::WHITE,
                ))],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if passes.occlusion {
                pass.set_pipeline(&self.ao_pipeline);
                pass.set_bind_group(0, &self.deferred.ao_bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cic-render ambient occlusion blur pass"),
                color_attachments: &[Some(clear_attachment(
                    &self.deferred.ao_blurred_view,
                    wgpu::Color::WHITE,
                ))],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if passes.occlusion {
                pass.set_pipeline(&self.ao_blur_pipeline);
                pass.set_bind_group(0, &self.deferred.ao_bind_group, &[]);
                pass.set_bind_group(1, &self.deferred.ao_source_bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cic-render deferred lighting pass"),
                color_attachments: &[Some(clear_attachment(
                    &self.deferred.scene_view,
                    wgpu::Color::BLACK,
                ))],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.lighting_pipeline);
            pass.set_bind_group(0, &self.deferred.lighting_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cic-render scene composite pass"),
                color_attachments: &[Some(clear_attachment(view, wgpu::Color::BLACK))],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.composite_pipeline);
            pass.set_bind_group(0, &self.deferred.lighting_bind_group, &[]);
            pass.set_bind_group(1, &self.deferred.composite_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        if self.boundary.is_some() || self.overlays.is_some() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cic-render forward MAP diagnostics pass"),
                color_attachments: &[Some(load_attachment(view))],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(
                wireframe_pipelines
                    .map_or(&self.boundary_pipeline, |pipelines| &pipelines.boundary),
            );
            for geometry in [&self.boundary, &self.overlays].into_iter().flatten() {
                pass.set_bind_group(0, &geometry.bind_group, &[]);
                pass.set_vertex_buffer(0, geometry.vertex_buffer.slice(..));
                pass.set_index_buffer(geometry.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..geometry.index_count, 0, 0..1);
            }
        }
        if let Some(water) = &self.water {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cic-render forward water pass"),
                color_attachments: &[Some(load_attachment(view))],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(
                wireframe_pipelines.map_or(&self.water_pipeline, |pipelines| &pipelines.water),
            );
            pass.set_bind_group(0, &self.deferred.water_bind_group, &[]);
            pass.set_vertex_buffer(0, water.vertex_buffer.slice(..));
            pass.set_index_buffer(water.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..water.index_count, 0, 0..1);
        }
        Ok(encoder)
    }
}

fn create_boundary_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cic-render boundary fence layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(CAMERA_UNIFORM_BYTES),
            },
            count: None,
        }],
    })
}

/// Layout for the caster passes, which render one cascade at a time and so bind a single cascade
/// rather than the whole receiver array. Sized with [`SHADOW_CASCADE_BYTES`], not
/// [`SHADOW_UNIFORM_BYTES`] — the receiver bindings are the ones that need the full array.
fn create_shadow_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cic-render scene shadow cascade layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(SHADOW_CASCADE_BYTES),
            },
            count: None,
        }],
    })
}

fn create_static_scenery_pipelines(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    polygon_mode: wgpu::PolygonMode,
) -> StaticSceneryPipelines {
    StaticSceneryPipelines {
        opaque: create_static_scenery_pipeline_pair(
            device,
            shader,
            layout,
            "cic-render static scenery opaque pipeline",
            None,
            true,
            polygon_mode,
        ),
        overlay: create_static_scenery_pipeline_pair(
            device,
            shader,
            layout,
            "cic-render static scenery overlay pipeline",
            None,
            false,
            polygon_mode,
        ),
        alpha: create_static_scenery_pipeline_pair(
            device,
            shader,
            layout,
            "cic-render static scenery alpha pipeline",
            Some(wgpu::BlendState::ALPHA_BLENDING),
            false,
            polygon_mode,
        ),
        additive: create_static_scenery_pipeline_pair(
            device,
            shader,
            layout,
            "cic-render static scenery additive pipeline",
            Some(static_additive_blend()),
            false,
            polygon_mode,
        ),
        multiply: create_static_scenery_pipeline_pair(
            device,
            shader,
            layout,
            "cic-render static scenery multiply pipeline",
            Some(static_multiply_blend()),
            false,
            polygon_mode,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn create_static_scenery_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    label: &str,
    blend: Option<wgpu::BlendState>,
    depth_write: bool,
    two_sided: bool,
    polygon_mode: wgpu::PolygonMode,
) -> wgpu::RenderPipeline {
    let targets = terrain_color_targets(blend, true);
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vertex_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[
                Some(wgpu::VertexBufferLayout {
                    array_stride: 48,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 12,
                            shader_location: 1,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 24,
                            shader_location: 2,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 40,
                            shader_location: 3,
                        },
                    ],
                }),
                Some(wgpu::VertexBufferLayout {
                    array_stride: 80,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 0,
                            shader_location: 4,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 16,
                            shader_location: 5,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 32,
                            shader_location: 6,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 48,
                            shader_location: 7,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 64,
                            shader_location: 8,
                        },
                    ],
                }),
            ],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fragment_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &targets,
        }),
        primitive: wgpu::PrimitiveState {
            cull_mode: (!two_sided).then_some(wgpu::Face::Back),
            polygon_mode,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(depth_write),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: GBUFFER_SAMPLE_COUNT,
            // Alpha-tested cutout foliage (`discard`d in the shared fragment shader) gets
            // its per-leaf silhouette antialiased by treating output alpha as sample
            // coverage. Only meaningful for the non-blended opaque/overlay variants;
            // pipelines with real translucency already resolve their edges via blending.
            alpha_to_coverage_enabled: blend.is_none(),
            ..Default::default()
        },
        multiview_mask: None,
        cache: None,
    })
}

#[allow(clippy::too_many_arguments)]
fn create_static_scenery_pipeline_pair(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    label: &str,
    blend: Option<wgpu::BlendState>,
    depth_write: bool,
    polygon_mode: wgpu::PolygonMode,
) -> [wgpu::RenderPipeline; 2] {
    [
        create_static_scenery_pipeline(
            device,
            shader,
            layout,
            &format!("{label} single-sided"),
            blend,
            depth_write,
            false,
            polygon_mode,
        ),
        create_static_scenery_pipeline(
            device,
            shader,
            layout,
            &format!("{label} two-sided"),
            blend,
            depth_write,
            true,
            polygon_mode,
        ),
    ]
}

fn static_additive_blend() -> wgpu::BlendState {
    wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
    }
}

fn static_multiply_blend() -> wgpu::BlendState {
    wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Dst,
            dst_factor: wgpu::BlendFactor::Zero,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::DstAlpha,
            dst_factor: wgpu::BlendFactor::Zero,
            operation: wgpu::BlendOperation::Add,
        },
    }
}

fn create_boundary_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
    label: &str,
    polygon_mode: wgpu::PolygonMode,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vertex_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: 28,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x3,
                        offset: 0,
                        shader_location: 0,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x4,
                        offset: 12,
                        shader_location: 1,
                    },
                ],
            })],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fragment_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            cull_mode: None,
            polygon_mode,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn create_terrain_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cic-render terrain viewer layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(CAMERA_UNIFORM_BYTES),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            integer_texture_layout_entry(4),
            integer_texture_layout_entry(5),
            wgpu::BindGroupLayoutEntry {
                binding: 6,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(32),
                },
                count: None,
            },
        ],
    })
}

fn create_virtual_compose_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cic-render virtual terrain compose layout"),
        entries: &[
            compute_texture_layout_entry(0, wgpu::TextureSampleType::Float { filterable: false }),
            compute_texture_layout_entry(1, wgpu::TextureSampleType::Float { filterable: false }),
            compute_texture_layout_entry(2, wgpu::TextureSampleType::Uint),
            storage_buffer_layout_entry(3),
            storage_buffer_layout_entry(4),
            storage_texture_layout_entry(5),
            storage_texture_layout_entry(6),
            wgpu::BindGroupLayoutEntry {
                binding: 7,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(32),
                },
                count: None,
            },
        ],
    })
}

fn create_virtual_mip_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cic-render virtual terrain mip layout"),
        entries: &[
            compute_array_texture_layout_entry(0),
            compute_array_texture_layout_entry(1),
            storage_texture_layout_entry(2),
            storage_texture_layout_entry(3),
            storage_buffer_layout_entry(4),
        ],
    })
}

fn compute_texture_layout_entry(
    binding: u32,
    sample_type: wgpu::TextureSampleType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn compute_array_texture_layout_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2Array,
            multisampled: false,
        },
        count: None,
    }
}

fn storage_buffer_layout_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn storage_texture_layout_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: wgpu::TextureFormat::Rgba8Unorm,
            view_dimension: wgpu::TextureViewDimension::D2Array,
        },
        count: None,
    }
}

fn integer_texture_layout_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Uint,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

#[derive(Debug, Clone, Copy)]
struct TerrainPipelineOptions {
    blend: Option<wgpu::BlendState>,
    depth_write: bool,
    write_geometry: bool,
    polygon_mode: wgpu::PolygonMode,
    depth_bias: wgpu::DepthBiasState,
}

fn create_terrain_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    label: &str,
    options: TerrainPipelineOptions,
) -> wgpu::RenderPipeline {
    let targets = terrain_color_targets(options.blend, options.write_geometry);
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vertex_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: 32,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x3,
                        offset: 0,
                        shader_location: 0,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: 12,
                        shader_location: 1,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x3,
                        offset: 20,
                        shader_location: 2,
                    },
                ],
            })],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fragment_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &targets,
        }),
        primitive: wgpu::PrimitiveState {
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: Some(wgpu::Face::Back),
            polygon_mode: options.polygon_mode,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(options.depth_write),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: options.depth_bias,
        }),
        multisample: wgpu::MultisampleState {
            count: GBUFFER_SAMPLE_COUNT,
            ..Default::default()
        },
        multiview_mask: None,
        cache: None,
    })
}

fn terrain_color_targets(
    albedo_blend: Option<wgpu::BlendState>,
    write_geometry: bool,
) -> [Option<wgpu::ColorTargetState>; 3] {
    let geometry_write_mask = if write_geometry {
        wgpu::ColorWrites::ALL
    } else {
        wgpu::ColorWrites::empty()
    };
    [
        Some(wgpu::ColorTargetState {
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            blend: albedo_blend,
            write_mask: wgpu::ColorWrites::ALL,
        }),
        Some(wgpu::ColorTargetState {
            format: wgpu::TextureFormat::Rgba16Float,
            blend: None,
            write_mask: geometry_write_mask,
        }),
        // Coverage and emissive strength only. World position used to live here as a second
        // `Rgba16Float`; it is reconstructed from scene depth instead, which is both exact and
        // six bytes per sample cheaper. See `world_from_depth` in `terrain_deferred.wgsl`.
        Some(wgpu::ColorTargetState {
            format: GBUFFER_COVERAGE_FORMAT,
            blend: None,
            write_mask: geometry_write_mask.intersection(wgpu::ColorWrites::RED),
        }),
    ]
}

fn create_lighting_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cic-render deferred lighting layout"),
        entries: &[
            texture_layout_entry(0, true),
            texture_layout_entry(1, false),
            texture_layout_entry(2, false),
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(CAMERA_UNIFORM_BYTES),
                },
                count: None,
            },
            depth_texture_layout_entry(4),
            comparison_sampler_layout_entry(5),
            shadow_matrix_layout_entry(6),
            texture_layout_entry(8, false),
            // Binding 7 is the multisampled depth the resolve pass reads, declared in the same
            // shader module, so the resolved copy this pass samples takes the next free slot.
            texture_layout_entry(9, false),
        ],
    })
}

/// Group zero for both ambient-occlusion passes: the resolved geometry targets plus the camera.
fn create_ao_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cic-render ambient occlusion layout"),
        entries: &[
            texture_layout_entry(0, false),
            texture_layout_entry(1, false),
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(CAMERA_UNIFORM_BYTES),
                },
                count: None,
            },
            texture_layout_entry(3, false),
        ],
    })
}

/// Group one for the blur pass: the unfiltered occlusion it reduces.
fn create_ao_blur_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cic-render ambient occlusion blur layout"),
        entries: &[texture_layout_entry(0, false)],
    })
}

fn create_composite_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cic-render deferred composite layout"),
        entries: &[
            texture_layout_entry(0, true),
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

fn create_water_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cic-render forward water layout"),
        entries: &[
            texture_layout_entry(0, false),
            texture_layout_entry(1, false),
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(CAMERA_UNIFORM_BYTES),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            texture_layout_entry(5, true),
            wgpu::BindGroupLayoutEntry {
                binding: 6,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            texture_layout_entry(7, true),
            wgpu::BindGroupLayoutEntry {
                binding: 8,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            texture_layout_entry(9, true),
            wgpu::BindGroupLayoutEntry {
                binding: 10,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            depth_texture_layout_entry(11),
            comparison_sampler_layout_entry(12),
            shadow_matrix_layout_entry(13),
            texture_layout_entry(14, false),
        ],
    })
}

fn create_terrain_shadow_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("cic-render terrain shadow pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("terrain_shadow"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: 32,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 0,
                }],
            })],
        },
        fragment: None,
        primitive: wgpu::PrimitiveState {
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: Some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: Some(shadow_depth_state()),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn create_scenery_shadow_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("cic-render scenery shadow pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("scenery_shadow"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[
                Some(wgpu::VertexBufferLayout {
                    array_stride: 48,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 40,
                            shader_location: 3,
                        },
                    ],
                }),
                Some(wgpu::VertexBufferLayout {
                    array_stride: 80,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 0,
                            shader_location: 4,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 16,
                            shader_location: 5,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 32,
                            shader_location: 6,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 48,
                            shader_location: 7,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 64,
                            shader_location: 8,
                        },
                    ],
                }),
            ],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("scenery_shadow_fragment"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[],
        }),
        primitive: wgpu::PrimitiveState {
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(shadow_depth_state()),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn shadow_depth_state() -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format: wgpu::TextureFormat::Depth32Float,
        depth_write_enabled: Some(true),
        depth_compare: Some(wgpu::CompareFunction::LessEqual),
        stencil: wgpu::StencilState::default(),
        bias: wgpu::DepthBiasState {
            constant: 2,
            slope_scale: 2.0,
            clamp: 0.0,
        },
    }
}

fn depth_texture_layout_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Depth,
            view_dimension: wgpu::TextureViewDimension::D2Array,
            multisampled: false,
        },
        count: None,
    }
}

fn comparison_sampler_layout_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
        count: None,
    }
}

fn shadow_matrix_layout_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(SHADOW_UNIFORM_BYTES),
        },
        count: None,
    }
}

fn texture_layout_entry(binding: u32, filterable: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn create_fullscreen_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layouts: &[&wgpu::BindGroupLayout],
    fragment_entry: &str,
    format: wgpu::TextureFormat,
    label: &str,
) -> wgpu::RenderPipeline {
    let optional_layouts = layouts.iter().copied().map(Some).collect::<Vec<_>>();
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &optional_layouts,
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("fullscreen_vertex"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment_entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn create_depth_resolve_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cic-render depth resolve layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 7,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Depth,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: true,
            },
            count: None,
        }],
    })
}

fn create_depth_resolve_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("cic-render depth resolve pipeline layout"),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("cic-render depth resolve pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("fullscreen_vertex"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("depth_resolve_fragment"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: SCENE_DEPTH_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::RED,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn create_water_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    water_layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
    additive_blending: bool,
    label: &str,
    polygon_mode: wgpu::PolygonMode,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("cic-render forward water pipeline layout"),
        bind_group_layouts: &[Some(water_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("water_vertex"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: 12,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 0,
                }],
            })],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("water_fragment"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(if additive_blending {
                    wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent::OVER,
                    }
                } else {
                    wgpu::BlendState::ALPHA_BLENDING
                }),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            cull_mode: None,
            polygon_mode,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

impl DeferredTargets {
    // Keeping all attachments and bind groups in one constructor makes their matching
    // resize-time recreation auditable.
    #[allow(clippy::too_many_lines)]
    fn new(
        device: &wgpu::Device,
        size: PhysicalSize<u32>,
        resources: DeferredTargetResources<'_>,
    ) -> Self {
        let albedo = render_texture(
            device,
            size,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            "G-buffer albedo",
        );
        let normal = render_texture(
            device,
            size,
            wgpu::TextureFormat::Rgba16Float,
            "G-buffer normal",
        );
        let coverage = render_texture(device, size, GBUFFER_COVERAGE_FORMAT, "G-buffer coverage");
        let scene_depth = render_texture(device, size, SCENE_DEPTH_FORMAT, "resolved scene depth");
        let scene = render_texture(
            device,
            size,
            wgpu::TextureFormat::Rgba16Float,
            "lit scene color",
        );
        let ao = render_texture(device, size, AO_FORMAT, "ambient occlusion");
        let ao_blurred = render_texture(device, size, AO_FORMAT, "ambient occlusion blurred");
        let albedo_ms = render_texture_multisampled(
            device,
            size,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            "G-buffer albedo MSAA",
        );
        let normal_ms = render_texture_multisampled(
            device,
            size,
            wgpu::TextureFormat::Rgba16Float,
            "G-buffer normal MSAA",
        );
        let coverage_ms = render_texture_multisampled(
            device,
            size,
            GBUFFER_COVERAGE_FORMAT,
            "G-buffer coverage MSAA",
        );
        let depth_ms = create_depth_multisampled(device, size);
        let shadow = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cic-render primary directional shadow cascades"),
            size: wgpu::Extent3d {
                width: SHADOW_MAP_EXTENT,
                height: SHADOW_MAP_EXTENT,
                depth_or_array_layers: SHADOW_CASCADE_LAYERS,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let depth = create_depth(device, size);
        let albedo_view = albedo.create_view(&wgpu::TextureViewDescriptor::default());
        let normal_view = normal.create_view(&wgpu::TextureViewDescriptor::default());
        let coverage_view = coverage.create_view(&wgpu::TextureViewDescriptor::default());
        let scene_depth_view = scene_depth.create_view(&wgpu::TextureViewDescriptor::default());
        let scene_view = scene.create_view(&wgpu::TextureViewDescriptor::default());
        let ao_view = ao.create_view(&wgpu::TextureViewDescriptor::default());
        let ao_blurred_view = ao_blurred.create_view(&wgpu::TextureViewDescriptor::default());
        let albedo_ms_view = albedo_ms.create_view(&wgpu::TextureViewDescriptor::default());
        let normal_ms_view = normal_ms.create_view(&wgpu::TextureViewDescriptor::default());
        let coverage_ms_view = coverage_ms.create_view(&wgpu::TextureViewDescriptor::default());
        let depth_ms_view = depth_ms.create_view(&wgpu::TextureViewDescriptor::default());
        // One single-layer view per cascade to render into, plus one array view to sample.
        let shadow_layer_views = (0..SHADOW_CASCADE_LAYERS)
            .map(|index| {
                shadow.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("cic-render shadow cascade layer"),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: index,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect::<Vec<_>>();
        let shadow_view = shadow.create_view(&wgpu::TextureViewDescriptor {
            label: Some("cic-render shadow cascade array"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("cic-render primary directional shadow sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            compare: Some(wgpu::CompareFunction::LessEqual),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let lighting_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cic-render deferred lighting bind group"),
            layout: resources.lighting_layout,
            entries: &[
                texture_binding(0, &albedo_view),
                texture_binding(1, &normal_view),
                texture_binding(2, &coverage_view),
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: resources.camera_uniform.as_entire_binding(),
                },
                texture_binding(4, &shadow_view),
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&shadow_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: resources.shadow_uniform.as_entire_binding(),
                },
                texture_binding(8, &ao_blurred_view),
                texture_binding(9, &scene_depth_view),
            ],
        });
        let ao_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cic-render ambient occlusion bind group"),
            layout: resources.ao_layout,
            entries: &[
                texture_binding(0, &normal_view),
                texture_binding(1, &coverage_view),
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: resources.camera_uniform.as_entire_binding(),
                },
                texture_binding(3, &scene_depth_view),
            ],
        });
        let ao_source_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cic-render ambient occlusion blur source bind group"),
            layout: resources.ao_blur_layout,
            entries: &[texture_binding(0, &ao_view)],
        });
        let composite_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cic-render deferred composite bind group"),
            layout: resources.composite_layout,
            entries: &[
                texture_binding(0, &scene_view),
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&device.create_sampler(
                        &wgpu::SamplerDescriptor {
                            label: Some("cic-render deferred antialiasing sampler"),
                            address_mode_u: wgpu::AddressMode::ClampToEdge,
                            address_mode_v: wgpu::AddressMode::ClampToEdge,
                            mag_filter: wgpu::FilterMode::Linear,
                            min_filter: wgpu::FilterMode::Linear,
                            ..Default::default()
                        },
                    )),
                },
            ],
        });
        let water_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cic-render forward water bind group"),
            layout: resources.water_layout,
            entries: &[
                texture_binding(0, &scene_view),
                texture_binding(1, &coverage_view),
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: resources.camera_uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(
                        &resources.water_appearance.caustic_view,
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(
                        &resources.water_appearance.caustic_sampler,
                    ),
                },
                texture_binding(5, &resources.water_appearance.surface_view),
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::Sampler(
                        &resources.water_appearance.surface_sampler,
                    ),
                },
                texture_binding(7, &resources.water_appearance.sky_view),
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::Sampler(
                        &resources.water_appearance.sky_sampler,
                    ),
                },
                texture_binding(9, &resources.water_appearance.environment_view),
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: wgpu::BindingResource::Sampler(
                        &resources.water_appearance.environment_sampler,
                    ),
                },
                texture_binding(11, &shadow_view),
                wgpu::BindGroupEntry {
                    binding: 12,
                    resource: wgpu::BindingResource::Sampler(&shadow_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 13,
                    resource: resources.shadow_uniform.as_entire_binding(),
                },
                texture_binding(14, &scene_depth_view),
            ],
        });
        let depth_resolve_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cic-render depth resolve bind group"),
            layout: resources.depth_resolve_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 7,
                resource: wgpu::BindingResource::TextureView(&depth_ms_view),
            }],
        });
        Self {
            _albedo: albedo,
            _normal: normal,
            _coverage: coverage,
            _scene_depth: scene_depth,
            _scene: scene,
            _shadow: shadow,
            _ao: ao,
            _ao_blurred: ao_blurred,
            _albedo_ms: albedo_ms,
            _normal_ms: normal_ms,
            _coverage_ms: coverage_ms,
            _depth_ms: depth_ms,
            depth,
            shadow_layer_views,
            albedo_view,
            normal_view,
            coverage_view,
            scene_depth_view,
            scene_view,
            ao_view,
            ao_blurred_view,
            albedo_ms_view,
            normal_ms_view,
            coverage_ms_view,
            depth_ms_view,
            lighting_bind_group,
            composite_bind_group,
            water_bind_group,
            depth_resolve_bind_group,
            ao_bind_group,
            ao_source_bind_group,
        }
    }
}

fn render_texture(
    device: &wgpu::Device,
    size: PhysicalSize<u32>,
    format: wgpu::TextureFormat,
    label: &str,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size.width,
            height: size.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

/// A multisampled G-buffer color attachment; only ever read via an automatic
/// end-of-pass resolve into a single-sample `render_texture`, never sampled directly.
fn render_texture_multisampled(
    device: &wgpu::Device,
    size: PhysicalSize<u32>,
    format: wgpu::TextureFormat,
    label: &str,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size.width,
            height: size.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: GBUFFER_SAMPLE_COUNT,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    })
}

/// The multisampled depth buffer used while rasterizing the G-buffer pass. WGPU has no
/// automatic depth resolve, so `depth_resolve_fragment` reads this via `textureLoad` and
/// writes the single-sample `create_depth` texture that the later forward passes reuse.
fn create_depth_multisampled(device: &wgpu::Device, size: PhysicalSize<u32>) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("cic-render terrain viewer depth MSAA"),
        size: wgpu::Extent3d {
            width: size.width.max(1),
            height: size.height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: GBUFFER_SAMPLE_COUNT,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

fn texture_binding(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    }
}

fn clear_attachment(
    view: &wgpu::TextureView,
    color: wgpu::Color,
) -> wgpu::RenderPassColorAttachment<'_> {
    wgpu::RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: None,
        ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(color),
            store: wgpu::StoreOp::Store,
        },
    }
}

/// A multisampled G-buffer color attachment that resolves into `resolve_target` at the
/// end of the pass; the raw multisample data itself is discarded since only the resolved,
/// single-sample texture is ever read afterward.
fn clear_attachment_resolved<'a>(
    view: &'a wgpu::TextureView,
    resolve_target: &'a wgpu::TextureView,
    color: wgpu::Color,
) -> wgpu::RenderPassColorAttachment<'a> {
    wgpu::RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: Some(resolve_target),
        ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(color),
            store: wgpu::StoreOp::Discard,
        },
    }
}

fn load_attachment(view: &wgpu::TextureView) -> wgpu::RenderPassColorAttachment<'_> {
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

fn perspective(field_of_view: f32, aspect: f32, near: f32, far: f32) -> [[f32; 4]; 4] {
    let focal = 1.0 / (field_of_view * 0.5).tan();
    [
        [focal / aspect, 0.0, 0.0, 0.0],
        [0.0, focal, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, near * far / (near - far), 0.0],
    ]
}

#[cfg(test)]
fn orthographic(radius: f32, near: f32, far: f32) -> [[f32; 4]; 4] {
    orthographic_extents(radius, radius, near, far)
}

/// Orthographic projection with independent half-extents per axis, so a cascade can be fitted to a
/// box rather than forced to a square.
fn orthographic_extents(half_x: f32, half_y: f32, near: f32, far: f32) -> [[f32; 4]; 4] {
    [
        [1.0 / half_x, 0.0, 0.0, 0.0],
        [0.0, 1.0 / half_y, 0.0, 0.0],
        [0.0, 0.0, 1.0 / (near - far), 0.0],
        [0.0, 0.0, near / (near - far), 1.0],
    ]
}

/// Rounds a cascade half-extent up onto a fixed geometric ladder.
///
/// A light-space box is not invariant to camera yaw the way a bounding sphere is, so its extents
/// would otherwise change every frame as the view turns. Texel snapping only removes translation
/// jitter; a changing extent rescales every texel and makes shadow edges re-quantize, which crawls.
/// Snapping the extent to a ladder means it holds still across small rotations and steps only
/// occasionally, at the cost of at most this ladder's ratio in wasted coverage.
fn quantize_cascade_extent(value: f32) -> f32 {
    // 2^(1/8): at most about nine percent of an axis given away to keep the fit stable.
    const LADDER: f32 = 1.090_507_7;
    if value <= f32::EPSILON || !value.is_finite() {
        return 1.0;
    }
    LADDER.powf((value.ln() / LADDER.ln()).ceil())
}

/// One fitted cascade of the primary directional light, plus the two world-space scales a shadow
/// receiver needs to bias itself correctly.
///
/// `depth_range` and `texel_world` are per cascade because both bias terms have to be expressed in
/// world units and converted against the frustum they belong to, rather than hard-coded as a
/// normalized-depth epsilon: a fixed epsilon silently becomes a larger world-space offset as the
/// frustum grows, and with cascades the frusta differ from each other by more than an order of
/// magnitude.
#[derive(Clone, Copy)]
struct ShadowCascade {
    matrix: [[f32; 4]; 4],
    /// World units spanned by the full `0..=1` normalized depth range.
    depth_range: f32,
    /// World units covered by one shadow-map texel.
    texel_world: f32,
    /// What this cascade actually covers, for deciding which casters are worth drawing into it.
    bounds: CascadeBounds,
}

/// One cascade's coverage, measured along the light basis its `SceneShadow` carries.
#[derive(Clone, Copy)]
struct CascadeBounds {
    center_right: f32,
    half_right: f32,
    center_up: f32,
    half_up: f32,
    near_forward: f32,
    far_forward: f32,
}

impl CascadeBounds {
    /// A degenerate volume that accepts nothing, for the array initializer every cascade is then
    /// overwritten in and for tests that only exercise the packing.
    const EMPTY: Self = Self {
        center_right: 0.0,
        half_right: 0.0,
        center_up: 0.0,
        half_up: 0.0,
        near_forward: 0.0,
        far_forward: 0.0,
    };

    /// Whether a caster's bounding sphere can put anything into this cascade's shadow map.
    ///
    /// Exact rather than merely conservative, which is what makes culling on it invisible. The light
    /// is directional, so a caster's shadow keeps the caster's own light-space right and up
    /// coordinates; one that misses the cascade laterally rasterizes to no texels at all. The depth
    /// test rejects only what the pipeline's own near and far planes would clip. So a rejected
    /// instance could not have changed a single texel, and the rendered frame is unchanged.
    fn accepts(self, light_space: [f32; 3], radius: f32) -> bool {
        (light_space[0] - self.center_right).abs() <= self.half_right + radius
            && (light_space[1] - self.center_up).abs() <= self.half_up + radius
            && light_space[2] + radius >= self.near_forward
            && light_space[2] - radius <= self.far_forward
    }
}

#[derive(Clone, Copy)]
struct SceneShadow {
    cascades: [ShadowCascade; SHADOW_CASCADE_COUNT],
    /// Shared light basis every cascade's bounds are expressed in, so a caster is projected into
    /// light space once per frame rather than once per cascade.
    light_basis: [[f32; 3]; 3],
}

/// The primary light's travel direction, falling back to the original preview angle when a MAP
/// declares a degenerate one.
fn primary_light_forward(lighting: TerrainLighting) -> [f32; 3] {
    let forward = lighting.lights()[0].source_direction();
    if dot(forward, forward) <= f32::EPSILON {
        return normalize([0.45, 0.35, -0.82]);
    }
    normalize(forward)
}

/// Light-space basis. `look_to` derives its right and up the same way from the same reference, so
/// texel snapping performed in this basis matches the projection the snapped centre feeds.
fn light_basis(forward: [f32; 3]) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let reference = if forward[2].abs() > 0.95 {
        [0.0, 1.0, 0.0]
    } else {
        [0.0, 0.0, 1.0]
    };
    let right = normalize(cross(forward, reference));
    let up = cross(right, forward);
    (right, up, reference)
}

/// Fits one cascade to the slice of the camera frustum between `near` and `far`.
///
/// The slice's bounding sphere sets the orthographic extent, which makes the fit independent of
/// camera yaw and so keeps the cascade a stable size as the view turns. The centre is then snapped
/// to whole texels in light space: without that, sub-texel centre motion re-quantizes every shadow
/// edge each frame and the result crawls visibly while the camera moves.
#[allow(clippy::cast_precision_loss)]
fn fit_cascade(
    camera: TerrainCamera,
    aspect: f32,
    near: f32,
    far: f32,
    light_forward: [f32; 3],
) -> ShadowCascade {
    let view_forward = normalize(camera.forward());
    let (view_right, view_up, _) = light_basis(view_forward);
    let tangent_vertical = (CAMERA_VERTICAL_FOV * 0.5).tan();
    let tangent_horizontal = tangent_vertical * aspect;

    let mut corners = [[0.0_f32; 3]; 8];
    let mut index = 0;
    for distance in [near, far] {
        let half_height = tangent_vertical * distance;
        let half_width = tangent_horizontal * distance;
        let slice_center = add(camera.position, scale_vector(view_forward, distance));
        for vertical in [-half_height, half_height] {
            for horizontal in [-half_width, half_width] {
                corners[index] = add(
                    slice_center,
                    add(
                        scale_vector(view_right, horizontal),
                        scale_vector(view_up, vertical),
                    ),
                );
                index += 1;
            }
        }
    }

    // Fit the slice with a box aligned to the light instead of a bounding sphere. A sphere has to
    // contain the slice's longest diagonal, which is far larger than the slice's actual footprint
    // when viewed along the light -- most of all on the axis the slice is shallow in.
    let (light_right, light_up, reference) = light_basis(light_forward);
    let mut minimum = [f32::MAX; 3];
    let mut maximum = [f32::MIN; 3];
    for corner in corners {
        let light_space = [
            dot(corner, light_right),
            dot(corner, light_up),
            dot(corner, light_forward),
        ];
        for axis in 0..3 {
            minimum[axis] = minimum[axis].min(light_space[axis]);
            maximum[axis] = maximum[axis].max(light_space[axis]);
        }
    }

    let half_right = quantize_cascade_extent((maximum[0] - minimum[0]) * 0.5);
    let half_up = quantize_cascade_extent((maximum[1] - minimum[1]) * 0.5);
    let extent = SHADOW_MAP_EXTENT as f32;
    let texel_right = half_right * 2.0 / extent;
    let texel_up = half_up * 2.0 / extent;
    // Each axis snaps by its own texel size, since they now differ.
    let snapped = add(
        add(
            scale_vector(
                light_right,
                (((minimum[0] + maximum[0]) * 0.5) / texel_right).floor() * texel_right,
            ),
            scale_vector(
                light_up,
                (((minimum[1] + maximum[1]) * 0.5) / texel_up).floor() * texel_up,
            ),
        ),
        scale_vector(light_forward, (minimum[2] + maximum[2]) * 0.5),
    );

    let depth_half = ((maximum[2] - minimum[2]) * 0.5).max(1.0);
    let position = subtract(
        snapped,
        scale_vector(light_forward, depth_half + SHADOW_CASTER_HEADROOM),
    );
    let near_plane = 0.1;
    let far_plane = depth_half * 2.0 + SHADOW_CASTER_HEADROOM + 1.0;
    let position_forward = dot(position, light_forward);
    ShadowCascade {
        matrix: multiply_matrix(
            orthographic_extents(half_right, half_up, near_plane, far_plane),
            look_to(position, light_forward, reference),
        ),
        depth_range: far_plane - near_plane,
        // Bias follows the coarser axis, so it stays sufficient on both.
        texel_world: texel_right.max(texel_up),
        bounds: CascadeBounds {
            // Read back off the snapped centre rather than recomputed, so the cull volume is exactly
            // the volume the matrix above projects and cannot drift from it.
            center_right: dot(snapped, light_right),
            half_right,
            center_up: dot(snapped, light_up),
            half_up,
            near_forward: position_forward + near_plane,
            far_forward: position_forward + far_plane,
        },
    }
}

/// Fits every cascade to the current view. Recomputed per frame, since each cascade tracks the
/// camera rather than the map.
fn scene_shadow(camera: TerrainCamera, aspect: f32, lighting: TerrainLighting) -> SceneShadow {
    let light_forward = primary_light_forward(lighting);
    let shadowed_distance = camera.far_plane.min(SHADOW_MAX_DISTANCE);
    let mut cascades = [ShadowCascade {
        matrix: [[0.0; 4]; 4],
        depth_range: 1.0,
        texel_world: 1.0,
        bounds: CascadeBounds::EMPTY,
    }; SHADOW_CASCADE_COUNT];
    let mut near = 1.0_f32;
    for (cascade, split) in cascades.iter_mut().zip(SHADOW_CASCADE_SPLITS) {
        let far = (shadowed_distance * split).max(near + 1.0);
        *cascade = fit_cascade(camera, aspect, near, far, light_forward);
        // Overlap the next cascade slightly so a receiver near a split still finds coverage after
        // its normal offset nudges it across the boundary.
        near = far * 0.98;
    }
    let (right, up, _) = light_basis(light_forward);
    SceneShadow {
        cascades,
        light_basis: [right, up, light_forward],
    }
}

fn look_to(position: [f32; 3], forward: [f32; 3], up: [f32; 3]) -> [[f32; 4]; 4] {
    let forward = normalize(forward);
    let right = normalize(cross(forward, up));
    let camera_up = cross(right, forward);
    [
        [right[0], camera_up[0], -forward[0], 0.0],
        [right[1], camera_up[1], -forward[1], 0.0],
        [right[2], camera_up[2], -forward[2], 0.0],
        [
            -dot(right, position),
            -dot(camera_up, position),
            dot(forward, position),
            1.0,
        ],
    ]
}

/// Inverse of a 4x4 column-major matrix, or the identity if it is singular.
///
/// Gauss-Jordan elimination with partial pivoting, rather than an expanded cofactor formula. The
/// expansion is the faster route and this runs once per frame, so the deciding factor is that every
/// published cofactor expansion is written for one indexing convention and silently produces the
/// inverse of the transpose under the other. Elimination reads the same in either convention because
/// it only ever refers to whole rows.
///
/// Returning the identity on a singular input is deliberate: the caller is filling a uniform read at
/// every pixel, and a matrix of infinities there would take the whole frame with it, whereas the
/// identity merely misplaces reconstruction for a frame that is already degenerate.
fn invert_matrix(matrix: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    // Row-major working copy augmented with the identity: `rows[r][c]` is row `r`, column `c`, and
    // the input is column-major, so the transpose happens here and again on the way out.
    let mut rows = [[0.0_f64; 8]; 4];
    for (row_index, row) in rows.iter_mut().enumerate() {
        for column in 0..4 {
            row[column] = f64::from(matrix[column][row_index]);
        }
        row[4 + row_index] = 1.0;
    }
    for pivot in 0..4 {
        // Partial pivoting: a projection matrix has zeroes on the diagonal of its last two rows, so
        // eliminating without a row swap divides by zero on entirely ordinary input.
        let mut best = pivot;
        for candidate in pivot + 1..4 {
            if rows[candidate][pivot].abs() > rows[best][pivot].abs() {
                best = candidate;
            }
        }
        if rows[best][pivot] == 0.0 {
            return IDENTITY_MATRIX;
        }
        rows.swap(pivot, best);
        let scale = 1.0 / rows[pivot][pivot];
        for value in &mut rows[pivot] {
            *value *= scale;
        }
        for row_index in 0..4 {
            if row_index == pivot {
                continue;
            }
            let factor = rows[row_index][pivot];
            if factor == 0.0 {
                continue;
            }
            let pivot_row = rows[pivot];
            for (value, scaled) in rows[row_index].iter_mut().zip(pivot_row) {
                *value -= factor * scaled;
            }
        }
    }
    let mut result = [[0.0_f32; 4]; 4];
    for (row_index, row) in rows.iter().enumerate() {
        for column in 0..4 {
            #[allow(clippy::cast_possible_truncation)]
            let value = row[4 + column] as f32;
            if !value.is_finite() {
                return IDENTITY_MATRIX;
            }
            // Back to column-major.
            result[column][row_index] = value;
        }
    }
    result
}

const IDENTITY_MATRIX: [[f32; 4]; 4] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

fn multiply_matrix(left: [[f32; 4]; 4], right: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut result = [[0.0; 4]; 4];
    for column in 0..4 {
        for row in 0..4 {
            result[column][row] = (0..4)
                .map(|index| left[index][row] * right[column][index])
                .sum();
        }
    }
    result
}

/// Packs one cascade for the caster passes, which each see a single cascade at a time.
fn cascade_uniform_bytes(cascade: ShadowCascade, time: f32) -> [u8; 80] {
    let mut bytes = [0; 80];
    for (target, value) in
        bytes
            .chunks_exact_mut(4)
            .zip(cascade.matrix.into_iter().flatten().chain([
                time,
                cascade.depth_range,
                cascade.texel_world,
                0.0,
            ]))
    {
        target.copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

/// Packs every cascade for the receiver passes, which select among them per pixel.
fn shadow_uniform_bytes(shadow: &SceneShadow, time: f32) -> [u8; SHADOW_UNIFORM_LEN] {
    let mut bytes = [0; SHADOW_UNIFORM_LEN];
    for (target, cascade) in bytes.chunks_exact_mut(80).zip(shadow.cascades) {
        target.copy_from_slice(&cascade_uniform_bytes(cascade, time));
    }
    bytes
}

#[derive(Clone, Copy)]
struct CameraUniformInput {
    matrix: [[f32; 4]; 4],
    /// Inverse of `matrix`. The deferred passes reconstruct each pixel's world position from scene
    /// depth through this, so nothing has to store position in a G-buffer target.
    inverse_matrix: [[f32; 4]; 4],
    position: [f32; 3],
    time: f32,
    viewport: [f32; 2],
    detail_fade_uv: [f32; 2],
    caustic_animation: [f32; 2],
    water_material: [f32; 4],
    water_surface: [f32; 4],
    water_motion: [f32; 4],
    lighting: TerrainLighting,
}

fn camera_bytes(input: &CameraUniformInput) -> [u8; 368] {
    let CameraUniformInput {
        matrix,
        inverse_matrix,
        position,
        time,
        viewport,
        detail_fade_uv,
        caustic_animation,
        water_material,
        water_surface,
        water_motion,
        lighting,
    } = *input;
    let mut bytes = [0; 368];
    let values = matrix
        .into_iter()
        .flatten()
        .chain([position[0], position[1], position[2], time])
        .chain([
            viewport[0],
            viewport[1],
            1.0 / viewport[0],
            1.0 / viewport[1],
        ])
        .chain([
            detail_fade_uv[0],
            detail_fade_uv[1],
            caustic_animation[0],
            caustic_animation[1],
        ])
        .chain(water_material)
        .chain(water_surface)
        .chain(water_motion)
        .chain(lighting.lights().into_iter().flat_map(|light| {
            let ambient = light.ambient();
            let diffuse = light.diffuse();
            let direction = light.source_direction();
            [
                ambient[0],
                ambient[1],
                ambient[2],
                0.0,
                diffuse[0],
                diffuse[1],
                diffuse[2],
                0.0,
                direction[0],
                direction[1],
                direction[2],
                0.0,
            ]
        }))
        .chain(inverse_matrix.into_iter().flatten());
    for (index, value) in values.enumerate() {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

/// Exact bit pattern of a matrix, for deciding whether a cascade may be reused. Compared as bits
/// rather than floats so the test is an identity check with no equality edge cases.
fn matrix_bits(matrix: [[f32; 4]; 4]) -> [u32; 16] {
    let mut bits = [0; 16];
    for (target, value) in bits.iter_mut().zip(matrix.into_iter().flatten()) {
        *target = value.to_bits();
    }
    bits
}

fn add(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] + right[0], left[1] + right[1], left[2] + right[2]]
}

fn subtract(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn scale_vector(value: [f32; 3], scale: f32) -> [f32; 3] {
    [value[0] * scale, value[1] * scale, value[2] * scale]
}

fn add_scaled(target: &mut [f32; 3], direction: [f32; 3], scale: f32) {
    for axis in 0..3 {
        target[axis] += direction[axis] * scale;
    }
}

fn dot(left: [f32; 3], right: [f32; 3]) -> f32 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn cross(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn normalize(value: [f32; 3]) -> [f32; 3] {
    let length = dot(value, value).sqrt().max(f32::EPSILON);
    [value[0] / length, value[1] / length, value[2] / length]
}

fn detail_projection_scale(viewport_height: f32) -> f32 {
    viewport_height * TERRAIN_CELL_WORLD_SIZE * DETAIL_SCREEN_OVERSAMPLE
        / (2.0 * (CAMERA_VERTICAL_FOV * 0.5).tan())
}

fn detail_fade_distances(viewport_height: f32) -> [f32; 2] {
    let fine_end = detail_projection_scale(viewport_height.max(1.0)) / 16.0;
    [fine_end * DETAIL_FADE_START_RATIO, fine_end]
}

fn ray_distance_for_view_depth(
    direction: [f32; 3],
    forward: [f32; 3],
    maximum_depth: f32,
) -> Option<f32> {
    let forward_scale = dot(direction, forward);
    (forward_scale > f32::EPSILON).then_some(maximum_depth / forward_scale)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        CascadeBounds, ROAD_DEPTH_BIAS, ROTATE_CLICK_MAX_HOLD, ROTATE_CLICK_SLOP_PIXELS,
        SCROLL_ANCHOR_DEAD_ZONE_PIXELS, SCROLL_ANCHOR_FULL_PIXELS, SHADOW_CASCADE_BYTES,
        SHADOW_CASCADE_COUNT, SHADOW_CASTER_HEADROOM, SHADOW_UNIFORM_BYTES, SOURCE_ROAD_MIP_LEVELS,
        SceneShadow, ShadowCascade, TerrainCamera, TerrainInput, anchor_scroll_request,
        cascade_uniform_bytes, dot, gray_mip, invert_matrix, look_to, multiply_matrix,
        orthographic, perspective, quantize_cascade_extent, ray_distance_for_view_depth,
        rotate_release_is_click, scene_shadow, shadow_uniform_bytes, source_road_mips,
        terrain_color_targets, terrain_viewer_title,
    };
    use cic_camera::{CameraIntent, RtsCamera, RtsCameraProfile};

    use crate::TerrainLighting;

    /// The caster passes bind one cascade and the receivers bind the whole array, so the two
    /// minimum binding sizes are deliberately different, and nothing else forces the receiver
    /// array to stay a whole number of cascades.
    ///
    /// Whether the GPU accepts an exactly-sized cascade buffer against the caster layout used to be
    /// checked here with a device of its own, because the caster bind groups were only built inside
    /// the windowed constructor and no headless test could reach it. `capture_map_view` now builds
    /// every one of those bind groups on a real device, so that half lives in
    /// `tests/gpu_capture.rs` and this stays the CPU-side arithmetic guard.
    #[test]
    fn caster_cascade_bind_group_accepts_a_single_cascade_buffer() {
        assert_eq!(
            SHADOW_UNIFORM_BYTES,
            SHADOW_CASCADE_BYTES * SHADOW_CASCADE_COUNT as u64,
            "receiver array size must stay a whole number of cascades"
        );
    }

    #[test]
    fn road_texture_inputs_keep_at_most_three_total_levels() {
        assert_eq!(SOURCE_ROAD_MIP_LEVELS, 3);
        for (width, height, expected) in [
            (1, 1, &[][..]),
            (2, 1, &[(1, 1)][..]),
            (3, 2, &[(1, 1)][..]),
            (4, 4, &[(2, 2), (1, 1)][..]),
            (8, 4, &[(4, 2), (2, 1)][..]),
            (8, 8, &[(4, 4), (2, 2)][..]),
        ] {
            let pixels = vec![255; width as usize * height as usize * 4];
            let mips = source_road_mips(width, height, &pixels).expect("road mips");
            let dimensions = mips
                .iter()
                .map(|mip| (mip.width, mip.height))
                .collect::<Vec<_>>();
            assert_eq!(dimensions, expected, "{width}x{height}");
            assert!(mips.iter().all(|mip| !mip.rgba.is_empty()));
        }

        assert!(source_road_mips(2, 2, &[255; 15]).is_err());
        assert!(source_road_mips(0, 2, &[]).is_err());
    }

    #[test]
    fn viewer_diagnostic_defaults_and_title_inputs_are_exact() {
        assert_eq!(ROAD_DEPTH_BIAS.constant, -2);
        assert_eq!(ROAD_DEPTH_BIAS.slope_scale.to_bits(), (-1.0_f32).to_bits());
        assert_eq!(ROAD_DEPTH_BIAS.clamp.to_bits(), 0.0_f32.to_bits());
        assert_eq!(
            terrain_viewer_title("map", false, false),
            "map | WASD/arrows scroll, RMB hold to scroll, wheel zoom, MMB rotate or click to face north, R reset, Esc close"
        );
        assert_eq!(
            terrain_viewer_title("map", false, true),
            "map | WASD/arrows scroll, RMB hold to scroll, wheel zoom, MMB rotate or click to face north, R reset, M wireframe, Esc close"
        );
        assert_eq!(
            terrain_viewer_title("map", true, false),
            "map [wireframe] | WASD/arrows scroll, RMB hold to scroll, wheel zoom, MMB rotate or click to face north, R reset, Esc close"
        );
        assert_eq!(
            terrain_viewer_title("map", true, true),
            "map [wireframe] | WASD/arrows scroll, RMB hold to scroll, wheel zoom, MMB rotate or click to face north, R reset, M wireframe, Esc close"
        );
    }

    /// The deferred passes reconstruct every pixel's world position through this inverse, so an
    /// error here does not look like an error: shadows and occlusion simply stop finding their
    /// cascades and the frame reads as unshadowed rather than as broken. A cofactor expansion under
    /// the wrong indexing convention did exactly that, which is why the round trip is pinned.
    #[test]
    fn the_camera_inverse_round_trips_a_world_position_through_clip_space() {
        let camera = TerrainCamera {
            position: [1_400.0, 2_150.0, 260.0],
            yaw: 0.6,
            pitch: -0.65,
            far_plane: 84_000.0,
        };
        let matrix = camera.view_projection(1.6);
        let inverse = invert_matrix(matrix);

        // The inverse view-projection maps the clip-space origin to the eye, so its translation
        // column is the camera position. Well conditioned, unlike the product against the forward
        // matrix: that cancels terms of order `position * far_plane`, which at these magnitudes
        // leaves a quarter of a unit of f32 rounding in the translation column and says nothing
        // about whether the inverse is right.
        for (axis, expected) in camera.position.into_iter().enumerate() {
            let value = inverse[3][axis] / inverse[3][3];
            assert!(
                (value - expected).abs() < 1.0,
                "inverse translation column axis {axis} was {value}, expected the eye at {expected}"
            );
        }

        // The property the shaders rely on: a world point projected to normalized device coordinates
        // and reconstructed through the inverse comes back where it started.
        let transform = |m: [[f32; 4]; 4], point: [f32; 3]| {
            [0, 1, 2, 3]
                .map(|row| (0..3).map(|axis| m[axis][row] * point[axis]).sum::<f32>() + m[3][row])
        };
        for world in [
            [1_400.0_f32, 2_150.0, 20.0],
            [1_500.0, 2_000.0, 45.0],
            [1_100.0, 2_400.0, 0.0],
            [1_800.0, 2_600.0, 120.0],
        ] {
            let clip = transform(matrix, world);
            assert!(clip[3] > 0.0, "{world:?} must be in front of the camera");
            let ndc = [clip[0] / clip[3], clip[1] / clip[3], clip[2] / clip[3]];
            let restored = transform(inverse, ndc);
            for (axis, expected) in world.into_iter().enumerate() {
                let value = restored[axis] / restored[3];
                // A third of a unit bounds this round trip, which transforms twice in f32 against a
                // far plane 84,000 units out. The shader transforms once, from a depth buffer the
                // rasterizer filled, where the error is depth quantization alone: about
                // `distance^2 * 6e-8` world units at this projection's 1.0 near plane, so a few
                // thousandths of a unit in the near field. Either way it is well inside the
                // one-to-two units the half-float position target lost everywhere.
                assert!(
                    (value - expected).abs() < 0.34,
                    "axis {axis} came back as {value}, expected {expected}"
                );
            }
        }
    }

    #[test]
    fn a_singular_camera_inverse_falls_back_to_the_identity() {
        // Nothing renders from a degenerate matrix, but a uniform full of infinities would take the
        // rest of the frame with it, so the fallback is pinned rather than assumed.
        let identity = invert_matrix([[0.0; 4]; 4]);
        for (column, values) in identity.into_iter().enumerate() {
            for (row, value) in values.into_iter().enumerate() {
                let expected = if column == row { 1.0 } else { 0.0 };
                assert!((value - expected).abs() < 1.0e-6);
            }
        }
    }

    #[test]
    fn anchored_scrolling_holds_still_inside_the_dead_zone_and_answers_a_short_nudge() {
        let magnitude = |offset: [f32; 2]| {
            let request = anchor_scroll_request(offset);
            request[0].hypot(request[1])
        };
        // Exactly still, not merely small: a creeping anchor is the failure being ruled out here.
        let still = |offset: [f32; 2]| {
            let request = anchor_scroll_request(offset);
            request[0].to_bits() == 0 && request[1].to_bits() == 0
        };

        // Inside the dead zone, and exactly on its edge, an anchored press must not creep.
        assert!(still([0.0, 0.0]));
        assert!(still([SCROLL_ANCHOR_DEAD_ZONE_PIXELS, 0.0]));
        assert!(still([0.0, -SCROLL_ANCHOR_DEAD_ZONE_PIXELS + 1.0]));

        // A short nudge past the dead zone already asks for a usable share of the full rate; a
        // linear ramp gave under a tenth here, which is what made scrolling feel late and floaty.
        let nudge = magnitude([20.0, 0.0]);
        assert!(nudge > 0.4, "a 20px nudge only asked for {nudge}");

        // Full rate is reached at the documented offset and never exceeds it, in any direction.
        assert!((magnitude([SCROLL_ANCHOR_FULL_PIXELS, 0.0]) - 1.0).abs() < 1.0e-4);
        for offset in [
            [SCROLL_ANCHOR_FULL_PIXELS * 4.0, 0.0],
            [0.0, -SCROLL_ANCHOR_FULL_PIXELS * 4.0],
            [900.0, 900.0],
        ] {
            assert!(
                (magnitude(offset) - 1.0).abs() < 1.0e-4,
                "{offset:?} should cap at full rate"
            );
        }

        // Rightward on screen pans right; downward on screen pans backward, because screen up is
        // negative while the camera's forward axis is positive.
        let right = anchor_scroll_request([40.0, 0.0]);
        assert!(right[0] > 0.0 && right[1].abs() < 1.0e-6, "{right:?}");
        let down = anchor_scroll_request([0.0, 40.0]);
        assert!(down[1] < 0.0 && down[0].abs() < 1.0e-6, "{down:?}");

        // The ramp still rises with distance, so the far travel keeps its fine control.
        let mut previous = 0.0;
        for distance in [10.0, 25.0, 45.0, 70.0, 89.0] {
            let current = magnitude([distance, 0.0]);
            assert!(
                current > previous,
                "{distance}px gave {current} <= {previous}"
            );
            previous = current;
        }

        // A malformed cursor position must not become a NaN pan request.
        assert!(still([f32::NAN, 0.0]));
        assert!(still([f32::INFINITY, 0.0]));
    }

    #[test]
    fn only_a_brief_and_still_middle_click_faces_the_camera_north() {
        let brief = ROTATE_CLICK_MAX_HOLD / 2;
        let held = ROTATE_CLICK_MAX_HOLD + Duration::from_millis(120);
        let still = ROTATE_CLICK_SLOP_PIXELS / 2.0;
        let moved = ROTATE_CLICK_SLOP_PIXELS + 4.0;

        assert!(
            rotate_release_is_click(brief, still),
            "a tap resets rotation"
        );
        assert!(
            !rotate_release_is_click(brief, moved),
            "a quick flick that travelled is a rotation"
        );
        // The reported fault: holding the button while barely moving reset the view far too often.
        assert!(
            !rotate_release_is_click(held, still),
            "a long hold is aiming a rotation, however still it stayed"
        );
        assert!(!rotate_release_is_click(held, moved));

        // Both limits are inclusive, so a release exactly on either one is still a click.
        assert!(rotate_release_is_click(
            ROTATE_CLICK_MAX_HOLD,
            ROTATE_CLICK_SLOP_PIXELS
        ));
        assert!(!rotate_release_is_click(
            ROTATE_CLICK_MAX_HOLD + Duration::from_millis(1),
            0.0
        ));
    }

    #[test]
    fn shadow_cascade_uniform_is_stably_packed_in_cascade_order() {
        let mut cascades = [ShadowCascade {
            matrix: [[0.0; 4]; 4],
            depth_range: 1.0,
            texel_world: 1.0,
            bounds: CascadeBounds::EMPTY,
        }; SHADOW_CASCADE_COUNT];
        for (index, cascade) in cascades.iter_mut().enumerate() {
            let radius = 100.0 * 3.0_f32.powi(i32::try_from(index).expect("cascade index"));
            *cascade = ShadowCascade {
                matrix: orthographic(radius, 0.1, radius * 4.0),
                depth_range: radius * 4.0 - 0.1,
                texel_world: radius * 2.0 / 2_048.0,
                bounds: CascadeBounds::EMPTY,
            };
        }
        let bytes = shadow_uniform_bytes(
            &SceneShadow {
                cascades,
                light_basis: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            },
            2.5,
        );
        assert_eq!(bytes.len() as u64, SHADOW_UNIFORM_BYTES);
        for (index, cascade) in cascades.into_iter().enumerate() {
            assert!(cascade.matrix.into_iter().flatten().all(f32::is_finite));
            let base = index * 80;
            assert_eq!(
                &bytes[base..base + 80],
                &cascade_uniform_bytes(cascade, 2.5),
                "cascade {index} slice must match its standalone packing"
            );
            assert_eq!(
                u32::from_le_bytes(bytes[base + 64..base + 68].try_into().expect("time bytes")),
                2.5_f32.to_bits()
            );
            assert_eq!(
                u32::from_le_bytes(
                    bytes[base + 68..base + 72]
                        .try_into()
                        .expect("depth range bytes")
                ),
                cascade.depth_range.to_bits()
            );
            assert_eq!(
                u32::from_le_bytes(
                    bytes[base + 72..base + 76]
                        .try_into()
                        .expect("texel extent bytes")
                ),
                cascade.texel_world.to_bits()
            );
        }
    }

    /// The light-space box fit is only safe because its extents are quantized. A sphere is
    /// invariant to camera yaw; a box is not, so an unquantized extent would change every frame as
    /// the view turned, rescaling every texel and making shadow edges re-quantize -- crawl that
    /// texel snapping cannot remove, because snapping only cancels translation. The ladder has to
    /// hold still across small changes while never returning less than it was asked to cover.
    #[test]
    fn cascade_extent_quantization_covers_its_input_and_holds_still() {
        for base in [1.0_f32, 7.5, 42.0, 137.0, 900.0, 3_000.0] {
            let quantized = quantize_cascade_extent(base);
            assert!(
                quantized >= base,
                "quantized {quantized} must still cover {base}"
            );
            assert!(
                quantized < base * 1.10,
                "quantized {quantized} gives away too much of {base}"
            );
            // A one-percent wobble in the fitted extent must not move the ladder rung, which is
            // what keeps the projection stable while the camera turns slowly.
            let wobbled = quantize_cascade_extent(base * 0.995);
            assert!(
                wobbled <= quantized,
                "a smaller input must not select a larger rung: {wobbled} vs {quantized}"
            );
        }
        // Degenerate inputs must not produce a zero or non-finite extent, which would make the
        // projection divide by zero.
        for degenerate in [0.0_f32, -1.0, f32::NAN, f32::INFINITY] {
            let quantized = quantize_cascade_extent(degenerate);
            assert!(
                quantized.is_finite() && quantized > 0.0,
                "degenerate input {degenerate} produced {quantized}"
            );
        }
    }

    #[test]
    fn near_cascade_resolves_far_more_finely_than_the_far_cascade() {
        // The whole point of cascades here: the near slice must land dramatically more texels on
        // the ground than a whole-map slice ever could, and each cascade's bias follows its own
        // texel extent rather than a shared constant.
        let camera = TerrainCamera {
            position: [0.0, 0.0, 200.0],
            yaw: 0.6,
            pitch: -0.7,
            far_plane: 10_000.0,
        };
        let shadow = scene_shadow(camera, 16.0 / 9.0, TerrainLighting::preview());
        let extents = shadow.cascades.map(|cascade| cascade.texel_world);
        assert!(
            extents
                .iter()
                .all(|extent| extent.is_finite() && *extent > 0.0),
            "every cascade needs a usable texel extent: {extents:?}"
        );
        assert!(
            extents.windows(2).all(|pair| pair[0] < pair[1]),
            "cascades must grow coarser outward: {extents:?}"
        );
        assert!(
            extents[0] < 0.25,
            "near cascade should resolve well under a quarter world unit per texel: {extents:?}"
        );
        // The outermost cascade is what distant blockiness is made of, and it is the one a split
        // scheme silently ruins: leaving most of the shadowed range to it makes it coarser than a
        // single whole-map slice would have been. A whole-map 4096-square slice reached roughly
        // 1.04 world units per texel, so the outermost cascade has to stay near that to be worth
        // having. Bounding only the near cascade is what let a 3.16 regression ship.
        let outermost = extents[SHADOW_CASCADE_COUNT - 1];
        assert!(
            outermost < 1.4,
            "outermost cascade must not be coarser than a whole-map slice: {extents:?}"
        );
        // Density should step by a roughly constant factor, not pile the range into one cascade.
        for pair in extents.windows(2) {
            let ratio = pair[1] / pair[0];
            assert!(
                (1.5..=5.0).contains(&ratio),
                "cascade density should step geometrically, got ratio {ratio} in {extents:?}"
            );
        }
        // Every cascade must reach far enough above its receiver region to contain a tall caster.
        // The near cascade's own radius is only tens of units, so without the headroom a structure
        // standing in it is clipped out of precisely the cascade a nearby receiver selects.
        for (index, cascade) in shadow.cascades.into_iter().enumerate() {
            assert!(cascade.matrix.into_iter().flatten().all(f32::is_finite));
            assert!(
                cascade.depth_range > SHADOW_CASTER_HEADROOM,
                "cascade {index} depth range {} must exceed the caster headroom",
                cascade.depth_range
            );
        }
    }

    /// Per-cascade caster culling is only allowed to be invisible if the volume it tests against is
    /// exactly the volume the cascade's matrix projects. This checks that equivalence directly: with
    /// a zero-radius caster, `accepts` must agree with clipping the projected point, for every
    /// cascade of a real fit and over a grid of world points that straddles all of them.
    ///
    /// Without this, a cull volume could drift from the projection under some future change to the
    /// fit and start dropping casters that do write texels — which reads as shadows popping in and
    /// out as the camera moves, the kind of bug that is easy to ship and hard to attribute.
    #[test]
    #[allow(clippy::cast_precision_loss)]
    fn caster_cull_volume_is_exactly_what_each_cascade_projects() {
        let camera = TerrainCamera {
            position: [1_400.0, 900.0, 260.0],
            yaw: 0.7,
            pitch: -0.65,
            far_plane: 4_000.0,
        };
        let shadow = scene_shadow(camera, 1.6, TerrainLighting::preview());
        let mut inside_seen = 0;
        let mut outside_seen = 0;
        for cascade in shadow.cascades {
            for x in (-400..2_400).step_by(157) {
                for y in (-400..2_400).step_by(163) {
                    for z in (-100..500).step_by(97) {
                        let point = [x as f32, y as f32, z as f32];
                        let light_space = [
                            dot(point, shadow.light_basis[0]),
                            dot(point, shadow.light_basis[1]),
                            dot(point, shadow.light_basis[2]),
                        ];
                        let clip = (0..3)
                            .map(|row| {
                                (0..3)
                                    .map(|column| cascade.matrix[column][row] * point[column])
                                    .sum::<f32>()
                                    + cascade.matrix[3][row]
                            })
                            .collect::<Vec<_>>();
                        let projects_inside = clip[0].abs() <= 1.0
                            && clip[1].abs() <= 1.0
                            && (0.0..=1.0).contains(&clip[2]);
                        // Points landing within a whisker of a plane are skipped: the two routes to
                        // the same predicate divide by different quantities, so a point exactly on a
                        // boundary can round either way without either route being wrong.
                        let margin = (1.0 - clip[0].abs())
                            .abs()
                            .min((1.0 - clip[1].abs()).abs())
                            .min(clip[2].abs())
                            .min((1.0 - clip[2]).abs());
                        if margin < 1e-4 {
                            continue;
                        }
                        if projects_inside {
                            inside_seen += 1;
                        } else {
                            outside_seen += 1;
                        }
                        assert_eq!(
                            cascade.bounds.accepts(light_space, 0.0),
                            projects_inside,
                            "point {point:?} clip {clip:?}"
                        );
                    }
                }
            }
        }
        assert!(
            inside_seen > 0 && outside_seen > 0,
            "the grid must straddle the cascades to prove anything: {inside_seen} in, {outside_seen} out"
        );
    }

    /// A caster's radius may only ever widen the volume, never narrow it: a sphere is accepted
    /// whenever its centre is, and a sphere large enough to reach the cascade is accepted even when
    /// its centre is well outside. Otherwise a tall or wide model would lose the shadow it casts
    /// across a cascade boundary.
    #[test]
    fn caster_cull_radius_only_ever_widens_the_volume() {
        let bounds = CascadeBounds {
            center_right: 10.0,
            half_right: 40.0,
            center_up: -5.0,
            half_up: 25.0,
            near_forward: 100.0,
            far_forward: 900.0,
        };
        assert!(bounds.accepts([10.0, -5.0, 500.0], 0.0));
        for radius in [0.0, 1.0, 60.0, 400.0] {
            assert!(bounds.accepts([10.0, -5.0, 500.0], radius));
        }
        // Just outside on each axis with no radius, reached once the radius covers the gap.
        for outside in [
            [10.0 + 45.0, -5.0, 500.0],
            [10.0, -5.0 - 30.0, 500.0],
            [10.0, -5.0, 90.0],
            [10.0, -5.0, 910.0],
        ] {
            assert!(!bounds.accepts(outside, 0.0), "{outside:?}");
            assert!(bounds.accepts(outside, 12.0), "{outside:?}");
        }
    }

    /// Sway is bounded rather than sampled, so the bound has to actually hold at every time a frame
    /// could ask for. A bound that was too small would clip a swaying tree out of a cascade it still
    /// reaches into.
    #[test]
    #[allow(clippy::cast_precision_loss)]
    fn tree_sway_offset_bound_holds_at_every_sampled_time() {
        for placement in 0_u32..10 {
            let sway = crate::TreeSwayPresentation::zero_hour_legacy_default(placement);
            let bound = sway.maximum_offset_fraction();
            assert!(bound > 0.0, "family {placement} must sway at all");
            for step in 0..2_000 {
                let seconds = step as f32 * 0.011;
                let offset = sway.offset_at(seconds, 1.0);
                let length = offset
                    .into_iter()
                    .map(|value| value * value)
                    .sum::<f32>()
                    .sqrt();
                assert!(
                    length <= bound + 1e-6,
                    "family {placement} at {seconds}s displaced {length} past its {bound} bound"
                );
            }
        }
    }

    #[test]
    fn shadow_depth_slack_stays_in_world_units_across_every_fitted_frustum() {
        // Mirrors `SHADOW_DEPTH_SLACK_TEXELS` in `terrain_deferred.wgsl` and the epsilon it
        // replaced. The shader biases by `texel_world * slack / depth_range` in normalized depth,
        // so the world-space consequence is `texel_world * slack` and is independent of how large
        // the fitted frustum grew. The legacy fixed normalized-depth epsilon instead scaled with
        // the frustum, which is what erased short occluders' shadows on larger maps.
        const LEGACY_NORMALIZED_EPSILON: f32 = 0.0015;
        const SLACK_TEXELS: f32 = 0.5;
        // Half-diagonals for a small skirmish map, a mid-size map, and a large four-player map.
        for radius in [141.0_f32, 1_414.0, 2_828.0] {
            let depth_range = radius * 4.0 - 0.1;
            let texel_world = radius * 2.0 / 2_048.0;
            let world_slack = texel_world * SLACK_TEXELS;
            let legacy_world_slack = LEGACY_NORMALIZED_EPSILON * depth_range;
            assert!(
                world_slack < legacy_world_slack,
                "radius {radius} regressed: {world_slack} >= {legacy_world_slack}"
            );
            assert!(
                world_slack < crate::terrain::TERRAIN_XY_SCALE,
                "radius {radius} slack {world_slack} exceeds one terrain cell"
            );
        }
    }

    #[test]
    fn terrain_input_constructor_and_every_key_transition_are_exact() {
        use winit::keyboard::KeyCode;

        let mut input = TerrainInput::default();
        assert_eq!(input, TerrainInput(0));
        let mappings = [
            (KeyCode::KeyW, TerrainInput::FORWARD),
            (KeyCode::KeyS, TerrainInput::BACKWARD),
            (KeyCode::KeyA, TerrainInput::LEFT),
            (KeyCode::KeyD, TerrainInput::RIGHT),
            (KeyCode::ArrowUp, TerrainInput::FORWARD),
            (KeyCode::ArrowDown, TerrainInput::BACKWARD),
            (KeyCode::ArrowLeft, TerrainInput::LEFT),
            (KeyCode::ArrowRight, TerrainInput::RIGHT),
        ];
        for (key, mask) in mappings {
            input = TerrainInput::default();
            input.set(key, true);
            assert_eq!(input, TerrainInput(mask), "{key:?} press");
            assert!(input.active(mask));
            input.set(key, false);
            assert_eq!(input, TerrainInput::default(), "{key:?} release");
        }

        input.set(KeyCode::KeyW, true);
        input.set(KeyCode::KeyS, true);
        assert_eq!(
            input,
            TerrainInput(TerrainInput::FORWARD | TerrainInput::BACKWARD)
        );
        input.set(KeyCode::KeyM, true);
        assert_eq!(
            input,
            TerrainInput(TerrainInput::FORWARD | TerrainInput::BACKWARD)
        );
    }

    #[test]
    fn edge_blending_writes_only_albedo() {
        let targets = terrain_color_targets(Some(wgpu::BlendState::ALPHA_BLENDING), false);
        let albedo = targets[0].as_ref().expect("albedo target");
        assert!(albedo.blend.is_some());
        assert_eq!(albedo.write_mask, wgpu::ColorWrites::ALL);
        for geometry in &targets[1..] {
            let geometry = geometry.as_ref().expect("geometry target");
            assert!(geometry.blend.is_none());
            assert!(geometry.write_mask.is_empty());
        }
    }

    #[test]
    fn caustic_mips_average_odd_linear_frames_without_dropping_edges() {
        let (width, height, mip) = gray_mip(3, 2, &[0, 30, 90, 120, 150, 210]).expect("gray mip");
        assert_eq!((width, height), (1, 1));
        assert_eq!(mip, [100]);
    }

    #[test]
    fn perspective_view_matrix_is_finite_and_pose_round_trips_through_yaw_and_pitch() {
        let camera = TerrainCamera {
            position: [10.0, 20.0, 30.0],
            yaw: 0.25,
            pitch: -0.5,
            far_plane: 10_000.0,
        };
        let matrix = multiply_matrix(
            perspective(1.0, 16.0 / 9.0, 1.0, camera.far_plane),
            look_to(camera.position, camera.forward(), [0.0, 0.0, 1.0]),
        );
        assert!(matrix.into_iter().flatten().all(f32::is_finite));

        // The camera model owns movement now, and this type only carries the pose it produces.
        // Yaw and pitch are recovered from the forward vector, so the round trip has to be exact:
        // the shadow cascades and the terrain detail selection both reason about those angles.
        let ground = cic_camera::FlatGround(12.0);
        let mut rts = RtsCamera::new(RtsCameraProfile::GENERALS_DEFAULT, [40.0, -25.0], &ground);
        rts.update(
            CameraIntent {
                pan: [1.0, 0.5],
                rotate: 40.0,
                zoom: -1.0,
                ..CameraIntent::default()
            },
            1.0 / 30.0,
            &ground,
        );
        let pose = rts.pose();
        let derived = TerrainCamera::from_pose(pose, 10_000.0);
        for (value, expected) in derived.position.into_iter().zip(pose.eye) {
            assert!(
                (value - expected).abs() < 1.0e-4,
                "eye survived: {derived:?}"
            );
        }
        for (value, expected) in derived.forward().into_iter().zip(pose.forward) {
            assert!(
                (value - expected).abs() < 1.0e-4,
                "forward round-tripped through yaw and pitch: {:?} vs {:?}",
                derived.forward(),
                pose.forward
            );
        }
        // A source-tilt camera must look downward, or the cascade fit would be aimed at the sky.
        assert!(derived.pitch < 0.0, "pitch was {}", derived.pitch);

        let focus_camera = TerrainCamera {
            position: [10.0, 20.0, 30.0],
            yaw: 0.0,
            pitch: -std::f32::consts::FRAC_PI_4,
            far_plane: 10_000.0,
        };
        for pitch in [-0.000_001, 0.0, 0.000_001] {
            let horizon_camera = TerrainCamera {
                pitch,
                ..focus_camera
            };
            let (minimum, maximum) = horizon_camera
                .viewport_ground_bounds(
                    ([-1_000.0, -1_000.0, 0.0], [1_000.0, 1_000.0, 100.0]),
                    16.0 / 9.0,
                )
                .expect("near-horizon frustum intersects terrain bounds");
            assert!(minimum.into_iter().chain(maximum).all(f32::is_finite));
            assert!(minimum[0] >= -1_000.0 && minimum[1] >= -1_000.0);
            assert!(maximum[0] <= 1_000.0 && maximum[1] <= 1_000.0);
            assert!((maximum[0] - 1_000.0).abs() < 0.001);
        }
    }

    #[test]
    fn shallow_view_detail_footprint_is_capped_before_the_horizon() {
        let camera = TerrainCamera {
            position: [0.0, 0.0, 200.0],
            yaw: 0.0,
            pitch: -0.1,
            far_plane: 10_000.0,
        };
        let terrain = ([-2_000.0, -2_000.0, 0.0], [2_000.0, 2_000.0, 100.0]);
        let (_, full_maximum) = camera
            .viewport_ground_bounds(terrain, 16.0 / 9.0)
            .expect("shallow frustum reaches terrain");
        let (limited_minimum, limited_maximum) = camera
            .viewport_ground_bounds_limited(terrain, 16.0 / 9.0, 650.0)
            .expect("foreground frustum reaches terrain");

        assert!(full_maximum[0] > limited_maximum[0] + 500.0);
        assert!(
            limited_minimum
                .into_iter()
                .chain(limited_maximum)
                .all(|value| { value.is_finite() && (-2_000.0..=2_000.0).contains(&value) })
        );
        assert!(limited_minimum[1] < -650.0 && limited_maximum[1] > 650.0);
        let diagonal = super::normalize([1.0, 1.0, 0.0]);
        let ray_distance = ray_distance_for_view_depth(diagonal, [1.0, 0.0, 0.0], 650.0)
            .expect("forward-facing ray");
        assert!(ray_distance > 650.0);
        assert!((super::dot(diagonal, [1.0, 0.0, 0.0]) * ray_distance - 650.0).abs() < 0.001);
    }

    #[test]
    fn limited_viewport_bounds_are_symmetric_after_half_turn() {
        let camera = TerrainCamera {
            position: [0.0, 0.0, 300.0],
            yaw: 0.37,
            pitch: -0.35,
            far_plane: 10_000.0,
        };
        let reverse = TerrainCamera {
            yaw: camera.yaw + std::f32::consts::PI,
            ..camera
        };
        let terrain = ([-4_000.0, -4_000.0, 0.0], [4_000.0, 4_000.0, 100.0]);
        let (forward_minimum, forward_maximum) = camera
            .viewport_ground_bounds_limited(terrain, 16.0 / 9.0, 1_200.0)
            .expect("forward footprint");
        let (reverse_minimum, reverse_maximum) = reverse
            .viewport_ground_bounds_limited(terrain, 16.0 / 9.0, 1_200.0)
            .expect("reverse footprint");
        for axis in 0..2 {
            assert!((forward_minimum[axis] + reverse_maximum[axis]).abs() < 0.01);
            assert!((forward_maximum[axis] + reverse_minimum[axis]).abs() < 0.01);
        }
    }
}
