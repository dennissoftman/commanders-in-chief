// Commanders in Chief
// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only
//
// Provenance: project design. The original's shell runs inside its own `GameEngine` frame loop and
// changes display mode through `Display.h`'s `setDisplayMode`, which this project neither has nor
// reproduces. What is preserved is the consequence rather than the mechanism: a mode change that the
// player cannot confirm must leave the previous mode in force, which is why the window and surface
// are reconfigured only through `cic_ui::UiDisplayTransaction` and never directly.

use std::error::Error;
use std::sync::Arc;
use std::time::Instant;

use cic_ui::{UiDisplaySelection, UiKey, UiMouseButton, UiPoint, UiViewport, UiWindowMode};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, OwnedDisplayHandle};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Fullscreen, Window, WindowId};

use crate::create_ui_pipeline;
use crate::ui::{StagedUiFrame, UiTexturePage};
use crate::ui_text::UiFontSet;

/// One input the shell viewer forwards, in this project's own vocabulary rather than `winit`'s.
///
/// The host is `cic-tools`, which already speaks `cic-ui`'s input types; translating here keeps the
/// shell free of any dependency on a windowing library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiViewEvent {
    /// The pointer moved to a position in physical pixels.
    PointerMoved(UiPoint),
    /// A mouse button went down at the last known pointer position.
    PointerPressed(UiMouseButton),
    /// A mouse button came up.
    PointerReleased(UiMouseButton),
    /// A key with structural meaning was pressed.
    Key(UiKey),
    /// Text was typed.
    Text(String),
    /// The surface was resized to a new physical size.
    Resized(UiViewport),
}

/// What the host asks the viewer to do after a frame.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum UiViewRequest {
    /// Nothing; keep presenting.
    #[default]
    Continue,
    /// Reconfigure the window and surface for this selection, then report the result.
    ///
    /// The viewer performs the change and hands the outcome back through
    /// [`UiViewContext::display_applied`] on the next update. It never decides whether the change
    /// is kept: that is the transaction's decision, and the host owns the transaction.
    ApplyDisplay(Box<UiDisplaySelection>),
    /// Close the window.
    Exit,
}

/// What the viewer tells the host at the start of one frame.
pub struct UiViewContext<'a> {
    /// Every input since the previous frame, in arrival order.
    pub events: &'a [UiViewEvent],
    /// The surface's current size in physical pixels.
    pub viewport: UiViewport,
    /// Milliseconds since the viewer started. This is the host's clock for the display timeout, and
    /// the only place a real clock enters the display path.
    pub elapsed_ms: u64,
    /// How many whole transition frames have elapsed since the previous update, at the original's
    /// thirty per second.
    pub transition_frames: u32,
    /// The result of a previously requested display change, once the platform has answered.
    ///
    /// `Ok` means the window and surface now present that selection; `Err` carries what the platform
    /// reported, which the host feeds to the transaction's failure path.
    pub display_applied: Option<Result<UiDisplaySelection, String>>,
}

/// What a host must provide to be driven by the viewer.
pub trait UiViewHost {
    /// Produces one frame from the current state, after applying the context's events.
    ///
    /// # Errors
    ///
    /// Any error ends the session and is returned from [`run_ui_view`].
    fn update(
        &mut self,
        context: &UiViewContext<'_>,
    ) -> Result<(StagedUiFrame, UiViewRequest), Box<dyn Error>>;

    /// Returns the texture pages the staged frames index into.
    fn pages(&self) -> &[UiTexturePage];

    /// Returns a counter the host increments whenever [`UiViewHost::pages`] changes.
    ///
    /// Pages are re-uploaded only when this moves, so a session that binds a new layout's images
    /// pays for it once rather than every frame.
    fn pages_revision(&self) -> u64;

    /// Returns the window title.
    fn title(&self) -> String;

    /// Hands the host the platform's monitors, once, before the first frame.
    ///
    /// A process may create only one `winit` event loop, and listing monitors needs one, so a host
    /// that wants the catalog cannot go and get it itself — the viewer already owns the only loop
    /// there will be. This is that catalog, enumerated from it.
    ///
    /// # Errors
    ///
    /// Any error ends the session and is returned from [`run_ui_view`].
    fn monitors_enumerated(
        &mut self,
        catalog: cic_ui::UiDisplayCatalog,
    ) -> Result<(), Box<dyn Error>> {
        let _ = catalog;
        Ok(())
    }
}

