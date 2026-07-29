//! The shell in a window: navigate it, type into it, and watch a settings change revert itself.
//!
//! ```bash
//! cargo run -p cic-render --example shell --release
//! ```
//!
//! # Why this exists next to a green capture suite
//!
//! Because this project has a standing rule that presentation needs running rather than only testing,
//! and it earned that rule: the one bug the headless suite structurally could not catch appeared the first
//! time a window opened. The interface has more of that class than the scene did, and none of it is
//! reachable from a capture — hover following a pointer, focus moving under Tab, a caret advancing as
//! somebody types, an input method placing its candidate window, a revert countdown actually counting.
//!
//! # What it demonstrates that a test cannot
//!
//! **The revert window against a real clock.** Change the resolution scale, press Apply, and do nothing.
//! Fifteen seconds later the setting comes back on its own. That is the whole reason the transaction
//! exists, and a test that passes it fabricated numbers proves the arithmetic rather than the behaviour.
//!
//! # Controls
//!
//! Pointer to hover and click. `Tab` and `Shift+Tab` move focus, `Enter` or `Space` activates, the arrow
//! keys adjust a slider or a list and move the caret in a text field, and `Escape` goes back — which at
//! the main menu asks whether to leave.

use std::error::Error;
use std::sync::Arc;
use std::time::Instant;

use cic_render::gpu::GpuContext;
use cic_render::text::{Font, GlyphAtlas};
use cic_render::ui::{DrawList, UiMetrics, UiRenderer, atlas_sizes};
use cic_render::{DisplaySettings, RenderError};
use cic_ui::paint::{Painter, Theme};
use cic_ui::{
    Adjust, Edit, FocusMove, Interface, Layout, Motion, Probation, Screen, Screens, Shell,
    StringTable, UiEvent, Viewport,
};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

fn main() -> Result<(), Box<dyn Error>> {
    let event_loop = EventLoop::new()?;
    // Poll rather than Wait: the revert countdown has to advance whether or not anybody is touching the
    // keyboard, which is the entire point of it.
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = ShellApp::default();
    event_loop.run_app(&mut app)?;
    Ok(())
}

/// The shell's authored screens, compiled in so the example runs from any directory.
fn screens() -> Result<Screens, Box<dyn Error>> {
    let mut screens = Screens::new();
    for (screen, bytes) in [
        (
            Screen::MainMenu,
            include_bytes!("../../../content/ui/main_menu.ciclayout.json").as_slice(),
        ),
        (
            Screen::Settings,
            include_bytes!("../../../content/ui/settings.ciclayout.json").as_slice(),
        ),
        (
            Screen::SkirmishSetup,
            include_bytes!("../../../content/ui/skirmish_setup.ciclayout.json").as_slice(),
        ),
        (
            Screen::QuitConfirm,
            include_bytes!("../../../content/ui/quit_confirm.ciclayout.json").as_slice(),
        ),
    ] {
        screens.insert(screen, Layout::from_json(bytes)?);
    }
    Ok(screens)
}

/// Everything that exists only once a window does.
struct Active {
    window: Arc<Window>,
    context: GpuContext,
    surface: wgpu::Surface<'static>,
    format: wgpu::TextureFormat,
    size: [u32; 2],
    scale: f32,
    renderer: UiRenderer,
    atlas: GlyphAtlas,
    shell: Shell<DisplaySettings>,
    strings: StringTable,
    list: DrawList,
    pointer: [f32; 2],
    last: Instant,
    ime: bool,
    shift: bool,
}

#[derive(Default)]
struct ShellApp {
    active: Option<Active>,
    theme: Theme,
}

