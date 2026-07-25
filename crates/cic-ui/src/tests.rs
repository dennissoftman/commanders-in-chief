// Commanders in Chief
// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Retained-runtime tests over original synthetic layouts. No retail data appears here.

use cic_formats::{WndColor, WndDrawDataSlot, WndLimits, parse_wnd};

use crate::{
    UiClipPolicy, UiControlKind, UiEvent, UiFrameItem, UiKey, UiLayout, UiLayoutError, UiLimits,
    UiMouseButton, UiPoint, UiPresentation, UiRect, UiScalePolicy, UiStatus, UiViewport,
};

fn viewport(width: i32, height: i32) -> UiViewport {
    UiViewport::new(width, height).expect("positive viewport")
}

fn classic(width: i32, height: i32) -> UiPresentation {
    UiPresentation::new(viewport(width, height), UiScalePolicy::Classic)
}

fn draw_data(image: &str) -> String {
    let mut record = format!("IMAGE: {image}, COLOR: 10 20 30 255, BORDERCOLOR: 1 2 3 255");
    for _ in 1..9 {
        record.push_str(",\n    IMAGE: NoImage, COLOR: 0 0 0 0, BORDERCOLOR: 0 0 0 0");
    }
    record
}

/// A synthetic two-level menu: a panel holding a button, an entry field, a check box, two radio
/// buttons in one group, a slider, a list box, and a combo box.
///
/// One literal fixture is clearer than several assembled fragments, so its length is deliberate.
#[expect(
    clippy::too_many_lines,
    reason = "one literal synthetic layout covering every control family under test"
)]
fn synthetic_layout() -> String {
    format!(
        r#"FILE_VERSION = 2;
STARTLAYOUTBLOCK
  LAYOUTINIT = "SynthInit";
  LAYOUTUPDATE = "[None]";
  LAYOUTSHUTDOWN = "[None]";
ENDLAYOUTBLOCK
WINDOW
  WINDOWTYPE = USER;
  SCREENRECT = UPPERLEFT: 100 50,
               BOTTOMRIGHT: 500 450,
               CREATIONRESOLUTION: 800 600;
  NAME = "SynthMenu.wnd:PanelSynth";
  STATUS = ENABLED+IMAGE;
  TEXTCOLOR = ENABLED: 255 255 255 255, ENABLEDBORDER: 0 0 0 255,
              DISABLED: 128 128 128 255, DISABLEDBORDER: 0 0 0 255,
              HILITE: 255 255 0 255, HILITEBORDER: 0 0 0 255;
  ENABLEDDRAWDATA = {panel};
CHILD
WINDOW
  WINDOWTYPE = PUSHBUTTON;
  SCREENRECT = UPPERLEFT: 120 70,
               BOTTOMRIGHT: 220 110,
               CREATIONRESOLUTION: 800 600;
  NAME = "SynthMenu.wnd:ButtonSynth";
  STATUS = ENABLED+TABSTOP;
  TEXT = "GUI:SynthButton";
  SYSTEMCALLBACK = "SynthButtonSystem";
  TEXTCOLOR = ENABLED: 255 255 255 255, ENABLEDBORDER: 0 0 0 255,
              DISABLED: 128 128 128 255, DISABLEDBORDER: 0 0 0 255,
              HILITE: 255 255 0 255, HILITEBORDER: 0 0 0 255;
  ENABLEDDRAWDATA = {button_enabled};
  HILITEDRAWDATA = {button_hilite};
END
CHILD
WINDOW
  WINDOWTYPE = ENTRYFIELD;
  SCREENRECT = UPPERLEFT: 120 130,
               BOTTOMRIGHT: 320 160,
               CREATIONRESOLUTION: 800 600;
  NAME = "SynthMenu.wnd:EntrySynth";
  STATUS = ENABLED+TABSTOP;
  TEXTENTRYDATA = MAXLEN: 6, SECRETTEXT: 0, NUMERICALONLY: 0,
                  ALPHANUMERICALONLY: 0, ASCIIONLY: 0;
END
CHILD
WINDOW
  WINDOWTYPE = CHECKBOX;
  SCREENRECT = UPPERLEFT: 120 180,
               BOTTOMRIGHT: 150 210,
               CREATIONRESOLUTION: 800 600;
  NAME = "SynthMenu.wnd:CheckSynth";
  STATUS = ENABLED+TABSTOP;
END
CHILD
WINDOW
  WINDOWTYPE = RADIOBUTTON;
  SCREENRECT = UPPERLEFT: 120 230,
               BOTTOMRIGHT: 150 260,
               CREATIONRESOLUTION: 800 600;
  NAME = "SynthMenu.wnd:RadioFirst";
  STATUS = ENABLED;
  RADIOBUTTONDATA = GROUP: 3;
END
CHILD
WINDOW
  WINDOWTYPE = RADIOBUTTON;
  SCREENRECT = UPPERLEFT: 160 230,
               BOTTOMRIGHT: 190 260,
               CREATIONRESOLUTION: 800 600;
  NAME = "SynthMenu.wnd:RadioSecond";
  STATUS = ENABLED;
  RADIOBUTTONDATA = GROUP: 3;
END
CHILD
WINDOW
  WINDOWTYPE = HORZSLIDER;
  SCREENRECT = UPPERLEFT: 120 280,
               BOTTOMRIGHT: 320 300,
               CREATIONRESOLUTION: 800 600;
  NAME = "SynthMenu.wnd:SliderSynth";
  STATUS = ENABLED;
  SLIDERDATA = MINVALUE: 5, MAXVALUE: 9;
END
CHILD
WINDOW
  WINDOWTYPE = SCROLLLISTBOX;
  SCREENRECT = UPPERLEFT: 340 70,
               BOTTOMRIGHT: 480 200,
               CREATIONRESOLUTION: 800 600;
  NAME = "SynthMenu.wnd:ListSynth";
  STATUS = ENABLED;
  LISTBOXDATA = LENGTH: 2, AUTOSCROLL: 0, AUTOPURGE: 0, SCROLLBAR: 1,
                MULTISELECT: 0, COLUMNS: 1, FORCESELECT: 0;
END
CHILD
WINDOW
  WINDOWTYPE = COMBOBOX;
  SCREENRECT = UPPERLEFT: 340 220,
               BOTTOMRIGHT: 480 250,
               CREATIONRESOLUTION: 800 600;
  NAME = "SynthMenu.wnd:ComboSynth";
  STATUS = ENABLED;
  COMBOBOXDATA = ISEDITABLE: 0, MAXCHARS: 8, MAXDISPLAY: 3, ASCIIONLY: 0,
                 LETTERSANDNUMBERS: 0;
END
ENDALLCHILDREN
END
"#,
        panel = draw_data("SynthPanel"),
        button_enabled = draw_data("SynthButtonEnabled"),
        button_hilite = draw_data("SynthButtonHilite"),
    )
}