/// What a finished viewer session did, for the caller's report.
///
/// The surface format is here because it is the one thing about the window that a caller cannot
/// choose and that changes what the player sees: an sRGB target would encode the already-encoded UI
/// a second time, so a run that could not get a linear one is a run whose colours are wrong, and
/// that has to be visible rather than inferred from a screenshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiViewSummary {
    /// The surface format the session presented through.
    pub surface_format: String,
    /// Whether that format applies the sRGB transfer function in hardware.
    pub surface_is_srgb: bool,
    /// How many frames were presented.
    pub frames: u64,
    /// How many pointer moves, presses, releases, keys, and text inputs the window delivered.
    ///
    /// An interactive session that does nothing is otherwise indistinguishable from one whose input
    /// never arrived, and those have very different causes.
    pub input_counts: [u64; 5],
}

/// Runs the retained shell in a window until the host asks to exit or the window closes.
///
/// # Errors
///
/// Returns an error when the event loop, window, adapter, device, or surface cannot be created, or
/// when the host fails.
pub fn run_ui_view(
    host: &mut dyn UiViewHost,
    initial: UiViewport,
    fonts: Vec<Vec<u8>>,
) -> Result<UiViewSummary, Box<dyn Error>> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let display = event_loop.owned_display_handle();
    let mut application = UiViewApplication {
        host,
        display,
        fonts,
        initial,
        window: None,
        gpu: None,
        events: Vec::new(),
        pointer: UiPoint::new(0, 0),
        started: Instant::now(),
        last_frame: Instant::now(),
        transition_remainder: 0.0,
        uploaded_revision: None,
        display_applied: None,
        frames: 0,
        input_counts: [0; 5],
        error: None,
    };
    event_loop.run_app(&mut application)?;
    if let Some(error) = application.error {
        return Err(error);
    }
    let format = application
        .gpu
        .as_ref()
        .map_or(wgpu::TextureFormat::Rgba8Unorm, |gpu| gpu.config.format);
    Ok(UiViewSummary {
        surface_format: format!("{format:?}"),
        surface_is_srgb: format.is_srgb(),
        frames: application.frames,
        input_counts: application.input_counts,
    })
}

/// GPU state bound to one surface.
struct UiViewGpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    viewport_layout: wgpu::BindGroupLayout,
    page_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    bind_groups: Vec<wgpu::BindGroup>,
    fonts: Option<UiFontSet>,
}

struct UiViewApplication<'host> {
    host: &'host mut dyn UiViewHost,
    display: OwnedDisplayHandle,
    fonts: Vec<Vec<u8>>,
    initial: UiViewport,
    window: Option<Arc<Window>>,
    gpu: Option<UiViewGpu>,
    events: Vec<UiViewEvent>,
    pointer: UiPoint,
    started: Instant,
    last_frame: Instant,
    transition_remainder: f32,
    uploaded_revision: Option<u64>,
    display_applied: Option<Result<UiDisplaySelection, String>>,
    frames: u64,
    input_counts: [u64; 5],
    error: Option<Box<dyn Error>>,
}