impl ShellApp {
    /// Rebuilds the atlas and re-solves for a new surface size or display scale.
    fn resize(&mut self, width: u32, height: u32, scale: f32) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        let (width, height) = (width.max(1), height.max(1));
        active.size = [width, height];
        active.surface.configure(
            active.context.device(),
            &configuration(active.format, width, height),
        );
        if (active.scale - scale).abs() > f32::EPSILON {
            active.scale = scale;
            // Explicit, because an atlas is built for declared sizes and rebuilding it inside a draw
            // would put a texture allocation in the middle of a frame.
            active.atlas = GlyphAtlas::new(&Font::new(), &atlas_sizes(&self.theme, scale));
            active.renderer.set_atlas(
                active.context.device(),
                active.context.queue(),
                &active.atlas,
            );
        }
        match Viewport::new(width, height, scale) {
            Ok(viewport) => {
                let metrics = UiMetrics::new(&self.theme, &active.strings, scale);
                active.shell.resize(viewport, &metrics);
            }
            Err(error) => eprintln!("ignoring a surface this layout cannot be solved for: {error}"),
        }
    }

    /// Mirrors the settings transaction into the labels that show it.
    ///
    /// The channel a stored value opens: a countdown and a slider's readout are per-frame text, and a
    /// string table is not where a per-frame value belongs.
    fn refresh_readouts(&mut self) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        if active.shell.top() != Screen::Settings {
            return;
        }
        let staged = *active.shell.settings().staged();
        let remaining = active.shell.settings().remaining();
        let dirty = active.shell.settings().is_dirty();
        let interface = active.shell.interface_mut();
        // The slider is the source of truth while it is being dragged, so the staged value follows it
        // rather than the other way round.
        if let Some(value) = interface.slide("settings_scale") {
            let mut next = staged;
            next.resolution_scale = value;
            if next != staged {
                active.shell.settings_mut().stage(next);
            }
        } else {
            interface.set_slide("settings_scale", staged.resolution_scale);
        }
        let interface = active.shell.interface_mut();
        interface.set_text(
            "settings_scale_value",
            format!("{:.2}x", staged.resolution_scale),
        );
        let prompt = match remaining {
            Some(left) => format!("Reverting in {:.0} s unless you keep it", left.ceil()),
            None if dirty => "Apply puts a change in force for 15 seconds.".to_owned(),
            None => "Nothing staged.".to_owned(),
        };
        active
            .shell
            .interface_mut()
            .set_text("settings_countdown", prompt);
    }

    /// Reads the checkbox back into the staged settings.
    fn stage_antialiasing(&mut self) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        let Some(on) = active.shell.interface().toggle("settings_antialias") else {
            return;
        };
        let wanted = if on {
            cic_render::Antialiasing::Fxaa
        } else {
            cic_render::Antialiasing::None
        };
        let staged = *active.shell.settings().staged();
        if staged.antialiasing != wanted {
            active
                .shell
                .settings_mut()
                .stage(staged.with_antialiasing(wanted));
        }
    }

    /// Draws one frame.
    fn draw(&mut self) -> Result<(), RenderError> {
        let Some(active) = self.active.as_mut() else {
            return Ok(());
        };
        let metrics = UiMetrics::new(&self.theme, &active.strings, active.scale);
        let painter = Painter::new(&self.theme, &metrics, active.shell.viewport());

        active.list.clear();
        let mut primitives = Vec::new();
        let blank = Interface::new();
        for (screen, reveal, solved) in active.shell.frames() {
            let interface = active.shell.stack().interface_for(screen).unwrap_or(&blank);
            primitives.clear();
            painter.paint_revealed(
                &mut primitives,
                solved,
                interface,
                active.shell.strings(),
                reveal,
            );
            active.list.extend(&primitives, &active.atlas);
        }

        let configuration = configuration(active.format, active.size[0], active.size[1]);
        let frame = match active.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            // Suboptimal still draws: the frame is usable and reconfiguring takes effect next time.
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                active
                    .surface
                    .configure(active.context.device(), &configuration);
                frame
            }
            // Outdated and Lost need the surface reconfigured before the next attempt; Timeout and
            // Occluded are resolved by not drawing this frame at all.
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                active
                    .surface
                    .configure(active.context.device(), &configuration);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(RenderError::SurfaceLost(
                    "the surface rejected the request".to_owned(),
                ));
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder =
            active
                .context
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("shell frame"),
                });
        clear_backdrop(&mut encoder, &view, self.theme.backdrop.to_linear());
        active.renderer.draw(
            active.context.device(),
            active.context.queue(),
            &mut encoder,
            &view,
            active.size,
            &active.list,
        );
        active.context.queue().submit([encoder.finish()]);
        active.context.queue().present(frame);

        // An input method cannot place its candidate window without being told where the text is, and the
        // painter's answer is the caret rather than the whole field.
        let wanted = active.shell.ime_wanted();
        if wanted != active.ime {
            active.ime = wanted;
            active.window.set_ime_allowed(wanted);
        }
        if wanted
            && let Some(solved) = active.shell.solved()
            && let Some(caret) = painter.ime_cursor_area(solved, active.shell.interface())
        {
            active.window.set_ime_cursor_area(
                winit::dpi::PhysicalPosition::new(caret.x, caret.y),
                winit::dpi::PhysicalSize::new(caret.width.max(1.0), caret.height),
            );
        }
        Ok(())
    }
}