fn instantiate(presentation: UiPresentation) -> UiLayout {
    let source = synthetic_layout();
    let document = parse_wnd(source.as_bytes(), WndLimits::default()).expect("decode layout");
    assert!(
        document.diagnostics().is_empty(),
        "synthetic fixture should decode cleanly: {:?}",
        document.diagnostics()
    );
    UiLayout::instantiate(&document, presentation, UiLimits::default()).expect("instantiate layout")
}

#[test]
fn instantiation_preserves_hierarchy_and_source_order() {
    let layout = instantiate(classic(800, 600));
    assert_eq!(layout.roots().len(), 1);
    let panel = layout.roots()[0];
    assert_eq!(layout.control(panel).children().len(), 8);
    let names: Vec<&str> = layout
        .control(panel)
        .children()
        .iter()
        .map(|child| layout.control(*child).name().unwrap_or("-"))
        .collect();
    assert_eq!(
        names,
        [
            "SynthMenu.wnd:ButtonSynth",
            "SynthMenu.wnd:EntrySynth",
            "SynthMenu.wnd:CheckSynth",
            "SynthMenu.wnd:RadioFirst",
            "SynthMenu.wnd:RadioSecond",
            "SynthMenu.wnd:SliderSynth",
            "SynthMenu.wnd:ListSynth",
            "SynthMenu.wnd:ComboSynth",
        ]
    );
    assert!(layout.duplicate_names().is_empty());
    assert!(layout.diagnostics().is_empty());
    // Callback names are retained as data on the control that declared them.
    let button = layout.find("SynthMenu.wnd:ButtonSynth").expect("button");
    assert_eq!(
        layout.control(button).system_callback(),
        Some("SynthButtonSystem")
    );
}

