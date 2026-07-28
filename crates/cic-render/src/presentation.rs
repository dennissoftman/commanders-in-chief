//! Windowed presentation: surface configuration, resize, and input.
//!
//! Presentation is the same deferred chain pointed at a swapchain instead of a capture target. Nothing
//! about the passes changes — which is why headless came first, and why a capture and a window cannot
//! disagree about what the renderer produces.
//!
//! # What is *not* here
//!
//! The event loop. This module owns a surface, its intermediate targets, and an input-to-intent
//! mapping; running a window is the caller's job. That boundary keeps every piece here testable —
//! resize, format selection, and input mapping all exercise without a window — and leaves the choice
//! of windowing library out of the renderer's API.

use cic_assets::Terrain;
use cic_camera::{CameraIntent, GroundHeight};

use crate::RenderError;
use crate::deferred::{DeferredFrame, DeferredRenderer, DeferredTargets};
use crate::model::ModelBatch;
use crate::terrain::TerrainRenderer;
use crate::water::WaterBody;

/// Adapts a [`Terrain`] to the camera's ground-height lookup.
///
/// The camera holds a height above the ground beneath its focus, and asks for that ground through a
/// trait so it never has to know what terrain is. This is the whole of the coupling between them.
#[derive(Debug, Clone, Copy)]
pub struct TerrainGround<'a>(pub &'a Terrain);

impl GroundHeight for TerrainGround<'_> {
    fn height_at(&self, x: f32, y: f32) -> Option<f32> {
        self.0.elevation_at_world(x, y)
    }
}

/// A configured surface with the deferred chain sized to it.
#[derive(Debug)]
pub struct SurfaceRenderer {
    surface: wgpu::Surface<'static>,
    configuration: wgpu::SurfaceConfiguration,
    targets: DeferredTargets,
    deferred: DeferredRenderer,
}

impl SurfaceRenderer {
    /// Configures a surface and builds the chain against it.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::NoSurfaceFormat`] when the surface offers no usable format, or a
    /// structured error from target allocation.
    pub fn new(
        context: &crate::GpuContext,
        terrain: &TerrainRenderer,
        surface: wgpu::Surface<'static>,
        width: u32,
        height: u32,
    ) -> Result<Self, RenderError> {
        let capabilities = surface.get_capabilities(context.adapter());
        let format = choose_format(&capabilities.formats).ok_or(RenderError::NoSurfaceFormat)?;
        let configuration = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: width.max(1),
            height: height.max(1),
            // Fifo, so presentation is vsynced and the frame loop cannot spin the GPU flat out for
            // frames nobody sees. A camera that feels wrong at an uncapped rate is a camera bug, not
            // something to hide behind more frames.
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: capabilities
                .alpha_modes
                .first()
                .copied()
                .unwrap_or(wgpu::CompositeAlphaMode::Auto),
            view_formats: vec![],
            // Auto, which every format in `formats` is guaranteed to support. Anything else has to be
            // checked against that format's own capability set first, and this renderer has no HDR
            // output path to justify one yet.
            color_space: wgpu::SurfaceColorSpace::Auto,
            desired_maximum_frame_latency: 2,
        };
        surface.configure(context.device(), &configuration);

