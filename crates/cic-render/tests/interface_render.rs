//! Capture tests for the interface pass, over the shell's own authored screens.
//!
//! # Why these are captures rather than assertions
//!
//! This project's standing rule: a green test suite is not verification for anything drawn. Every
//! rendering bug in it so far passed its own assertions and was found by opening the image. Interface
//! drawing has more of the same class than the scene did, because most of it is arithmetic over
//! rectangles whose failures are *plausible* — a border one pixel out, a caret at the wrong character, a
//! label centred in the wrong box, a glyph resampled because its quad was not snapped. None of those
//! reads as broken in a number.
//!
//! So the coverage here is the harness M3 built: render, capture, compare against a committed reference
//! that somebody has looked at.
//!
//! # Why the layouts are the real ones
//!
//! A fixture can be the bug — twice already in this tree. A layout constructed by a test to exercise the
//! painter would be a layout nobody ships, and it would go on passing while the four screens the game
//! actually navigates rotted. These load `content/ui/*.ciclayout.json`, so a change that breaks an
//! authored screen breaks this.

mod support;

use std::fmt::Write as _;
use std::sync::OnceLock;

use cic_render::gpu::{CAPTURE_FORMAT, Capture, CaptureTarget, GpuContext};
use cic_render::text::{Font, GlyphAtlas};
use cic_render::ui::{DrawList, UiMetrics, UiRenderer, atlas_sizes};
use cic_ui::paint::{Painter, Theme};
use cic_ui::solve::solve;
use cic_ui::{
    Action, Interface, Layout, Screen, Screens, Shell, StringTable, UiEvent, Value, Viewport,
};

/// The captured size. Wide enough for the settings screen's rows to be readable and small enough that a
/// reference is a few tens of kilobytes.
const SIZE: [u32; 2] = [640, 400];

/// One device for the whole binary, as the other capture targets do.
///
/// Not merely a saving. Four devices on one adapter, created and destroyed concurrently by the test
/// harness, crashed the driver outright -- an access violation rather than a failed test, so the run
/// reported nothing at all about the images.
static CONTEXT: OnceLock<Option<GpuContext>> = OnceLock::new();

fn context() -> Option<&'static GpuContext> {
    CONTEXT.get_or_init(support::shared_context).as_ref()
}

/// Stands in for the host's display settings, which is all the shell needs of them.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Display {
    scale: f32,
    antialiasing: bool,
}

/// Loads the shell's authored screens.
fn screens() -> Screens {
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
        let layout = Layout::from_json(bytes)
            .unwrap_or_else(|error| panic!("{} does not load: {error}", screen.slug()));
        screens.insert(screen, layout);
    }
    screens
}

fn strings() -> StringTable {
    StringTable::from_json(include_bytes!("../../../content/ui/strings.en.json"))
        .expect("the string table loads")
}

fn viewport() -> Viewport {
    Viewport::new(SIZE[0], SIZE[1], 1.0).expect("a valid viewport")
}

/// Builds the shell over the authored screens.
fn shell() -> Shell<Display> {
    let theme = Theme::default();
    let strings = strings();
    let metrics = UiMetrics::new(&theme, &strings, 1.0);
    Shell::new(
        screens(),
        strings.clone(),
        Display {
            scale: 1.0,
            antialiasing: false,
        },
        viewport(),
        &metrics,
    )
    .expect("every screen has a layout")
}

/// Renders whatever the shell currently shows and reads it back.
fn capture(context: &GpuContext, shell: &Shell<Display>) -> Capture {
    let theme = Theme::default();
    let metrics = UiMetrics::new(&theme, shell.strings(), 1.0);
    let atlas = GlyphAtlas::new(&Font::new(), &atlas_sizes(&theme, 1.0));
    let painter = Painter::new(&theme, &metrics, shell.viewport());

    let mut list = DrawList::new();
    for (screen, solved) in shell.drawn() {
        // Only the top screen's state is live; a screen beneath a modal keeps its own, which is the whole
        // point of the stack, so it is drawn with what it remembers rather than with the modal's.
        let blank = Interface::new();
        let interface = shell.stack().interface_for(*screen).unwrap_or(&blank);
        list.extend(&painter.paint(solved, interface, shell.strings()), &atlas);
    }
    assert!(!list.is_empty(), "the shell drew nothing at all");

    let target = CaptureTarget::new(context, SIZE[0], SIZE[1]).expect("a capture target");
    let mut renderer = UiRenderer::new(context.device(), context.queue(), CAPTURE_FORMAT, &atlas);
    let mut encoder = context
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("interface capture"),
        });
    // The interface pass loads rather than clears, because it is drawn over a scene as often as onto
    // nothing. So the backdrop is cleared here first, which is what a host without a scene does.
    clear(&mut encoder, target.colour_view(), theme_backdrop());
    renderer.draw(
        context.device(),
        context.queue(),
        &mut encoder,
        target.colour_view(),
        SIZE,
        &list,
    );
    // `resolve` submits the encoder itself after appending the copy, so the pass above must not be
    // submitted separately -- doing both would order the readback before the drawing.
    target
        .resolve(context, encoder)
        .expect("read the capture back")
}