impl UiViewApplication<'_> {
    fn initialize(&mut self, event_loop: &ActiveEventLoop) -> Result<(), Box<dyn Error>> {
        let attributes = Window::default_attributes()
            .with_title(self.host.title())
            .with_inner_size(PhysicalSize::new(
                u32::try_from(self.initial.width())?,
                u32::try_from(self.initial.height())?,
            ));
        let window = Arc::new(event_loop.create_window(attributes)?);
        // The host is told what displays exist before it is asked for a frame, so the first frame it
        // stages can already show the truth about the window it is being drawn into.
        self.host
            .monitors_enumerated(crate::display::display_catalog_from_monitors(
                event_loop.available_monitors(),
            )?)?;
        let gpu = pollster::block_on(UiViewGpu::new(
            window.clone(),
            self.display.clone(),
            &self.fonts,
        ))?;
        self.window = Some(window);
        self.gpu = Some(gpu);
        Ok(())
    }

    /// Applies a selection to the real window and surface.
    ///
    /// Each mode is a genuinely different request. Windowed asks for a client size and clears any
    /// fullscreen. Borderless takes the monitor whole without naming a mode. Exclusive has to find
    /// the `winit` video mode matching the selection's advertised resolution and refresh, and fails
    /// rather than substituting a near miss — substituting is exactly how a player ends up looking at
    /// a mode they did not choose and cannot confirm.
    fn apply_display(&mut self, selection: &UiDisplaySelection) -> Result<(), String> {
        let Some(window) = self.window.clone() else {
            return Err("the window is not open".to_owned());
        };
        let monitor = crate::display::find_monitor(window.available_monitors(), &selection.monitor)
            .or_else(|| window.primary_monitor());
        match selection.window_mode {
            UiWindowMode::Windowed => {
                window.set_fullscreen(None);
                window.set_decorations(true);
                let requested = PhysicalSize::new(selection.resolution.0, selection.resolution.1);
                // `request_inner_size` returns the new size when the platform resizes immediately
                // and `None` when the change arrives as an event instead; both are success.
                let _ = window.request_inner_size(requested);
            }
            UiWindowMode::BorderlessDesktop => {
                let monitor = monitor.ok_or("no monitor to go borderless on")?;
                window.set_fullscreen(Some(Fullscreen::Borderless(Some(monitor))));
            }
            UiWindowMode::ExclusiveFullscreen => {
                let monitor = monitor.ok_or("no monitor for an exclusive mode")?;
                let mode = monitor
                    .video_modes()
                    .find(|mode| {
                        let size = mode.size();
                        size.width == selection.resolution.0
                            && size.height == selection.resolution.1
                            && mode.refresh_rate_millihertz() == selection.refresh_millihertz
                    })
                    .ok_or_else(|| {
                        format!(
                            "the monitor no longer advertises {}x{} at {} mHz",
                            selection.resolution.0,
                            selection.resolution.1,
                            selection.refresh_millihertz
                        )
                    })?;
                window.set_fullscreen(Some(Fullscreen::Exclusive(mode)));
            }
        }
        Ok(())
    }

    /// Runs one frame: hand the host its events and the elapsed time, then present what it staged.
    fn draw(&mut self) -> Result<bool, Box<dyn Error>> {
        let Some(window) = self.window.clone() else {
            return Ok(false);
        };
        let viewport = viewport_of(window.inner_size())?;

        // Whole transition frames only. The handler's state machine is discrete, so handing it a
        // fraction would let a state be skipped at a high refresh rate and repeated at a low one;
        // the remainder is carried instead, exactly as the accumulator in the source does.
        let delta = self.last_frame.elapsed().as_secs_f32();
        self.last_frame = Instant::now();
        let advanced = self.transition_remainder + delta * TRANSITION_FRAMES_PER_SECOND;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let transition_frames = advanced.trunc() as u32;
        self.transition_remainder = advanced.fract();

        let events = std::mem::take(&mut self.events);
        let context = UiViewContext {
            events: &events,
            viewport,
            elapsed_ms: u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX),
            transition_frames,
            display_applied: self.display_applied.take(),
        };
        let (staged, request) = self.host.update(&context)?;

        let title = self.host.title();
        window.set_title(&title);

        let revision = self.host.pages_revision();
        if self.uploaded_revision != Some(revision) {
            let pages = self.host.pages().to_vec();
            if let Some(gpu) = self.gpu.as_mut() {
                gpu.upload_pages(&pages)?;
            }
            self.uploaded_revision = Some(revision);
        }
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.present(&staged)?;
        }
        self.frames += 1;

        match request {
            UiViewRequest::Continue => Ok(false),
            UiViewRequest::Exit => Ok(true),
            UiViewRequest::ApplyDisplay(selection) => {
                // The result goes back on the next frame rather than now, because the platform
                // answers a fullscreen or resize request through an event, not a return value.
                self.display_applied = Some(match self.apply_display(&selection) {
                    Ok(()) => Ok(*selection),
                    Err(reason) => Err(reason),
                });
                Ok(false)
            }
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: Box<dyn Error>) {
        self.error = Some(error);
        event_loop.exit();
    }
}

/// The original's transition rate, which is what a whole frame means here.
const TRANSITION_FRAMES_PER_SECOND: f32 = 30.0;

impl ApplicationHandler for UiViewApplication<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none()
            && let Err(error) = self.initialize(event_loop)
        {
            self.fail(event_loop, error);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.resize(size);
                }
                if let Ok(viewport) = viewport_of(size) {
                    self.events.push(UiViewEvent::Resized(viewport));
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.pointer = pointer_of(position);
                self.input_counts[0] += 1;
                self.events.push(UiViewEvent::PointerMoved(self.pointer));
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let Some(button) = mouse_button_of(button) else {
                    return;
                };
                self.events.push(match state {
                    ElementState::Pressed => {
                        self.input_counts[1] += 1;
                        UiViewEvent::PointerPressed(button)
                    }
                    ElementState::Released => {
                        self.input_counts[2] += 1;
                        UiViewEvent::PointerReleased(button)
                    }
                });
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    return;
                }
                match &event.logical_key {
                    Key::Named(named) => {
                        if let Some(key) = named_key_of(*named) {
                            self.input_counts[3] += 1;
                            self.events.push(UiViewEvent::Key(key));
                        }
                    }
                    Key::Character(text) => {
                        self.input_counts[4] += 1;
                        self.events.push(UiViewEvent::Text(text.to_string()));
                    }
                    Key::Dead(_) | Key::Unidentified(_) => {}
                }
            }
            WindowEvent::RedrawRequested => match self.draw() {
                Ok(true) => event_loop.exit(),
                Ok(false) => {}
                Err(error) => self.fail(event_loop, error),
            },
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