#[test]
fn child_rectangles_are_parent_relative_and_unscaled_at_creation_resolution() {
    let layout = instantiate(classic(800, 600));
    let panel = layout.roots()[0];
    assert_eq!(
        layout.control(panel).rect(),
        UiRect {
            x: 100,
            y: 50,
            width: 400,
            height: 400
        }
    );
    let button = layout.find("SynthMenu.wnd:ButtonSynth").expect("button");
    // Stored 120,70 minus the parent's 100,50 origin.
    assert_eq!(
        layout.control(button).rect(),
        UiRect {
            x: 20,
            y: 20,
            width: 100,
            height: 40
        }
    );
    assert_eq!(
        layout.screen_rect(button),
        UiRect {
            x: 120,
            y: 70,
            width: 100,
            height: 40
        }
    );
}

#[test]
fn the_classic_policy_stretches_each_axis_and_truncates_like_the_source() {
    // 1600x900 against an 800x600 creation resolution: 2.0 horizontally, 1.5 vertically.
    let layout = instantiate(classic(1600, 900));
    let panel = layout.roots()[0];
    assert_eq!(
        layout.control(panel).rect(),
        UiRect {
            x: 200,
            y: 75,
            width: 800,
            height: 600
        }
    );
    let button = layout.find("SynthMenu.wnd:ButtonSynth").expect("button");
    assert_eq!(
        layout.screen_rect(button),
        UiRect {
            x: 240,
            y: 105,
            width: 200,
            height: 60
        }
    );
    // A non-uniform ratio stretches, which is exactly what the original does: the authored
    // 100x40 button becomes 200x60 rather than keeping its 2.5 aspect ratio.
    let stretched = layout.screen_rect(button);
    assert_ne!(stretched.width * 40, stretched.height * 100);
}

#[test]
fn the_modern_policy_scales_uniformly_and_centres() {
    let presentation = UiPresentation::new(viewport(1600, 900), UiScalePolicy::Modern);
    let layout = instantiate(presentation);
    // The smaller ratio is 900/600 = 1.5, so the scaled 800x600 layout is 1200x900 and is
    // centred horizontally by (1600 - 1200) / 2 = 200.
    let panel = layout.roots()[0];
    assert_eq!(
        layout.control(panel).rect(),
        UiRect {
            x: 350,
            y: 75,
            width: 600,
            height: 600
        }
    );
    let button = layout.find("SynthMenu.wnd:ButtonSynth").expect("button");
    let rect = layout.screen_rect(button);
    // The authored 100x40 button keeps its 2.5 aspect ratio.
    assert_eq!(rect.width, 150);
    assert_eq!(rect.height, 60);
}

#[test]
fn hit_testing_descends_to_the_deepest_visible_enabled_control() {
    let layout = instantiate(classic(800, 600));
    let panel = layout.roots()[0];
    let button = layout.find("SynthMenu.wnd:ButtonSynth").expect("button");

    assert_eq!(layout.hit_test(UiPoint::new(150, 90)), Some(button));
    // Inside the panel but not any child.
    assert_eq!(layout.hit_test(UiPoint::new(460, 430)), Some(panel));
    // Outside every root.
    assert_eq!(layout.hit_test(UiPoint::new(10, 10)), None);
    // Both edges are inclusive, matching winPointInWindow.
    assert_eq!(layout.hit_test(UiPoint::new(220, 110)), Some(button));
    assert_eq!(layout.hit_test(UiPoint::new(221, 111)), Some(panel));
}