        let targets = DeferredTargets::new(context, configuration.width, configuration.height)?;
        let deferred = DeferredRenderer::new(context, terrain, &targets, format)?;
        Ok(Self {
            surface,
            configuration,
            targets,
            deferred,
        })
    }

    /// Returns the surface's current size.
    #[must_use]
    pub const fn size(&self) -> (u32, u32) {
        (self.configuration.width, self.configuration.height)
    }

    /// Returns the layout a [`ModelBatch`] binds its materials through.
    #[must_use]
    pub const fn material_layout(&self) -> &wgpu::BindGroupLayout {
        self.deferred.material_layout()
    }

    /// Returns the layout a [`WaterBody`] binds its uniform through.
    #[must_use]
    pub const fn water_layout(&self) -> &wgpu::BindGroupLayout {
        self.deferred.water_layout()
    }

    /// Returns the format the composite writes.
    #[must_use]
    pub const fn format(&self) -> wgpu::TextureFormat {
        self.configuration.format
    }

    /// Resizes the surface and everything sized to it.
    ///
    /// The whole chain is rebuilt rather than just the surface. Every bind group in it holds views of
    /// the intermediate targets, so a target reallocated at a new size leaves those groups pointing at
    /// textures of the old one — which is a validation error at best and a garbled frame at worst.
    ///
    /// A zero dimension is ignored rather than treated as an error: minimising a window reports one,
    /// and it is not a failure, just a frame not worth drawing.
    ///
    /// # Errors
    ///
    /// Returns a structured error when the new targets cannot be allocated.
    pub fn resize(
        &mut self,
        context: &crate::GpuContext,
        terrain: &TerrainRenderer,
        width: u32,
        height: u32,
    ) -> Result<(), RenderError> {
        if width == 0 || height == 0 {
            return Ok(());
        }
        if width == self.configuration.width && height == self.configuration.height {
            return Ok(());
        }
        self.configuration.width = width;
        self.configuration.height = height;
        self.surface
            .configure(context.device(), &self.configuration);
        self.targets = DeferredTargets::new(context, width, height)?;
        self.deferred =
            DeferredRenderer::new(context, terrain, &self.targets, self.configuration.format)?;
        Ok(())
    }

    /// Renders one frame and presents it.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::SurfaceLost`] when the surface must be reconfigured before it can be
    /// drawn to again, or a structured error from uniform upload.
    pub fn render(
        &mut self,
        context: &crate::GpuContext,
        terrain: &TerrainRenderer,
        models: &[ModelBatch],
        water: &[WaterBody],
        mut frame: DeferredFrame,
    ) -> Result<(), RenderError> {
        // Every non-success case here is a "skip this frame" rather than an error. A resize, a
        // minimise, or a compositor hiccup all land in one of them, and treating any as fatal would
        // close the window on an entirely normal event.
        let surface_frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            // Suboptimal still draws: the frame is usable, and reconfiguring takes effect next time.
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                self.surface
                    .configure(context.device(), &self.configuration);
                frame
            }
            // Outdated and Lost need the surface reconfigured before the next attempt.
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface
                    .configure(context.device(), &self.configuration);
                return Ok(());
            }
            // Occluded means nothing would be seen; Timeout means the swapchain is busy. Both are
            // resolved by simply not drawing this frame.
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(RenderError::SurfaceLost(
                    "surface acquisition raised a validation error".to_owned(),
                ));
            }
        };
        let view = surface_frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        // The surface is authoritative about its own size, so its configuration wins over whatever the
        // caller put on the frame. A stale viewport here — one frame behind a resize, say — would
        // misplace every world position the lighting pass reconstructs.
        frame.viewport = [self.configuration.width, self.configuration.height];
        self.deferred
            .set_frame(context, terrain, models, water, frame)?;
        self.deferred
            .render(context, terrain, models, water, &self.targets, &view);
        // Presenting is the queue's operation in this API version, not the texture's.
        context.queue().present(surface_frame);
        Ok(())
    }
}

/// Picks a surface format, preferring sRGB.
///
/// sRGB matters here rather than being a preference: the composite writes linear colour, and an sRGB
/// surface applies the transfer function in hardware. Presenting the same linear values to a non-sRGB
/// surface displays them uncorrected, which looks markedly darker than the capture of the same frame —
/// a difference easy to mistake for a lighting bug.
#[must_use]
pub fn choose_format(available: &[wgpu::TextureFormat]) -> Option<wgpu::TextureFormat> {
    available
        .iter()
        .copied()
        .find(wgpu::TextureFormat::is_srgb)
        .or_else(|| available.first().copied())
}

/// Accumulates window input and drains it into one [`CameraIntent`] per frame.
///
/// Held state and per-frame state are deliberately separate. A key that is *down* contributes every
/// frame it stays down; a scroll notch or a mouse motion happens *once* and must not repeat, so those
/// are drained by [`Self::take_intent`] and the held keys are not.
///
/// One flag per direction rather than a packed bitset or a set of held actions: the directions are
/// genuinely independent, several are down at once in normal play, and a lint against many booleans is
/// aimed at types where they encode a state machine — which this is not.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Default)]
pub struct InputState {
    pan_west: bool,
    pan_east: bool,
    pan_south: bool,
    pan_north: bool,
    rotate_left: bool,
    rotate_right: bool,
    dragging: bool,
    drag: [f32; 2],
    zoom: f32,
    reset: bool,
    reset_rotation: bool,
}