/// Converts a physical size to a viewport, refusing a zero dimension a minimised window reports.
fn viewport_of(size: PhysicalSize<u32>) -> Result<UiViewport, Box<dyn Error>> {
    Ok(UiViewport::new(
        i32::try_from(size.width.max(1))?,
        i32::try_from(size.height.max(1))?,
    )?)
}

#[allow(clippy::cast_possible_truncation)]
fn pointer_of(position: PhysicalPosition<f64>) -> UiPoint {
    UiPoint::new(position.x as i32, position.y as i32)
}

const fn mouse_button_of(button: MouseButton) -> Option<UiMouseButton> {
    match button {
        MouseButton::Left => Some(UiMouseButton::Left),
        MouseButton::Right => Some(UiMouseButton::Right),
        _ => None,
    }
}

const fn named_key_of(key: NamedKey) -> Option<UiKey> {
    match key {
        NamedKey::Tab => Some(UiKey::Tab),
        NamedKey::Backspace => Some(UiKey::Backspace),
        NamedKey::Delete => Some(UiKey::Delete),
        NamedKey::ArrowLeft => Some(UiKey::Left),
        NamedKey::ArrowRight => Some(UiKey::Right),
        NamedKey::ArrowUp => Some(UiKey::Up),
        NamedKey::ArrowDown => Some(UiKey::Down),
        NamedKey::Home => Some(UiKey::Home),
        NamedKey::End => Some(UiKey::End),
        NamedKey::Enter => Some(UiKey::Enter),
        NamedKey::Escape => Some(UiKey::Escape),
        _ => None,
    }
}

impl UiViewGpu {
    async fn new(
        window: Arc<Window>,
        display: OwnedDisplayHandle,
        fonts: &[Vec<u8>],
    ) -> Result<Self, Box<dyn Error>> {
        let descriptor = wgpu::InstanceDescriptor::new_with_display_handle(Box::new(display));
        let instance = wgpu::Instance::new(descriptor);
        let surface = instance.create_surface(window.clone())?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("cic-render ui view device"),
                ..Default::default()
            })
            .await?;

        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or("the surface supports no usable configuration")?;
        config.present_mode = wgpu::PresentMode::Fifo;
        let capabilities = surface.get_capabilities(&adapter);
        // The UI path is the mirror image of the terrain viewer's. Its composite returns unencoded
        // values and relies on an sRGB target to encode them in hardware; the UI pipeline writes
        // bytes that are *already* display-encoded — pages upload in the target's own space and
        // reach the attachment unchanged, which is what makes the headless capture correct — so an
        // sRGB target here would apply the transfer function a second time and wash the menu out.
        // Prefer the linear pair of whatever the backend chose, so the window matches the PNG.
        if config.format.is_srgb() {
            let linear = config.format.remove_srgb_suffix();
            if capabilities.formats.contains(&linear) {
                config.format = linear;
            } else if let Some(other) = capabilities
                .formats
                .iter()
                .copied()
                .find(|candidate| !candidate.is_srgb())
            {
                config.format = other;
            }
        }
        let format = config.format;
        surface.configure(&device, &config);

        let (pipeline, viewport_layout, page_layout) = create_ui_pipeline(&device, format);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("cic-render ui view sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let fonts = if fonts.is_empty() {
            None
        } else {
            Some(UiFontSet::new(&device, &queue, format, fonts)?)
        };
        Ok(Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            viewport_layout,
            page_layout,
            sampler,
            bind_groups: Vec::new(),
            fonts,
        })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        self.config.width = size.width.max(1);
        self.config.height = size.height.max(1);
        self.surface.configure(&self.device, &self.config);
    }

    /// Uploads every page plus the one white pixel a colour-only batch binds.
    fn upload_pages(&mut self, pages: &[UiTexturePage]) -> Result<(), Box<dyn Error>> {
        let white = UiTexturePage::new(1, 1, vec![255, 255, 255, 255])?;
        let mut bind_groups = Vec::with_capacity(pages.len() + 1);
        for page in std::iter::once(&white).chain(pages) {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("cic-render ui view page"),
                size: wgpu::Extent3d {
                    width: page.width(),
                    height: page.height(),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.config.format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &reorder_for(self.config.format, page),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(page.width() * 4),
                    rows_per_image: Some(page.height()),
                },
                texture.size(),
            );
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            bind_groups.push(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("cic-render ui view page bind group"),
                layout: &self.page_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            }));
        }
        self.bind_groups = bind_groups;
        Ok(())
    }
}