#[test]
fn a_hidden_or_disabled_control_falls_through_to_its_parent() {
    let mut layout = instantiate(classic(800, 600));
    let panel = layout.roots()[0];
    let button = layout.find("SynthMenu.wnd:ButtonSynth").expect("button");

    layout.set_hidden(button, true);
    assert_eq!(layout.hit_test(UiPoint::new(150, 90)), Some(panel));
    layout.set_hidden(button, false);
    assert_eq!(layout.hit_test(UiPoint::new(150, 90)), Some(button));

    layout.set_enabled(button, false);
    assert_eq!(layout.hit_test(UiPoint::new(150, 90)), Some(panel));

    // Hiding the parent hides the subtree.
    layout.set_enabled(button, true);
    layout.set_hidden(panel, true);
    assert_eq!(layout.hit_test(UiPoint::new(150, 90)), None);
    assert!(!layout.is_effectively_visible(button));
}

#[test]
fn press_and_release_activate_only_when_the_release_lands_on_the_control() {
    let mut layout = instantiate(classic(800, 600));
    let button = layout.find("SynthMenu.wnd:ButtonSynth").expect("button");
    let panel = layout.roots()[0];

    let events = layout.pointer_pressed(UiPoint::new(150, 90), UiMouseButton::Left);
    assert!(events.contains(&UiEvent::Pressed {
        control: button,
        button: UiMouseButton::Left
    }));
    assert!(layout.control(button).is_pressed());
    assert_eq!(layout.capture(), Some(button));

    let events = layout.pointer_released(UiPoint::new(150, 90), UiMouseButton::Left);
    assert!(events.iter().any(|event| matches!(
        event,
        UiEvent::Activated { control, callback: Some(callback), .. }
            if *control == button && callback == "SynthButtonSystem"
    )));
    assert!(!layout.control(button).is_pressed());
    assert_eq!(layout.capture(), None);

    // Releasing away from the pressed control cancels instead of activating.
    layout.pointer_pressed(UiPoint::new(150, 90), UiMouseButton::Left);
    let events = layout.pointer_released(UiPoint::new(460, 430), UiMouseButton::Left);
    assert_eq!(events, vec![UiEvent::PressCancelled { control: button }]);
    assert_eq!(layout.hit_test(UiPoint::new(460, 430)), Some(panel));
}

#[test]
fn hover_transitions_are_emitted_once_per_change() {
    let mut layout = instantiate(classic(800, 600));
    let button = layout.find("SynthMenu.wnd:ButtonSynth").expect("button");
    assert_eq!(
        layout.pointer_moved(UiPoint::new(150, 90)),
        vec![UiEvent::HoverEntered { control: button }]
    );
    assert!(layout.pointer_moved(UiPoint::new(151, 91)).is_empty());
    assert_eq!(
        layout.pointer_moved(UiPoint::new(10, 10)),
        vec![UiEvent::HoverLeft { control: button }]
    );
}

#[test]
fn a_right_click_only_reaches_a_control_that_asks_for_it() {
    let mut layout = instantiate(classic(800, 600));
    assert!(
        layout
            .pointer_pressed(UiPoint::new(150, 90), UiMouseButton::Right)
            .is_empty()
    );
    assert_eq!(layout.capture(), None);
}