/// A semantic action, so the caller maps its own keys and this stays free of any key-code type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Pan toward negative X.
    PanWest,
    /// Pan toward positive X.
    PanEast,
    /// Pan toward negative Y.
    PanSouth,
    /// Pan toward positive Y.
    PanNorth,
    /// Rotate anticlockwise.
    RotateLeft,
    /// Rotate clockwise.
    RotateRight,
    /// Return height and yaw to their starting values.
    Reset,
    /// Return yaw alone to its starting value.
    ResetRotation,
}

impl InputState {
    /// Records an action being pressed or released.
    ///
    /// `Reset` and `ResetRotation` latch on press and are cleared when the intent is taken, so a
    /// single tap produces exactly one reset however many frames the key stays down.
    pub const fn set_action(&mut self, action: Action, pressed: bool) {
        match action {
            Action::PanWest => self.pan_west = pressed,
            Action::PanEast => self.pan_east = pressed,
            Action::PanSouth => self.pan_south = pressed,
            Action::PanNorth => self.pan_north = pressed,
            Action::RotateLeft => self.rotate_left = pressed,
            Action::RotateRight => self.rotate_right = pressed,
            Action::Reset => {
                if pressed {
                    self.reset = true;
                }
            }
            Action::ResetRotation => {
                if pressed {
                    self.reset_rotation = true;
                }
            }
        }
    }

    /// Starts or ends a pointer drag.
    ///
    /// Ending a drag clears any motion accumulated but not yet taken, so releasing the button cannot
    /// leave one last frame of pan queued behind it.
    pub const fn set_dragging(&mut self, dragging: bool) {
        self.dragging = dragging;
        if !dragging {
            self.drag = [0.0, 0.0];
        }
    }

    /// Adds pointer motion in pixels. Ignored unless a drag is in progress.
    pub fn add_pointer_motion(&mut self, dx: f32, dy: f32) {
        if !self.dragging {
            return;
        }
        self.drag[0] += dx;
        self.drag[1] += dy;
    }

    /// Adds scroll input, positive toward the ground.
    pub fn add_scroll(&mut self, amount: f32) {
        if amount.is_finite() {
            self.zoom += amount;
        }
    }

    /// Returns whether a drag is in progress.
    #[must_use]
    pub const fn is_dragging(&self) -> bool {
        self.dragging
    }

    /// Drains one frame's input into a camera intent.
    ///
    /// Pan is normalized, so holding two directions at once cannot travel faster diagonally than
    /// along an axis — the classic bug of summing axis inputs without clamping.
    pub fn take_intent(&mut self, drag_pixels_to_units: f32) -> CameraIntent {
        let axis = |negative: bool, positive: bool| f32::from(positive) - f32::from(negative);
        let intent = CameraIntent {
            pan: [
                axis(self.pan_west, self.pan_east),
                axis(self.pan_south, self.pan_north),
            ],
            // Screen Y grows downward while the world's does not, so a downward drag pans south.
            drag: [
                -self.drag[0] * drag_pixels_to_units,
                self.drag[1] * drag_pixels_to_units,
            ],
            zoom: self.zoom,
            rotate: axis(self.rotate_left, self.rotate_right),
            reset: self.reset,
            reset_rotation: self.reset_rotation,
        }
        .normalized();

        // Per-frame accumulators are consumed; held keys are not.
        self.drag = [0.0, 0.0];
        self.zoom = 0.0;
        self.reset = false;
        self.reset_rotation = false;
        intent
    }
}

#[cfg(test)]
mod tests {
    // Exact comparisons are against values the mapping produces structurally: zeros, unit axes, and
    // the pixel scale the caller supplies.
    #![allow(clippy::float_cmp)]

    use super::{Action, InputState, TerrainGround, choose_format};
    use cic_assets::Terrain;
    use cic_camera::GroundHeight;