impl ShellApp {
    /// Opens a window and everything that depends on one, reporting why if it cannot.
    fn start(&mut self, event_loop: &ActiveEventLoop) -> Result<(), Box<dyn Error>> {
        let attributes = Window::default_attributes()
            .with_title("Commanders in Chief -- shell")
            .with_inner_size(winit::dpi::LogicalSize::new(960.0, 600.0));
        let window = Arc::new(event_loop.create_window(attributes)?);
        let (context, surface) = pollster::block_on(GpuContext::for_window(window.clone()))?;

        let capabilities = surface.get_capabilities(context.adapter());
        // An sRGB format on purpose: the paint layer hands over *linear* colours because the hardware is
        // expected to apply the encoding on write. Presented through a non-sRGB format the whole
        // interface would come out visibly dark, and nothing would report it.
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .or_else(|| capabilities.formats.first().copied())
            .ok_or(RenderError::NoSurfaceFormat)?;
        if !format.is_srgb() {
            eprintln!("warning: {format:?} is not sRGB-encoded, so colours will read dark");
        }

        let size = window.inner_size();
        let (width, height) = (size.width.max(1), size.height.max(1));
        #[allow(clippy::cast_possible_truncation)]
        let scale = window.scale_factor() as f32;
        surface.configure(context.device(), &configuration(format, width, height));

        let strings =
            StringTable::from_json(include_bytes!("../../../content/ui/strings.en.json"))?;
        let viewport = Viewport::new(width, height, scale)?;
        let metrics = UiMetrics::new(&self.theme, &strings, scale);
        // Animated, because the whole reason this example exists is that motion cannot be judged from a
        // still image.
        let shell = Shell::with_motion(
            screens()?,
            strings.clone(),
            DisplaySettings::NATIVE,
            viewport,
            Motion::DEFAULT,
            &metrics,
        )?;
        for absent in shell.missing_strings() {
            eprintln!("warning: no string for {absent}, which will draw as its own key");
        }

        let atlas = GlyphAtlas::new(&Font::new(), &atlas_sizes(&self.theme, scale));
        let renderer = UiRenderer::new(context.device(), context.queue(), format, &atlas);
        eprintln!(
            "surface: {format:?} at {width}x{height}, scale {scale}, atlas {}x{} holding {} glyphs",
            atlas.width(),
            atlas.height(),
            atlas.len()
        );
        eprintln!("change the resolution scale, press Apply, and wait: it reverts itself.");

        self.active = Some(Active {
            window,
            context,
            surface,
            format,
            size: [width, height],
            scale,
            renderer,
            atlas,
            shell,
            strings,
            list: DrawList::new(),
            pointer: [0.0, 0.0],
            last: Instant::now(),
            ime: false,
            shift: false,
        });
        self.refresh_readouts();
        Ok(())
    }
}

impl ApplicationHandler for ShellApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.active.is_some() {
            return;
        }
        if let Err(error) = self.start(event_loop) {
            eprintln!("the shell could not start: {error}");
            event_loop.exit();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                let scale = self.active.as_ref().map_or(1.0, |active| active.scale);
                self.resize(size.width, size.height, scale);
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                let size = self.active.as_ref().map_or([1, 1], |active| active.size);
                #[allow(clippy::cast_possible_truncation)]
                self.resize(size[0], size[1], scale_factor as f32);
            }
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.draw() {
                    eprintln!("frame dropped: {error}");
                }
            }
            other => self.route(event_loop, other),
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        // A real clock, and deliberately not scene time: a display mode that produces no frames advances
        // no frame counter, and a revert that depends on rendering succeeding cannot fire in the case it
        // exists for.
        let now = Instant::now();
        let elapsed = now.duration_since(active.last).as_secs_f32();
        active.last = now;
        if let Probation::Lapsed = active.shell.tick(elapsed) {
            eprintln!(
                "the revert window ran out: {:?} is back in force",
                active.shell.settings().in_force()
            );
        }
        self.refresh_readouts();
        if let Some(active) = self.active.as_ref() {
            active.window.request_redraw();
        }
    }
}