/// The theme's backdrop as a clear colour, linear because the target is sRGB-encoded.
fn theme_backdrop() -> wgpu::Color {
    let linear = Theme::default().backdrop.to_linear();
    wgpu::Color {
        r: f64::from(linear[0]),
        g: f64::from(linear[1]),
        b: f64::from(linear[2]),
        a: f64::from(linear[3]),
    }
}

fn clear(encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, colour: wgpu::Color) {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("interface backdrop"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(colour),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
}

#[test]
fn every_authored_screen_names_a_string_the_table_defines() {
    // Not a rendering test, and it does not need an adapter. A missing key draws as the key, which is
    // deliberate and legible -- but it is still a packaging mistake, and finding it here names every one
    // at once rather than one per screenshot.
    let shell = shell();
    assert!(
        shell.missing_strings().is_empty(),
        "the shell names strings the table does not define: {:?}",
        shell.missing_strings()
    );
}

#[test]
fn the_authored_screens_solve_without_collapsing() {
    // The cheapest guard against an authoring mistake that a capture would show as a blank area: a
    // screen whose root has no extent, or whose controls all landed at zero size.
    let theme = Theme::default();
    let strings = strings();
    let metrics = UiMetrics::new(&theme, &strings, 1.0);
    let screens = screens();
    for screen in Screen::ALL {
        let layout = screens.get(*screen).expect("a layout");
        let solved = solve(layout, viewport(), &metrics);
        let root = solved.nodes().first().expect("a root");
        assert!(
            root.rect.width > 0.0 && root.rect.height > 0.0,
            "{} solved to nothing",
            screen.slug()
        );
        let drawable = solved
            .nodes()
            .iter()
            .filter(|node| node.rect.width > 1.0 && node.rect.height > 1.0)
            .count();
        assert!(
            drawable >= solved.len() - 1,
            "{} has {} of {} nodes with no extent",
            screen.slug(),
            solved.len() - drawable,
            solved.len()
        );
    }
}

#[test]
fn the_main_menu_draws() {
    let Some(context) = context() else {
        return;
    };
    let shell = shell();
    support::check_reference(context, "ui-main-menu.png", &capture(context, &shell));
}

#[test]
fn the_settings_screen_draws_every_widget_kind_it_has() {
    // The screen with the most in it: a slider mid-range, a checkbox on, a focused text entry with a
    // caret, a warning label, and four buttons one of which is hovered. If any of those is drawn wrong
    // this is the capture that says so.
    let Some(context) = context() else {
        return;
    };
    let theme = Theme::default();
    let strings = strings();
    let metrics = UiMetrics::new(&theme, &strings, 1.0);
    let mut shell = shell();
    shell.act(Action::OpenSettings, &metrics);
    shell.interface_mut().set_slide("settings_scale", 1.25);
    shell
        .interface_mut()
        .set_text("settings_scale_value", "1.25x");
    shell.interface_mut().set_toggle("settings_antialias", true);
    shell
        .interface_mut()
        .set_text("settings_profile", "Ardennes");
    shell.interface_mut().set_focus(Some("settings_profile"));
    // Hover over the apply button, so the capture pins one control's hovered face too.
    let apply = shell
        .solved()
        .and_then(|solved| solved.by_id("settings_apply"))
        .map(|node| node.rect)
        .expect("the apply button is in the layout");
    shell.handle(
        UiEvent::PointerMoved {
            x: apply.x + apply.width / 2.0,
            y: apply.y + apply.height / 2.0,
        },
        &metrics,
    );
    support::check_reference(context, "ui-settings.png", &capture(context, &shell));
}

#[test]
fn a_modal_draws_over_the_screen_it_covers() {
    // The stack's own property, as an image: the skirmish screen is still there behind the scrim, dimmed
    // rather than replaced, with its list selection and typed name intact.
    let Some(context) = context() else {
        return;
    };
    let theme = Theme::default();
    let strings = strings();
    let metrics = UiMetrics::new(&theme, &strings, 1.0);
    let mut shell = shell();
    shell.act(Action::OpenSkirmishSetup, &metrics);
    shell.interface_mut().set_selection("skirmish_maps", 1);
    shell.interface_mut().set_toggle("skirmish_fog", true);
    shell
        .interface_mut()
        .set_text("skirmish_commander", "Mitin");
    shell.act(Action::OpenQuitConfirm, &metrics);
    shell.interface_mut().set_focus(Some("quit_cancel"));
    assert_eq!(
        shell.drawn().len(),
        2,
        "the modal must not replace the screen"
    );
    support::check_reference(context, "ui-modal.png", &capture(context, &shell));
}

#[test]
fn a_scrolled_container_is_clipped_to_itself() {
    // The one thing the four authored screens do not exercise, and the one place the drawing layer keeps
    // state of its own: a scroll offset shifts contents and a scissor confines them. Built here rather
    // than authored into a screen, because no screen needs one yet and a layout added only to be
    // photographed would be a fixture pretending to be content.
    let Some(context) = context() else {
        return;
    };
    let theme = Theme::default();
    let mut strings = StringTable::new();
    for row in 0..12 {
        strings.set(format!("row.{row}"), format!("Row {row} of twelve"));
    }
    // The container is inset from the surface on purpose: a scroll container that filled the viewport
    // would have a clip equal to the viewport's own, and the scissor would be indistinguishable from no
    // scissor at all -- which is a fixture that cannot show what it is testing.
    let mut rows = String::from(
        r#"{"format_version":1,"root":{"width":{"fill":1},"height":{"fill":1},
           "direction":"column","justify":"center","align":"center","children":[
           {"id":"scroller","widget":"scroll","width":{"fixed":420.0},"height":{"fixed":260.0},
            "direction":"column","gap":6.0,
            "padding":{"left":16.0,"top":16.0,"right":16.0,"bottom":16.0},"children":["#,
    );
    for row in 0..12 {
        if row > 0 {
            rows.push(',');
        }
        write!(
            rows,
            r#"{{"style":"card","widget":"panel","height":{{"fixed":34.0}},"direction":"column",
                "padding":{{"left":10.0,"top":6.0,"right":10.0,"bottom":6.0}},"children":[
                {{"widget":"label","text_key":"row.{row}","height":{{"fill":1}}}}]}}"#
        )
        .expect("a String never fails to write");
    }
    rows.push_str("]}]}}");
    let layout = Layout::from_json(rows.as_bytes()).expect("the scroll fixture loads");

    let metrics = UiMetrics::new(&theme, &strings, 1.0);
    let solved = solve(&layout, viewport(), &metrics);
    let mut interface = Interface::new();
    interface.set("scroller", Value::Scroll(90.0));
    let atlas = GlyphAtlas::new(&Font::new(), &atlas_sizes(&theme, 1.0));
    let painted = Painter::new(&theme, &metrics, viewport()).paint(&solved, &interface, &strings);
    let mut list = DrawList::new();
    list.extend(&painted, &atlas);
    assert!(
        list.runs() >= 2,
        "a scrollable container needs a scissor of its own"
    );

    let target = CaptureTarget::new(context, SIZE[0], SIZE[1]).expect("a capture target");
    let mut renderer = UiRenderer::new(context.device(), context.queue(), CAPTURE_FORMAT, &atlas);
    let mut encoder = context
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("interface scroll capture"),
        });
    clear(&mut encoder, target.colour_view(), theme_backdrop());
    renderer.draw(
        context.device(),
        context.queue(),
        &mut encoder,
        target.colour_view(),
        SIZE,
        &list,
    );
    let capture = target
        .resolve(context, encoder)
        .expect("read the capture back");
    support::check_reference(context, "ui-scrolled.png", &capture);
}