    #[test]
    fn held_keys_pan_every_frame_while_scroll_is_consumed_once() {
        // The distinction the whole type exists for: a held key repeats, a discrete event does not.
        let mut input = InputState::default();
        input.set_action(Action::PanNorth, true);
        input.add_scroll(3.0);

        let first = input.take_intent(1.0);
        assert_eq!(first.pan, [0.0, 1.0]);
        assert_eq!(first.zoom, 3.0);

        let second = input.take_intent(1.0);
        assert_eq!(second.pan, [0.0, 1.0], "a held key still pans");
        assert_eq!(second.zoom, 0.0, "a scroll notch must not repeat");
    }

    #[test]
    fn opposing_keys_cancel_and_diagonals_do_not_outrun_axes() {
        let mut input = InputState::default();
        input.set_action(Action::PanNorth, true);
        input.set_action(Action::PanSouth, true);
        assert_eq!(input.take_intent(1.0).pan, [0.0, 0.0]);

        input.set_action(Action::PanSouth, false);
        input.set_action(Action::PanEast, true);
        let diagonal = input.take_intent(1.0).pan;
        let length = (diagonal[0] * diagonal[0] + diagonal[1] * diagonal[1]).sqrt();
        assert!(
            (length - 1.0).abs() < 1.0e-5,
            "diagonal pan should be unit length, was {length}"
        );
    }

    #[test]
    fn pointer_motion_only_counts_while_dragging() {
        let mut input = InputState::default();
        input.add_pointer_motion(10.0, 10.0);
        assert_eq!(input.take_intent(1.0).drag, [0.0, 0.0], "not dragging yet");

        input.set_dragging(true);
        input.add_pointer_motion(10.0, 4.0);
        let intent = input.take_intent(0.5);
        // Screen Y grows downward, so a downward drag pans south: X negates, Y does not.
        assert!(
            intent.drag[0] < 0.0 && intent.drag[1] > 0.0,
            "{:?}",
            intent.drag
        );
    }

    #[test]
    fn releasing_a_drag_discards_motion_not_yet_taken() {
        // Otherwise letting go leaves one last frame of pan queued behind the release.
        let mut input = InputState::default();
        input.set_dragging(true);
        input.add_pointer_motion(40.0, 0.0);
        input.set_dragging(false);
        assert_eq!(input.take_intent(1.0).drag, [0.0, 0.0]);
        assert!(!input.is_dragging());
    }

    #[test]
    fn reset_latches_once_per_press() {
        let mut input = InputState::default();
        input.set_action(Action::Reset, true);
        assert!(input.take_intent(1.0).reset);
        // Still held, but the reset already happened.
        assert!(
            !input.take_intent(1.0).reset,
            "a held key must not reset repeatedly"
        );
    }

    #[test]
    fn rotation_maps_to_opposing_signs() {
        let mut input = InputState::default();
        input.set_action(Action::RotateLeft, true);
        assert_eq!(input.take_intent(1.0).rotate, -1.0);
        input.set_action(Action::RotateLeft, false);
        input.set_action(Action::RotateRight, true);
        assert_eq!(input.take_intent(1.0).rotate, 1.0);
    }

    #[test]
    fn an_srgb_format_is_preferred() {
        // Presenting linear colour to a non-sRGB surface looks markedly darker than the capture of
        // the same frame, which is easy to mistake for a lighting bug.
        let chosen = choose_format(&[
            wgpu::TextureFormat::Bgra8Unorm,
            wgpu::TextureFormat::Bgra8UnormSrgb,
        ]);
        assert_eq!(chosen, Some(wgpu::TextureFormat::Bgra8UnormSrgb));

        // With nothing sRGB on offer, the first is better than refusing to draw.
        assert_eq!(
            choose_format(&[wgpu::TextureFormat::Rgba8Unorm]),
            Some(wgpu::TextureFormat::Rgba8Unorm)
        );
        assert_eq!(choose_format(&[]), None);
    }

    #[test]
    fn terrain_ground_reports_elevation_and_absence() {
        let terrain = Terrain::new(3, 3, 10.0, 0.5, vec![100; 9], Vec::new()).expect("valid");
        let ground = TerrainGround(&terrain);
        assert_eq!(ground.height_at(10.0, 10.0), Some(50.0));
        // Off the map is absent, not zero -- the camera holds its height rather than diving.
        assert_eq!(ground.height_at(-5.0, 0.0), None);
        assert_eq!(ground.height_at(1_000.0, 0.0), None);
    }
}