#[test]
fn radio_buttons_are_exclusive_within_their_group() {
    let mut layout = instantiate(classic(800, 600));
    let first = layout
        .find("SynthMenu.wnd:RadioFirst")
        .expect("first radio");
    let second = layout
        .find("SynthMenu.wnd:RadioSecond")
        .expect("second radio");

    layout.select_radio(first);
    assert!(matches!(
        layout.control(first).kind(),
        UiControlKind::RadioButton {
            selected: true,
            group: 3
        }
    ));
    layout.select_radio(second);
    assert!(matches!(
        layout.control(first).kind(),
        UiControlKind::RadioButton {
            selected: false,
            ..
        }
    ));
    assert!(matches!(
        layout.control(second).kind(),
        UiControlKind::RadioButton { selected: true, .. }
    ));
}

#[test]
fn check_boxes_toggle_and_sliders_clamp() {
    let mut layout = instantiate(classic(800, 600));
    let check = layout.find("SynthMenu.wnd:CheckSynth").expect("check box");
    assert_eq!(layout.toggle_check(check), Some(true));
    assert_eq!(layout.toggle_check(check), Some(false));

    let slider = layout.find("SynthMenu.wnd:SliderSynth").expect("slider");
    assert!(matches!(
        layout.control(slider).kind(),
        UiControlKind::Slider {
            minimum: 5,
            maximum: 9,
            value: 5
        }
    ));
    assert_eq!(layout.set_slider_value(slider, 7), Some(7));
    assert_eq!(layout.set_slider_value(slider, 100), Some(9));
    assert_eq!(layout.set_slider_value(slider, -100), Some(5));
}

#[test]
fn list_and_combo_selection_stay_within_bounds() {
    let mut layout = instantiate(classic(800, 600));
    let list = layout.find("SynthMenu.wnd:ListSynth").expect("list");
    assert!(!layout.select_list_row(list, 0, false));
    for row in 0..4 {
        assert!(layout.push_list_row(list, format!("Row {row}")));
    }
    assert!(layout.select_list_row(list, 3, false));
    assert!(!layout.select_list_row(list, 4, false));
    // Single-select replaces rather than accumulating, even when asked to add.
    assert!(layout.select_list_row(list, 1, true));
    assert!(matches!(
        layout.control(list).kind(),
        UiControlKind::ListBox { selected, .. } if selected.as_slice() == [1]
    ));
    // Scrolling clamps so the final page stays full: 4 rows, 2 visible.
    assert_eq!(layout.scroll_list(list, 99), Some(2));

    let combo = layout.find("SynthMenu.wnd:ComboSynth").expect("combo");
    assert!(!layout.select_combo_entry(combo, 0));
    assert!(layout.push_combo_entry(combo, "One"));
    assert!(layout.select_combo_entry(combo, 0));
    assert!(!layout.select_combo_entry(combo, 1));
}

#[test]
fn text_entry_honors_its_declared_length_in_characters() {
    let mut layout = instantiate(classic(800, 600));
    let entry = layout.find("SynthMenu.wnd:EntrySynth").expect("entry");
    layout.set_focus(Some(entry));
    assert_eq!(layout.focus(), Some(entry));

    // MAXLEN is 6, and the count is characters rather than bytes.
    let events = layout.insert_text("aéióuñ");
    assert_eq!(events, vec![UiEvent::TextChanged { control: entry }]);
    assert!(layout.insert_text("x").is_empty());
    assert!(matches!(
        layout.control(entry).kind(),
        UiControlKind::TextEntry { text, caret: 6, .. } if text == "aéióuñ"
    ));

    // Editing keys operate on characters, not bytes.
    layout.press_key(UiKey::Backspace);
    assert!(matches!(
        layout.control(entry).kind(),
        UiControlKind::TextEntry { text, caret: 5, .. } if text == "aéióu"
    ));
    layout.press_key(UiKey::Home);
    layout.press_key(UiKey::Delete);
    assert!(matches!(
        layout.control(entry).kind(),
        UiControlKind::TextEntry { text, caret: 0, .. } if text == "éióu"
    ));
}