impl ShellApp {
    /// Maps one window event to a semantic event and runs whatever it triggers.
    fn route(&mut self, event_loop: &ActiveEventLoop, event: WindowEvent) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        // Whether the arrow keys mean "adjust a control" or "move a caret" is the host's decision, not the
        // interface's: it is the only side that knows a text field has focus. Sending both would
        // double-handle, and sending only one would break the other control.
        if let WindowEvent::ModifiersChanged(modifiers) = &event {
            active.shift = modifiers.state().shift_key();
            return;
        }
        let editing = active.shell.ime_wanted();
        let reversed = active.shift;
        let mut events: Vec<UiEvent> = Vec::new();
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                #[allow(clippy::cast_possible_truncation)]
                let at = [position.x as f32, position.y as f32];
                active.pointer = at;
                events.push(UiEvent::PointerMoved { x: at[0], y: at[1] });
            }
            WindowEvent::CursorLeft { .. } => events.push(UiEvent::PointerLeft),
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                let [x, y] = active.pointer;
                events.push(if state == ElementState::Pressed {
                    UiEvent::PointerPressed { x, y }
                } else {
                    UiEvent::PointerReleased { x, y }
                });
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // Negated, because a wheel turned away from the user scrolls content *up* while the
                // offset it changes counts downward from the top.
                #[allow(clippy::cast_possible_truncation)]
                let amount = match delta {
                    MouseScrollDelta::LineDelta(_, lines) => -lines * 24.0,
                    MouseScrollDelta::PixelDelta(pixels) => -(pixels.y as f32),
                };
                let [x, y] = active.pointer;
                events.push(UiEvent::Scrolled { x, y, amount });
            }
            WindowEvent::Ime(Ime::Preedit(text, cursor)) => events.push(UiEvent::Compose {
                // The byte range winit reports is turned into a character offset, because that is what a
                // field's cursor is throughout this engine.
                cursor: cursor.map(|(from, _)| text[..from.min(text.len())].chars().count()),
                text,
            }),
            WindowEvent::Ime(Ime::Commit(text)) => events.push(UiEvent::Commit(text)),
            WindowEvent::Ime(Ime::Disabled) => events.push(UiEvent::ComposeCancelled),
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    return;
                }
                match event.logical_key.as_ref() {
                    Key::Named(NamedKey::Tab) => events.push(UiEvent::Focus(if reversed {
                        FocusMove::Previous
                    } else {
                        FocusMove::Next
                    })),
                    Key::Named(NamedKey::Enter | NamedKey::Space) => {
                        events.push(UiEvent::Activate);
                    }
                    Key::Named(NamedKey::Escape) => events.push(UiEvent::Cancel),
                    Key::Named(NamedKey::ArrowLeft) => events.push(if editing {
                        UiEvent::Edit(Edit::Left)
                    } else {
                        UiEvent::Adjust(Adjust::Decrease)
                    }),
                    Key::Named(NamedKey::ArrowRight) => events.push(if editing {
                        UiEvent::Edit(Edit::Right)
                    } else {
                        UiEvent::Adjust(Adjust::Increase)
                    }),
                    Key::Named(NamedKey::ArrowUp) => {
                        events.push(UiEvent::Adjust(Adjust::Decrease));
                    }
                    Key::Named(NamedKey::ArrowDown) => {
                        events.push(UiEvent::Adjust(Adjust::Increase));
                    }
                    Key::Named(NamedKey::Backspace) => events.push(UiEvent::Edit(Edit::Backspace)),
                    Key::Named(NamedKey::Delete) => events.push(UiEvent::Edit(Edit::Delete)),
                    Key::Named(NamedKey::Home) => events.push(UiEvent::Edit(Edit::Home)),
                    Key::Named(NamedKey::End) => events.push(UiEvent::Edit(Edit::End)),
                    _ => {
                        if let Some(text) = event.text.as_deref() {
                            events.extend(
                                text.chars()
                                    .filter(|character| !character.is_control())
                                    .map(|character| UiEvent::Edit(Edit::Insert(character))),
                            );
                        }
                    }
                }
            }
            _ => return,
        }

        for event in events {
            self.handle(event_loop, event);
        }
        self.stage_antialiasing();
        self.refresh_readouts();
    }

    /// Runs one semantic event and acts on what the shell reports.
    fn handle(&mut self, event_loop: &ActiveEventLoop, event: UiEvent) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        let metrics = UiMetrics::new(&self.theme, &active.strings, active.scale);
        let outcome = active.shell.handle(event, &metrics);
        if outcome.settings_in_force {
            // Where a host would hand the new settings to the chain. There is no scene here, so it is
            // reported instead -- which is still the point: the host learns from one flag rather than by
            // comparing values it kept a copy of.
            eprintln!(
                "settings in force: {:?}",
                active.shell.settings().in_force()
            );
        }
        match outcome.request {
            Some(cic_ui::Request::Quit) => event_loop.exit(),
            Some(cic_ui::Request::LaunchSkirmish) => eprintln!(
                "launch: {:?}, fog {:?}",
                active.shell.interface().selection("skirmish_maps"),
                active.shell.interface().toggle("skirmish_fog")
            ),
            None => {}
        }
    }
}

/// Clears the surface before the interface is drawn over it.
///
/// The interface pass loads rather than clears, because it is drawn over a scene as often as onto nothing.
/// With no scene there is a backdrop to put down first, and that is a host's job rather than the pass's.
fn clear_backdrop(encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, colour: [f32; 4]) {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("shell backdrop"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color {
                    r: f64::from(colour[0]),
                    g: f64::from(colour[1]),
                    b: f64::from(colour[2]),
                    a: 1.0,
                }),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
}

fn configuration(
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> wgpu::SurfaceConfiguration {
    wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width,
        height,
        // Fifo, so the frame loop cannot spin the GPU flat out drawing a menu.
        present_mode: wgpu::PresentMode::Fifo,
        alpha_mode: wgpu::CompositeAlphaMode::Auto,
        view_formats: vec![],
        color_space: wgpu::SurfaceColorSpace::Auto,
        desired_maximum_frame_latency: 2,
    }
}