impl UiViewGpu {
    /// Draws one staged frame to the surface, in the batch order `cic-ui` produced.
    ///
    /// This is the same submission order as the headless capture — batches with their scissor and
    /// page, then shaped text over the top — so what is on screen and what `ui-render` writes to a
    /// PNG differ only in where they land.
    #[expect(
        clippy::too_many_lines,
        reason = "one straight-line pass: acquire, upload geometry, submit batches, present"
    )]
    fn present(&mut self, staged: &StagedUiFrame) -> Result<(), Box<dyn Error>> {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
            // An outdated or lost surface is ordinary during a mode change, which is the whole point
            // of this viewer: reconfigure and let the next frame draw rather than failing.
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err("the surface reported a validation failure".into());
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let canvas = [self.config.width, self.config.height];

        #[allow(clippy::cast_precision_loss)]
        let uniform_values = [canvas[0] as f32, canvas[1] as f32, 0.0, 0.0];
        let mut uniform_bytes = Vec::with_capacity(16);
        for value in uniform_values {
            uniform_bytes.extend_from_slice(&value.to_le_bytes());
        }
        let uniform = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cic-render ui view viewport uniform"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&uniform, 0, &uniform_bytes);
        let viewport_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cic-render ui view viewport bind group"),
            layout: &self.viewport_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });

        let vertex_bytes = staged.vertex_bytes();
        let index_bytes = staged.index_bytes();
        let geometry = if vertex_bytes.is_empty() {
            None
        } else {
            let vertices = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("cic-render ui view vertices"),
                size: vertex_bytes.len() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let indices = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("cic-render ui view indices"),
                size: index_bytes.len() as u64,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.queue.write_buffer(&vertices, 0, &vertex_bytes);
            self.queue.write_buffer(&indices, 0, &index_bytes);
            Some((vertices, indices))
        };

        let mut fonts = match self.fonts.as_mut() {
            Some(fonts) if !staged.text().is_empty() => {
                fonts.prepare(&self.device, &self.queue, canvas, staged.text())?;
                Some(fonts)
            }
            _ => None,
        };

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("cic-render ui view encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cic-render ui view pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let Some((vertices, indices)) = geometry.as_ref()
                && !self.bind_groups.is_empty()
            {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &viewport_bind_group, &[]);
                pass.set_vertex_buffer(0, vertices.slice(..));
                pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
                for batch in staged.batches() {
                    let bound = match batch.page() {
                        Some(page) => page + 1,
                        None => 0,
                    };
                    let Some(group) = self.bind_groups.get(bound) else {
                        continue;
                    };
                    pass.set_bind_group(1, group, &[]);
                    match batch.scissor() {
                        Some(rect) => {
                            let Some(scissor) = crate::clamp_scissor(rect, canvas[0], canvas[1])
                            else {
                                continue;
                            };
                            pass.set_scissor_rect(scissor.0, scissor.1, scissor.2, scissor.3);
                        }
                        None => pass.set_scissor_rect(0, 0, canvas[0], canvas[1]),
                    }
                    let first = batch.first_index();
                    pass.draw_indexed(first..first + batch.index_count(), 0, 0..1);
                }
                pass.set_scissor_rect(0, 0, canvas[0], canvas[1]);
            }
            if let Some(fonts) = fonts.as_ref() {
                fonts.draw(&mut pass)?;
            }
        }
        if let Some(fonts) = fonts.as_mut() {
            fonts.trim();
        }
        self.queue.submit(std::iter::once(encoder.finish()));
        self.queue.present(frame);
        Ok(())
    }
}

/// Returns page bytes in the surface format's channel order.
///
/// Pages are authored RGBA. A `Bgra8Unorm` surface — which is what Windows and macOS usually offer
/// first — reads the same four bytes as BGRA, so the red and blue channels swap unless they are
/// swapped here. Nothing else about the bytes changes.
fn reorder_for(format: wgpu::TextureFormat, page: &UiTexturePage) -> Vec<u8> {
    let rgba = page.rgba();
    if !matches!(
        format,
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
    ) {
        return rgba.to_vec();
    }
    let mut swapped = rgba.to_vec();
    for pixel in swapped.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    swapped
}