#[test]
fn tab_traversal_visits_declared_stops_and_wraps() {
    let mut layout = instantiate(classic(800, 600));
    let button = layout.find("SynthMenu.wnd:ButtonSynth").expect("button");
    let entry = layout.find("SynthMenu.wnd:EntrySynth").expect("entry");
    let check = layout.find("SynthMenu.wnd:CheckSynth").expect("check");
    assert_eq!(layout.tab_order(), [button, entry, check]);

    layout.focus_next();
    assert_eq!(layout.focus(), Some(button));
    layout.focus_next();
    assert_eq!(layout.focus(), Some(entry));
    layout.focus_next();
    assert_eq!(layout.focus(), Some(check));
    layout.focus_next();
    assert_eq!(layout.focus(), Some(button));
    layout.focus_previous();
    assert_eq!(layout.focus(), Some(check));

    // A disabled stop is skipped rather than trapping focus.
    layout.set_enabled(entry, false);
    layout.set_focus(Some(button));
    layout.focus_next();
    assert_eq!(layout.focus(), Some(check));
}

#[test]
fn hiding_the_focused_control_releases_focus_and_press_state() {
    let mut layout = instantiate(classic(800, 600));
    let button = layout.find("SynthMenu.wnd:ButtonSynth").expect("button");
    layout.pointer_pressed(UiPoint::new(150, 90), UiMouseButton::Left);
    assert_eq!(layout.focus(), Some(button));
    layout.set_hidden(button, true);
    assert_eq!(layout.focus(), None);
    assert_eq!(layout.capture(), None);
    assert!(!layout.control(button).is_pressed());
}

#[test]
fn the_frame_submits_parents_before_children_and_selects_the_state_slot() {
    let mut layout = instantiate(classic(800, 600));
    let panel = layout.roots()[0];
    let button = layout.find("SynthMenu.wnd:ButtonSynth").expect("button");

    let frame = layout.frame(UiClipPolicy::None);
    let quads: Vec<_> = frame
        .items()
        .iter()
        .filter_map(|item| match item {
            UiFrameItem::Quad {
                control,
                slot,
                image,
                ..
            } => Some((*control, *slot, image.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(quads[0].0, panel);
    assert_eq!(quads[0].2.as_deref(), Some("SynthPanel"));
    assert_eq!(quads[1].0, button);
    assert_eq!(quads[1].1, WndDrawDataSlot::Enabled);
    assert_eq!(quads[1].2.as_deref(), Some("SynthButtonEnabled"));

    // Hovering selects the hilite slot.
    layout.pointer_moved(UiPoint::new(150, 90));
    let frame = layout.frame(UiClipPolicy::None);
    assert!(frame.items().iter().any(|item| matches!(
        item,
        UiFrameItem::Quad { control, slot: WndDrawDataSlot::Hilite, image: Some(image), .. }
            if *control == button && image == "SynthButtonHilite"
    )));

    // Disabling selects the disabled slot, which this control does not declare, so it draws
    // without an image rather than reusing the enabled visual.
    layout.pointer_moved(UiPoint::new(10, 10));
    layout.set_enabled(button, false);
    let frame = layout.frame(UiClipPolicy::None);
    assert!(frame.items().iter().any(|item| matches!(
        item,
        UiFrameItem::Quad { control, slot: WndDrawDataSlot::Disabled, image: None, color: None, .. }
            if *control == button
    )));
}

#[test]
fn the_frame_carries_text_runs_with_state_colour() {
    let layout = instantiate(classic(800, 600));
    let button = layout.find("SynthMenu.wnd:ButtonSynth").expect("button");
    let frame = layout.frame(UiClipPolicy::None);
    let run = frame
        .items()
        .iter()
        .find_map(|item| match item {
            UiFrameItem::Text(run) if run.control == button => Some(run),
            _ => None,
        })
        .expect("button text run");
    assert_eq!(run.label, "GUI:SynthButton");
    assert_eq!(
        run.color.map(WndColor::channels),
        Some([255, 255, 255, 255])
    );
    assert!(!run.masked);
}

#[test]
fn a_hidden_subtree_and_a_see_thru_parent_are_handled_separately() {
    let mut layout = instantiate(classic(800, 600));
    let panel = layout.roots()[0];
    let before = layout.frame(UiClipPolicy::None).len();
    layout.set_hidden(panel, true);
    assert!(layout.frame(UiClipPolicy::None).is_empty());

    layout.set_hidden(panel, false);
    let clipped = layout.frame(UiClipPolicy::ClipToParent);
    assert!(
        clipped.len() > before,
        "clipping adds push and pop instructions"
    );
    assert!(matches!(
        clipped.items().first(),
        Some(UiFrameItem::Quad { .. })
    ));
    assert!(
        clipped
            .items()
            .iter()
            .any(|item| matches!(item, UiFrameItem::PushClip { .. }))
    );
    assert!(
        clipped
            .items()
            .iter()
            .any(|item| matches!(item, UiFrameItem::PopClip))
    );
}

#[test]
fn declared_status_seeds_live_state_and_unmapped_names_are_reported() {
    let source = "FILE_VERSION = 2;\n\
         STARTLAYOUTBLOCK\n\
           LAYOUTINIT = \"[None]\";\n\
           LAYOUTUPDATE = \"[None]\";\n\
           LAYOUTSHUTDOWN = \"[None]\";\n\
         ENDLAYOUTBLOCK\n\
         WINDOW\n\
           WINDOWTYPE = PUSHBUTTON;\n\
           SCREENRECT = UPPERLEFT: 0 0,\n\
                        BOTTOMRIGHT: 10 10,\n\
                        CREATIONRESOLUTION: 800 600;\n\
           NAME = \"SynthMenu.wnd:ButtonHidden\";\n\
           STATUS = HIDDEN+NOFOCUS+SPARKLE;\n\
         END\n";
    let document = parse_wnd(source.as_bytes(), WndLimits::default()).expect("decode layout");
    let mut layout = UiLayout::instantiate(&document, classic(800, 600), UiLimits::default())
        .expect("instantiate layout");
    let button = layout.roots()[0];
    assert!(layout.control(button).is_hidden());
    assert!(!layout.control(button).is_enabled());
    assert!(layout.control(button).status().contains(UiStatus::NO_FOCUS));
    // `SPARKLE` is outside the established vocabulary; it is reported rather than ignored. Every
    // real status name is mapped, so retail layouts produce no such diagnostic.
    assert_eq!(layout.diagnostics().len(), 1);

    // A control refusing focus never receives it.
    assert!(layout.set_focus(Some(button)).is_empty());
    assert_eq!(layout.focus(), None);
}

#[test]
fn limits_and_invalid_inputs_are_structured_errors() {
    assert_eq!(
        UiViewport::new(0, 600),
        Err(UiLayoutError::InvalidViewport {
            width: 0,
            height: 600
        })
    );
    let source = synthetic_layout();
    let document = parse_wnd(source.as_bytes(), WndLimits::default()).expect("decode layout");
    let limits = UiLimits {
        max_controls: 4,
        ..UiLimits::default()
    };
    assert_eq!(
        UiLayout::instantiate(&document, classic(800, 600), limits).err(),
        Some(UiLayoutError::TooManyControls { limit: 4 })
    );
    let limits = UiLimits {
        max_depth: 1,
        ..UiLimits::default()
    };
    assert_eq!(
        UiLayout::instantiate(&document, classic(800, 600), limits).err(),
        Some(UiLayoutError::TooDeep { limit: 1 })
    );
}

#[test]
fn instantiation_is_deterministic_for_the_same_inputs() {
    let first = instantiate(classic(1366, 768));
    let second = instantiate(classic(1366, 768));
    assert_eq!(first.controls(), second.controls());
    assert_eq!(
        first.frame(UiClipPolicy::None),
        second.frame(UiClipPolicy::None)
    );
}
