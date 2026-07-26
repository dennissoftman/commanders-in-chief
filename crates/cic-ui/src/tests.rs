// Commanders in Chief
// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Retained-runtime tests over original synthetic layouts. No retail data appears here.

use cic_formats::{
    TransitionStyle, UiIniLimits, WndColor, WndDrawDataSlot, WndLimits,
    parse_window_transitions_ini, parse_wnd,
};

use crate::{
    UI_DISPLAY_CONFIRM_TIMEOUT_MS, UiDisplayCapability, UiDisplayCatalog, UiDisplayOutcome,
    UiDisplaySelection, UiDisplayTransaction, UiMonitor, UiRollbackReason, UiScaleChoice,
    UiSelectionError, UiVideoMode, UiWindowMode,
};
use crate::{
    UI_MAX_SHELL_STACK, UiActionAllowlist, UiCallbackBinding, UiCallbackSlot, UiCallbackTable,
    UiClipPolicy, UiControlFamily, UiControlId, UiControlKind, UiDemoAction, UiEvent, UiFrameItem,
    UiGadgetRole, UiKey, UiLayout, UiLayoutError, UiLimits, UiMouseButton, UiPoint, UiPresentation,
    UiRect, UiScalePolicy, UiScreen, UiShell, UiShellError, UiShellEvent, UiStatus, UiTextAlign,
    UiTransitionDiagnosticKind, UiTransitionDraw, UiTransitionHandler, UiViewport,
    classify_callback, is_none_callback,
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
/// buttons in one group, a slider, a list box, a combo box, a tab control, and three static texts
/// that differ only in which draw procedure they name.
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
  STYLE = PUSHBUTTON+MOUSETRACK;
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
  LISTBOXENABLEDUPBUTTONDRAWDATA = {up_button};
  SLIDERTHUMBENABLEDDRAWDATA = {thumb};
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
  COMBOBOXDROPDOWNBUTTONENABLEDDRAWDATA = {drop_button};
  COMBOBOXEDITBOXENABLEDDRAWDATA = {edit_box};
END
CHILD
WINDOW
  WINDOWTYPE = TABCONTROL;
  SCREENRECT = UPPERLEFT: 340 260,
               BOTTOMRIGHT: 480 400,
               CREATIONRESOLUTION: 800 600;
  NAME = "SynthMenu.wnd:TabsSynth";
  STATUS = ENABLED+IMAGE;
  TABCONTROLDATA = TABORIENTATION: 1, TABEDGE: 3, TABWIDTH: 40, TABHEIGHT: 20,
                   TABCOUNT: 2, PANEBORDER: 4, PANEDISABLED: 2 0 0;
END
CHILD
WINDOW
  WINDOWTYPE = STATICTEXT;
  SCREENRECT = UPPERLEFT: 120 320,
               BOTTOMRIGHT: 300 340,
               CREATIONRESOLUTION: 800 600;
  NAME = "SynthMenu.wnd:ColorSynth";
  STATUS = ENABLED+IMAGE;
  DRAWCALLBACK = "GadgetStaticTextDraw";
  STATICTEXTDATA = CENTERED: 0;
END
CHILD
WINDOW
  WINDOWTYPE = STATICTEXT;
  SCREENRECT = UPPERLEFT: 120 350,
               BOTTOMRIGHT: 300 370,
               CREATIONRESOLUTION: 800 600;
  NAME = "SynthMenu.wnd:ImageSynth";
  STATUS = ENABLED;
  DRAWCALLBACK = "GadgetStaticTextImageDraw";
  STATICTEXTDATA = CENTERED: 0;
END
CHILD
WINDOW
  WINDOWTYPE = STATICTEXT;
  SCREENRECT = UPPERLEFT: 120 380,
               BOTTOMRIGHT: 300 400,
               CREATIONRESOLUTION: 800 600;
  NAME = "SynthMenu.wnd:NoneSynth";
  STATUS = ENABLED+IMAGE;
  DRAWCALLBACK = "[None]";
  STATICTEXTDATA = CENTERED: 0;
END
ENDALLCHILDREN
END
"#,
        panel = draw_data("SynthPanel"),
        button_enabled = draw_data("SynthButtonEnabled"),
        button_hilite = draw_data("SynthButtonHilite"),
        drop_button = draw_data("SynthDropButton"),
        edit_box = draw_data("SynthEditBox"),
        up_button = draw_data("SynthUpButton"),
        thumb = draw_data("SynthThumb"),
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

/// Returns the synthesised gadget part of a control, by role.
fn part(layout: &UiLayout, owner: UiControlId, role: UiGadgetRole) -> UiControlId {
    *layout
        .control(owner)
        .children()
        .iter()
        .find(|child| layout.control(**child).gadget_role() == Some(role))
        .unwrap_or_else(|| panic!("{owner:?} should have a {} part", role.name()))
}

#[test]
fn instantiation_preserves_hierarchy_and_source_order() {
    let layout = instantiate(classic(800, 600));
    assert_eq!(layout.roots().len(), 1);
    let panel = layout.roots()[0];
    assert_eq!(layout.control(panel).children().len(), 12);
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
            "SynthMenu.wnd:TabsSynth",
            "SynthMenu.wnd:ColorSynth",
            "SynthMenu.wnd:ImageSynth",
            "SynthMenu.wnd:NoneSynth",
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
fn a_combo_box_builds_the_three_parts_its_gadget_creation_makes() {
    let layout = instantiate(classic(800, 600));
    let combo = layout.find("SynthMenu.wnd:ComboSynth").expect("combo");
    let rect = layout.control(combo).rect();
    assert_eq!((rect.width, rect.height), (140, 30));

    // Creation order is drop-down button, edit field, then list, and each part is parent-relative.
    assert_eq!(
        layout
            .control(combo)
            .children()
            .iter()
            .map(|child| layout.control(*child).gadget_role())
            .collect::<Vec<_>>(),
        [
            Some(UiGadgetRole::ComboBoxDropDownButton),
            Some(UiGadgetRole::ComboBoxEditBox),
            Some(UiGadgetRole::ComboBoxListBox),
        ]
    );

    // The button is a fixed 21 pixels wide against the box's full height, and the field takes the
    // rest. Neither width scales with the layout.
    let button = part(&layout, combo, UiGadgetRole::ComboBoxDropDownButton);
    assert_eq!(
        layout.control(button).rect(),
        UiRect {
            x: 119,
            y: 0,
            width: 21,
            height: 30
        }
    );
    let field = part(&layout, combo, UiGadgetRole::ComboBoxEditBox);
    assert_eq!(
        layout.control(field).rect(),
        UiRect {
            x: 0,
            y: 0,
            width: 119,
            height: 30
        }
    );
    // The field is created with the literal "Entry" and the box's `MAXCHARS`, and a non-editable
    // combo box refuses input on it.
    assert_eq!(layout.control(field).text_label(), Some("Entry"));
    assert!(matches!(
        layout.control(field).kind(),
        UiControlKind::TextEntry { max_length: 8, .. }
    ));
    assert!(layout.control(field).status().contains(UiStatus::NO_INPUT));

    // The drop-down hangs below the closed box, starts hidden, and drops the box's `IMAGE` bit.
    let list = part(&layout, combo, UiGadgetRole::ComboBoxListBox);
    assert_eq!(
        layout.control(list).rect(),
        UiRect {
            x: 0,
            y: 30,
            width: 140,
            height: 30
        }
    );
    assert!(layout.control(list).is_hidden());
    assert!(!layout.control(list).status().contains(UiStatus::IMAGE));
    assert!(layout.control(list).status().contains(UiStatus::ABOVE));
}

#[test]
fn a_scroll_list_box_builds_a_scroll_bar_with_a_thumb() {
    let layout = instantiate(classic(800, 600));
    let list = layout.find("SynthMenu.wnd:ListSynth").expect("list");
    let rect = layout.control(list).rect();
    assert_eq!((rect.width, rect.height), (140, 130));

    let up = part(&layout, list, UiGadgetRole::ListBoxUpButton);
    let down = part(&layout, list, UiGadgetRole::ListBoxDownButton);
    let slider = part(&layout, list, UiGadgetRole::ListBoxSlider);
    // All three sit in the same 21-pixel column two pixels in from the right edge; the buttons are
    // 22 tall and the slider fills what is left between them.
    assert_eq!(
        layout.control(up).rect(),
        UiRect {
            x: 117,
            y: 2,
            width: 21,
            height: 22
        }
    );
    assert_eq!(
        layout.control(down).rect(),
        UiRect {
            x: 117,
            y: 106,
            width: 21,
            height: 22
        }
    );
    assert_eq!(
        layout.control(slider).rect(),
        UiRect {
            x: 117,
            y: 25,
            width: 21,
            height: 80
        }
    );

    // A vertical thumb is as wide as its slider and one pixel taller, and it is draggable.
    let thumb = part(&layout, slider, UiGadgetRole::SliderThumb);
    assert_eq!(
        layout.control(thumb).rect(),
        UiRect {
            x: 0,
            y: 0,
            width: 21,
            height: 22
        }
    );
    assert!(layout.control(thumb).status().contains(UiStatus::DRAGGABLE));
}

#[test]
fn a_horizontal_slider_thumb_uses_the_source_s_fixed_thumb_box() {
    let layout = instantiate(classic(800, 600));
    let slider = layout.find("SynthMenu.wnd:SliderSynth").expect("slider");
    let thumb = part(&layout, slider, UiGadgetRole::SliderThumb);
    // `HORIZONTAL_SLIDER_THUMB_WIDTH`/`_HEIGHT` are 13 by 16, and `_POSITION` is two thirds of the
    // height under integer division.
    assert_eq!(
        layout.control(thumb).rect(),
        UiRect {
            x: 0,
            y: 10,
            width: 13,
            height: 16
        }
    );
}

#[test]
fn a_gadget_part_draws_from_its_parent_s_records_for_that_part() {
    let layout = instantiate(classic(800, 600));
    let combo = layout.find("SynthMenu.wnd:ComboSynth").expect("combo");
    let button = part(&layout, combo, UiGadgetRole::ComboBoxDropDownButton);
    // `winCreateFromScript` copies each part's arrays out of the parent once the part exists, so
    // the button's own enabled slot is the combo box's drop-down-button enabled record.
    let owner = layout
        .control(combo)
        .draw_entry(WndDrawDataSlot::ComboBoxDropDownButtonEnabled);
    assert_eq!(
        layout.control(button).draw_entry(WndDrawDataSlot::Enabled),
        owner
    );
    assert!(owner.is_some());
}

#[test]
fn a_scroll_bar_thumb_reads_the_thumb_records_of_the_list_box_above_its_slider() {
    let layout = instantiate(classic(800, 600));
    let list = layout.find("SynthMenu.wnd:ListSynth").expect("list");
    let slider = part(&layout, list, UiGadgetRole::ListBoxSlider);
    let thumb = part(&layout, slider, UiGadgetRole::SliderThumb);

    // `SLIDERTHUMBENABLEDDRAWDATA` is written on the list box, two levels above the thumb, and the
    // slider between them declares nothing of its own. `winCreateFromScript` bridges that gap
    // through file statics; reading only the immediate parent would leave the thumb blank.
    let declared = layout
        .control(list)
        .draw_entry(WndDrawDataSlot::SliderThumbEnabled);
    assert_eq!(
        layout.control(thumb).draw_entry(WndDrawDataSlot::Enabled),
        declared
    );
    assert_eq!(
        declared.and_then(|entry| entry.image().map(str::to_owned)),
        Some("SynthThumb".to_owned())
    );

    // The up button reads the list box's own up-button record directly.
    let up = part(&layout, list, UiGadgetRole::ListBoxUpButton);
    assert_eq!(
        layout
            .control(up)
            .draw_entry(WndDrawDataSlot::Enabled)
            .and_then(|entry| entry.image().map(str::to_owned)),
        Some("SynthUpButton".to_owned())
    );
}

#[test]
fn tab_traversal_visits_declared_stops_and_wraps() {
    let mut layout = instantiate(classic(800, 600));
    let button = layout.find("SynthMenu.wnd:ButtonSynth").expect("button");
    let entry = layout.find("SynthMenu.wnd:EntrySynth").expect("entry");
    let check = layout.find("SynthMenu.wnd:CheckSynth").expect("check");
    // Three controls declare `TABSTOP`, but they are not the whole order: `gogoGadgetSlider` sets
    // the bit on every slider it creates and the thumb inherits it, so each slider in the layout —
    // the declared one, the list box's scroll bar, and the scroll bar inside the combo box's
    // drop-down list — contributes a stop and a thumb stop.
    let slider = layout.find("SynthMenu.wnd:SliderSynth").expect("slider");
    let list = layout.find("SynthMenu.wnd:ListSynth").expect("list");
    let list_slider = part(&layout, list, UiGadgetRole::ListBoxSlider);
    let combo = layout.find("SynthMenu.wnd:ComboSynth").expect("combo");
    let combo_list = part(&layout, combo, UiGadgetRole::ComboBoxListBox);
    let combo_slider = part(&layout, combo_list, UiGadgetRole::ListBoxSlider);
    assert_eq!(
        layout.tab_order(),
        [
            button,
            entry,
            check,
            slider,
            part(&layout, slider, UiGadgetRole::SliderThumb),
            list_slider,
            part(&layout, list_slider, UiGadgetRole::SliderThumb),
            combo_slider,
            part(&layout, combo_slider, UiGadgetRole::SliderThumb),
        ]
    );

    // The combo box's drop-down list starts hidden, so the two stops inside it stay in the tab list
    // but cannot hold focus until the list opens. The cycle visits the rest in order and wraps.
    let reachable: Vec<_> = layout
        .tab_order()
        .iter()
        .copied()
        .filter(|stop| layout.is_effectively_visible(*stop))
        .collect();
    assert_eq!(reachable.len(), layout.tab_order().len() - 2);
    assert!(!reachable.contains(&combo_slider));
    for stop in &reachable {
        layout.focus_next();
        assert_eq!(layout.focus(), Some(*stop));
    }
    layout.focus_next();
    assert_eq!(layout.focus(), Some(reachable[0]));
    layout.focus_previous();
    assert_eq!(layout.focus(), reachable.last().copied());

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
                images,
                ..
            } => Some((*control, *slot, images.image(*slot, 0).map(str::to_owned))),
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
        UiFrameItem::Quad { control, slot: WndDrawDataSlot::Hilite, images, .. }
            if *control == button
                && images.image(WndDrawDataSlot::Hilite, 0) == Some("SynthButtonHilite")
    )));

    // Disabling selects the disabled slot, which this control does not declare, so it draws
    // without an image rather than reusing the enabled visual.
    layout.pointer_moved(UiPoint::new(10, 10));
    layout.set_enabled(button, false);
    let frame = layout.frame(UiClipPolicy::None);
    assert!(frame.items().iter().any(|item| matches!(
        item,
        UiFrameItem::Quad { control, slot: WndDrawDataSlot::Disabled, images, color: None, .. }
            if *control == button && images.image(WndDrawDataSlot::Disabled, 0).is_none()
    )));
}

#[test]
fn only_a_mouse_tracking_control_hilites_under_the_pointer() {
    let mut layout = instantiate(classic(800, 600));
    let panel = layout.roots()[0];
    let button = layout.find("SynthMenu.wnd:ButtonSynth").expect("button");
    assert!(layout.control(button).is_mouse_track());
    // The enclosing panel is a plain `USER` window and declares no style at all.
    assert!(!layout.control(panel).is_mouse_track());

    // Over the button, which tracks: the hilite slot.
    layout.pointer_moved(UiPoint::new(150, 90));
    assert!(layout.control(button).is_hovered());
    assert_eq!(layout.state_slot(button), WndDrawDataSlot::Hilite);

    // Over the panel's own margin, which does not track: hovered, but still drawn enabled. Nothing
    // in the original sets `WIN_STATE_HILITED` for a window whose input handler does not, so a
    // pointer resting on a menu's background must not repaint the whole screen.
    layout.pointer_moved(UiPoint::new(110, 60));
    assert!(layout.control(panel).is_hovered());
    assert!(!layout.control(button).is_hovered());
    assert_eq!(layout.state_slot(panel), WndDrawDataSlot::Enabled);
    assert_eq!(layout.state_slot(button), WndDrawDataSlot::Enabled);

    // A press reports `selected` rather than moving the slot: the pressed art of a retail button
    // comes from the selected bit on top of the hover it already had.
    layout.pointer_moved(UiPoint::new(150, 90));
    layout.pointer_pressed(UiPoint::new(150, 90), UiMouseButton::Left);
    assert!(layout.control(button).is_pressed());
    assert_eq!(layout.state_slot(button), WndDrawDataSlot::Hilite);
    assert_eq!(layout.state_slot(panel), WndDrawDataSlot::Enabled);
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

#[test]
fn each_control_reports_the_family_whose_composition_rules_apply() {
    let layout = instantiate(classic(800, 600));
    let family = |name: &str| {
        layout
            .control(layout.find(name).expect("named control"))
            .family()
    };
    assert_eq!(
        family("SynthMenu.wnd:ButtonSynth"),
        UiControlFamily::PushButton
    );
    assert_eq!(
        family("SynthMenu.wnd:RadioFirst"),
        UiControlFamily::RadioButton
    );
    assert_eq!(
        family("SynthMenu.wnd:CheckSynth"),
        UiControlFamily::CheckBox
    );
    assert_eq!(
        family("SynthMenu.wnd:EntrySynth"),
        UiControlFamily::TextEntry
    );
    // Both slider orientations decode one `SLIDERDATA`, so the declared window type is what
    // separates them.
    assert_eq!(
        family("SynthMenu.wnd:SliderSynth"),
        UiControlFamily::HorizontalSlider
    );
    // A list box, a combo box, and the panel all draw one stretched image.
    assert_eq!(family("SynthMenu.wnd:ListSynth"), UiControlFamily::Simple);
    assert_eq!(family("SynthMenu.wnd:ComboSynth"), UiControlFamily::Simple);
}

#[test]
fn the_selected_bit_means_pressed_checked_or_chosen_by_family() {
    let mut layout = instantiate(classic(800, 600));
    let button = layout.find("SynthMenu.wnd:ButtonSynth").expect("button");
    let check = layout.find("SynthMenu.wnd:CheckSynth").expect("check box");
    let radio = layout
        .find("SynthMenu.wnd:RadioFirst")
        .expect("radio button");
    assert!(!layout.control(button).is_selected());
    assert!(!layout.control(check).is_selected());
    assert!(!layout.control(radio).is_selected());

    layout.pointer_pressed(UiPoint::new(150, 90), UiMouseButton::Left);
    assert!(layout.control(button).is_selected());
    layout.toggle_check(check);
    assert!(layout.control(check).is_selected());
    layout.select_radio(radio);
    assert!(layout.control(radio).is_selected());
}

#[test]
fn the_draw_callback_name_decides_the_image_path_before_the_status_bit() {
    let layout = instantiate(classic(800, 600));
    // The panel declares `IMAGE` and names no draw callback, so the status bit decides.
    let panel = layout.roots()[0];
    assert!(layout.control(panel).is_image_draw());
    // The button declares neither, so it draws colour only.
    let button = layout.find("SynthMenu.wnd:ButtonSynth").expect("button");
    assert!(!layout.control(button).is_image_draw());
    // A named procedure overrides the bit in both directions, and `"[None]"` is not a procedure.
    let colored = layout
        .find("SynthMenu.wnd:ColorSynth")
        .expect("static text");
    assert_eq!(
        layout.control(colored).draw_callback(),
        Some("GadgetStaticTextDraw")
    );
    assert!(!layout.control(colored).is_image_draw());
    let imaged = layout
        .find("SynthMenu.wnd:ImageSynth")
        .expect("static text");
    assert!(layout.control(imaged).is_image_draw());
    let none = layout.find("SynthMenu.wnd:NoneSynth").expect("static text");
    assert_eq!(layout.control(none).draw_callback(), Some("[None]"));
    assert!(layout.control(none).is_image_draw());
}

#[test]
fn a_check_box_indents_its_label_past_its_box_while_a_radio_button_centres() {
    let layout = instantiate(classic(800, 600));
    let align = |name: &str| {
        layout
            .control(layout.find(name).expect("named control"))
            .text_align()
    };
    // `drawCheckBoxText` centres only vertically and starts the label one control-height in.
    assert_eq!(
        align("SynthMenu.wnd:CheckSynth"),
        UiTextAlign::CenteredBesideBox
    );
    assert_eq!(align("SynthMenu.wnd:RadioFirst"), UiTextAlign::Centered);
    assert_eq!(align("SynthMenu.wnd:ButtonSynth"), UiTextAlign::Centered);
    assert_eq!(align("SynthMenu.wnd:EntrySynth"), UiTextAlign::TopLeft);
}

#[test]
fn a_tab_control_retains_its_declared_strip_and_moves_the_active_tab_with_the_pane() {
    let mut layout = instantiate(classic(800, 600));
    let tabs = layout.find("SynthMenu.wnd:TabsSynth").expect("tab control");
    let UiControlFamily::TabControl(geometry) = layout.control(tabs).family() else {
        panic!("tab control family");
    };
    assert_eq!(geometry.count, 2);
    assert_eq!((geometry.width, geometry.height), (40, 20));
    assert_eq!(geometry.pane_border, 4);
    assert_eq!(geometry.active, 0);

    assert!(layout.select_tab_pane(tabs, 1));
    let UiControlFamily::TabControl(moved) = layout.control(tabs).family() else {
        panic!("tab control family");
    };
    assert_eq!(moved.active, 1);
    // An index outside the declared panes is refused rather than clamped.
    assert!(!layout.select_tab_pane(tabs, 2));
}

#[test]
fn every_slot_travels_with_a_quad_so_a_composition_can_read_the_one_it_needs() {
    let layout = instantiate(classic(800, 600));
    let button = layout.find("SynthMenu.wnd:ButtonSynth").expect("button");
    let images = layout.control(button).slot_images();
    assert_eq!(
        images.image(WndDrawDataSlot::Enabled, 0),
        Some("SynthButtonEnabled")
    );
    assert_eq!(
        images.image(WndDrawDataSlot::Hilite, 0),
        Some("SynthButtonHilite")
    );
    assert_eq!(images.image(WndDrawDataSlot::Disabled, 0), None);
    // An index past the record's own entries is absent rather than out of range.
    assert_eq!(images.image(WndDrawDataSlot::Enabled, 8), None);
    assert!(!images.is_empty());
    assert_eq!(
        images.names().collect::<Vec<_>>(),
        ["SynthButtonEnabled", "SynthButtonHilite"]
    );
}

/// A synthetic layout with two overlapping top-level windows and two overlapping children, for
/// pinning the order the original's window manager searches.
fn overlapping_layout() -> String {
    r#"FILE_VERSION = 2;
STARTLAYOUTBLOCK
  LAYOUTINIT = MainMenuInit;
  LAYOUTUPDATE = "SynthMenuUpdate";
  LAYOUTSHUTDOWN = [NONE];
ENDLAYOUTBLOCK
WINDOW
  WINDOWTYPE = USER;
  SCREENRECT = UPPERLEFT: 0 0,
               BOTTOMRIGHT: 400 400,
               CREATIONRESOLUTION: 800 600;
  NAME = "SynthOverlap.wnd:RootFirst";
  STATUS = ENABLED;
CHILD
WINDOW
  WINDOWTYPE = PUSHBUTTON;
  SCREENRECT = UPPERLEFT: 10 10,
               BOTTOMRIGHT: 200 200,
               CREATIONRESOLUTION: 800 600;
  NAME = "SynthOverlap.wnd:ChildFirst";
  STATUS = ENABLED;
END
CHILD
WINDOW
  WINDOWTYPE = PUSHBUTTON;
  SCREENRECT = UPPERLEFT: 10 10,
               BOTTOMRIGHT: 200 200,
               CREATIONRESOLUTION: 800 600;
  NAME = "SynthOverlap.wnd:ChildSecond";
  STATUS = ENABLED;
END
ENDALLCHILDREN
END
WINDOW
  WINDOWTYPE = USER;
  SCREENRECT = UPPERLEFT: 0 0,
               BOTTOMRIGHT: 400 400,
               CREATIONRESOLUTION: 800 600;
  NAME = "SynthOverlap.wnd:RootSecond";
  STATUS = ENABLED;
END
"#
    .to_owned()
}

/// Instantiates one synthetic source at 800x600 under the classic policy.
fn instantiate_source(source: &str) -> UiLayout {
    let document = parse_wnd(source.as_bytes(), WndLimits::default()).expect("decode layout");
    assert!(
        document.diagnostics().is_empty(),
        "synthetic fixture should decode cleanly: {:?}",
        document.diagnostics()
    );
    UiLayout::instantiate(&document, classic(800, 600), UiLimits::default())
        .expect("instantiate layout")
}

#[test]
fn the_layered_search_runs_front_to_back_which_is_reverse_file_order() {
    let layout = instantiate_source(&overlapping_layout());
    let root_second = layout
        .find("SynthOverlap.wnd:RootSecond")
        .expect("second root");
    let child_second = layout
        .find("SynthOverlap.wnd:ChildSecond")
        .expect("second child");

    // Both roots cover the point. `winCreate` links each new window at the head of the manager's
    // list, and `getWindowUnderCursor` walks from the head, so the last window in the file is tested
    // first — and it is also the one drawn on top.
    assert_eq!(layout.hit_test(UiPoint::new(50, 50)), Some(root_second));

    // The same holds for children, whose list `addWindowToParent` also prepends to.
    let mut layout = layout;
    layout.set_hidden(root_second, true);
    assert_eq!(layout.hit_test(UiPoint::new(50, 50)), Some(child_second));
    layout.set_hidden(child_second, true);
    let child_first = layout
        .find("SynthOverlap.wnd:ChildFirst")
        .expect("first child");
    assert_eq!(layout.hit_test(UiPoint::new(50, 50)), Some(child_first));
}

#[test]
fn a_layout_retains_its_own_init_update_and_shutdown_names() {
    let layout = instantiate_source(&overlapping_layout());
    assert_eq!(layout.layout_init_callback(), Some("MainMenuInit"));
    // `[NONE]` is retained verbatim rather than turned into an absent name; the classifier decides
    // what it means.
    assert_eq!(layout.layout_shutdown_callback(), Some("[NONE]"));
    assert!(is_none_callback("[NONE]"));
    assert!(is_none_callback("[None]"));
    assert!(!is_none_callback("None"));

    // A layout callback keeps its quotes when the file writes them, because the source reads it with
    // `strtok(buffer, " =")` and never strips a quote — unlike a window callback, which
    // `parseSystemCallback` and its siblings read by scanning to the first quote and taking what is
    // inside. Retail writes layout callbacks bare, so they resolve; a quoted one is a name no table
    // carries, in the original as much as here.
    assert_eq!(layout.layout_update_callback(), Some("\"SynthMenuUpdate\""));
    assert_eq!(
        classify_callback(UiCallbackSlot::LayoutUpdate, "\"MainMenuUpdate\""),
        UiCallbackBinding::Unknown
    );
    assert_eq!(
        classify_callback(UiCallbackSlot::LayoutUpdate, "MainMenuUpdate"),
        UiCallbackBinding::Established {
            table: UiCallbackTable::LayoutUpdate
        }
    );
}

#[test]
fn a_control_retains_every_callback_slot_as_data() {
    let layout = instantiate(classic(800, 600));
    let button = layout.find("SynthMenu.wnd:ButtonSynth").expect("button");
    let control = layout.control(button);
    assert_eq!(control.system_callback(), Some("SynthButtonSystem"));
    assert_eq!(control.input_callback(), None);
    assert_eq!(control.tooltip_callback(), None);
    assert_eq!(control.draw_callback(), None);
}

#[test]
fn a_callback_name_resolves_only_in_the_table_its_record_searches() {
    // A pinned slot finds its own table's names and nothing else.
    assert_eq!(
        classify_callback(UiCallbackSlot::System, "MainMenuSystem"),
        UiCallbackBinding::Established {
            table: UiCallbackTable::System
        }
    );
    assert_eq!(
        classify_callback(UiCallbackSlot::System, "MainMenuInput"),
        UiCallbackBinding::Unknown
    );
    // Case matters: `nameToKey` compares with `strcmp`.
    assert_eq!(
        classify_callback(UiCallbackSlot::System, "mainmenusystem"),
        UiCallbackBinding::Unknown
    );
    // The two slots that default to `TABLE_ANY` reach the device tables, which is the only way a
    // gadget draw procedure or the device main-menu initializer resolves at all.
    assert_eq!(
        classify_callback(UiCallbackSlot::Draw, "W3DGadgetPushButtonImageDraw"),
        UiCallbackBinding::Established {
            table: UiCallbackTable::DeviceDraw
        }
    );
    assert_eq!(
        classify_callback(UiCallbackSlot::LayoutInit, "W3DMainMenuInit"),
        UiCallbackBinding::Established {
            table: UiCallbackTable::LayoutDeviceInit
        }
    );
    // An every-table search walks tables in `TableIndex` order, so an earlier table wins.
    assert_eq!(
        classify_callback(UiCallbackSlot::Draw, "MainMenuSystem"),
        UiCallbackBinding::Established {
            table: UiCallbackTable::System
        }
    );
    // The placeholder and an unknown name are both inert, and distinguishable.
    assert_eq!(
        classify_callback(UiCallbackSlot::System, "[None]"),
        UiCallbackBinding::None
    );
    let unknown = classify_callback(UiCallbackSlot::System, "ModdedMenuSystem");
    assert_eq!(unknown, UiCallbackBinding::Unknown);
    assert!(unknown.is_inert());
    assert!(UiCallbackBinding::None.is_inert());
    assert!(
        !UiCallbackBinding::Established {
            table: UiCallbackTable::System
        }
        .is_inert()
    );
}

#[test]
fn only_allowlisted_controls_route_a_typed_action() {
    let mut allowlist = UiActionAllowlist::new();
    assert!(allowlist.is_empty());
    allowlist.allow(
        "MainMenu.wnd:ButtonOptions",
        UiDemoAction::PushScreen {
            path: "Menus/OptionsMenu.wnd".to_owned(),
        },
    );
    allowlist.allow("ButtonBack", UiDemoAction::PopScreen);

    // A decorated key resolves by its exact spelling.
    assert_eq!(
        allowlist.resolve("MainMenu.wnd:ButtonOptions"),
        Some(
            [UiDemoAction::PushScreen {
                path: "Menus/OptionsMenu.wnd".to_owned()
            }]
            .as_slice()
        )
    );
    // An undecorated key resolves for any layout, which is how one verb covers every menu's Back.
    assert_eq!(
        allowlist.resolve("OptionsMenu.wnd:ButtonBack"),
        Some([UiDemoAction::PopScreen].as_slice())
    );
    // Anything not listed routes nothing, however established its callback name is.
    assert_eq!(allowlist.resolve("MainMenu.wnd:ButtonExit"), None);
    assert_eq!(allowlist.resolve("ButtonOptions"), None);
    assert_eq!(allowlist.len(), 2);
    assert_eq!(
        allowlist
            .entries()
            .map(|(name, _)| name)
            .collect::<Vec<_>>(),
        ["ButtonBack", "MainMenu.wnd:ButtonOptions"]
    );
}

#[test]
fn one_control_may_run_an_ordered_sequence_of_actions() {
    // `MainMenuSystem`'s single-player arm reveals a subpanel and then touches three transition
    // groups, so the binding has to keep four actions in the order the source runs them.
    let mut allowlist = UiActionAllowlist::new();
    allowlist.allow_all(
        "MainMenu.wnd:ButtonSinglePlayer",
        vec![
            UiDemoAction::ShowControl {
                control: "MainMenu.wnd:MapBorder".to_owned(),
            },
            UiDemoAction::RemoveTransitionGroup {
                group: "MainMenuDefaultMenu".to_owned(),
                skip_pending: false,
            },
            UiDemoAction::ReverseTransitionGroup {
                group: "MainMenuDefaultMenuBack".to_owned(),
            },
            UiDemoAction::SetTransitionGroup {
                group: "MainMenuSinglePlayerMenu".to_owned(),
                immediate: false,
            },
        ],
    );

    let routed = allowlist
        .resolve("MainMenu.wnd:ButtonSinglePlayer")
        .expect("the single-player binding");
    assert_eq!(routed.len(), 4);
    assert_eq!(
        routed
            .iter()
            .map(UiDemoAction::row_name)
            .collect::<Vec<_>>(),
        [
            "show_control",
            "remove_transition_group",
            "reverse_transition_group",
            "set_transition_group"
        ]
    );
    // The operands survive into the report field, which is what a capture's log is read from.
    assert_eq!(routed[0].row_detail(), "MainMenu.wnd:MapBorder");
    assert_eq!(
        routed[3].row_detail(),
        "MainMenuSinglePlayerMenu immediate=false"
    );
    // One control still holds one binding: allowing it again replaces the sequence rather than
    // appending to it.
    allowlist.allow("MainMenu.wnd:ButtonSinglePlayer", UiDemoAction::PopScreen);
    assert_eq!(
        allowlist.resolve("MainMenu.wnd:ButtonSinglePlayer"),
        Some([UiDemoAction::PopScreen].as_slice())
    );
    assert_eq!(allowlist.len(), 1);
}

/// Builds a shell screen from the overlapping fixture under a chosen path.
fn screen(path: &str) -> UiScreen {
    UiScreen::new(path, instantiate_source(&overlapping_layout()))
}

#[test]
fn a_composed_frame_draws_the_screens_in_draw_order_not_stack_order() {
    let mut shell = UiShell::new();
    shell
        .push(screen("Menus/MainMenu.wnd"), false)
        .expect("push the first screen");
    shell
        .push(screen("Menus/OptionsMenu.wnd"), false)
        .expect("push over it");
    shell.shutdown_complete();
    assert_eq!(shell.screen_count(), 2);

    // Each screen contributes its own frame, and the composed frame is exactly their concatenation.
    let frames = shell.frames(UiClipPolicy::None);
    assert_eq!(frames.len(), 2);
    let composed = shell.frame(UiClipPolicy::None);
    assert_eq!(
        composed.len(),
        frames.iter().map(|(_, frame)| frame.len()).sum::<usize>()
    );

    // The pushed screen was brought forward, so its items land last and therefore draw on top.
    let (front, front_frame) = frames.last().expect("a front screen");
    assert_eq!(front.index(), 1);
    assert_eq!(
        composed.items()[composed.len() - front_frame.len()..],
        *front_frame.items()
    );

    // Bringing the bottom screen forward reorders the composition without touching the stack.
    let bottom_id = shell
        .find_screen_by_path("Menus/MainMenu.wnd")
        .expect("the bottom screen");
    shell.bring_forward(bottom_id);
    assert_eq!(shell.top().expect("a top").path(), "Menus/OptionsMenu.wnd");
    assert_eq!(
        shell
            .frames(UiClipPolicy::None)
            .iter()
            .map(|(screen, _)| screen.index())
            .collect::<Vec<_>>(),
        [1, 0]
    );

    // A hidden screen draws nothing, so the composition shrinks to the visible screen alone.
    shell
        .layout_mut(bottom_id)
        .expect("the bottom layout")
        .hide(true);
    assert_eq!(
        shell.frame(UiClipPolicy::None).len(),
        front_frame.len(),
        "a hidden screen must contribute no items"
    );
}

#[test]
fn a_push_shuts_the_current_top_down_before_linking_the_new_screen() {
    let mut shell = UiShell::new();

    // With an empty stack there is nothing to shut down, so the push completes in one call.
    let events = shell
        .push(screen("Menus/MainMenu.wnd"), false)
        .expect("push onto an empty stack");
    assert_eq!(shell.screen_count(), 1);
    assert!(!shell.is_operation_pending());
    let first = UiShellEvent::ScreenPushed {
        screen: shell.top_id().expect("a top"),
        path: "Menus/MainMenu.wnd".to_owned(),
    };
    assert_eq!(events[0], first);
    assert!(matches!(
        events[1],
        UiShellEvent::LayoutInit {
            binding: Some(UiCallbackBinding::Established { .. }),
            ..
        }
    ));
    assert!(matches!(events[2], UiShellEvent::BroughtForward { .. }));

    // With a visible top the push waits: the top's shutdown runs and nothing is linked yet.
    let events = shell
        .push(screen("Menus/OptionsMenu.wnd"), false)
        .expect("push over a visible top");
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0],
        UiShellEvent::LayoutShutdown {
            immediate: false,
            ..
        }
    ));
    assert_eq!(shell.screen_count(), 1);
    assert!(shell.is_operation_pending());

    // The layout reports its shutdown finished, and only then is the screen linked.
    let events = shell.shutdown_complete();
    assert_eq!(shell.screen_count(), 2);
    assert!(!shell.is_operation_pending());
    assert_eq!(shell.top().expect("a top").path(), "Menus/OptionsMenu.wnd");
    assert!(matches!(events[0], UiShellEvent::ScreenPushed { .. }));

    // The pushed screen was brought to the front of the draw order.
    assert_eq!(
        shell.draw_order().last().copied(),
        Some(shell.top_id().expect("a top"))
    );
}

#[test]
fn a_hidden_top_short_circuits_the_shutdown_the_way_the_source_does() {
    let mut shell = UiShell::new();
    shell
        .push(screen("Menus/MainMenu.wnd"), false)
        .expect("first push");
    shell.top_mut().expect("a top").layout_mut().hide(true);

    // `Shell::push` only runs a shutdown when the top is visible, so this pushes immediately.
    let events = shell
        .push(screen("Menus/OptionsMenu.wnd"), false)
        .expect("push over a hidden top");
    assert_eq!(shell.screen_count(), 2);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, UiShellEvent::LayoutShutdown { .. }))
    );
}

#[test]
fn a_pop_waits_for_shutdown_and_an_immediate_pop_does_not() {
    let mut shell = UiShell::new();
    shell
        .push(screen("Menus/MainMenu.wnd"), false)
        .expect("first push");
    shell
        .push(screen("Menus/OptionsMenu.wnd"), false)
        .expect("second push");
    shell.shutdown_complete();
    assert_eq!(shell.screen_count(), 2);

    let events = shell.pop().expect("pop the top");
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0],
        UiShellEvent::LayoutShutdown {
            immediate: false,
            ..
        }
    ));
    assert_eq!(shell.screen_count(), 2);

    let events = shell.shutdown_complete();
    assert_eq!(shell.screen_count(), 1);
    assert_eq!(
        events[0],
        UiShellEvent::ScreenPopped {
            path: "Menus/OptionsMenu.wnd".to_owned()
        }
    );
    // The screen underneath is initialized again, as though it had just been pushed.
    assert!(matches!(events[1], UiShellEvent::LayoutInit { .. }));

    // An immediate pop tells the shutdown it is about to go and unlinks in the same call.
    shell
        .push(screen("Menus/SkirmishGameOptionsMenu.wnd"), false)
        .expect("third push");
    shell.shutdown_complete();
    let events = shell.pop_immediate().expect("immediate pop");
    assert_eq!(shell.screen_count(), 1);
    assert!(matches!(
        events[0],
        UiShellEvent::LayoutShutdown {
            immediate: true,
            ..
        }
    ));
    assert!(!shell.is_operation_pending());
}

#[test]
fn a_pop_that_only_makes_room_for_a_push_skips_the_new_tops_init() {
    let mut shell = UiShell::new();
    shell
        .push(screen("Menus/MainMenu.wnd"), false)
        .expect("first push");
    shell
        .push(screen("Menus/OptionsMenu.wnd"), false)
        .expect("second push");
    shell.shutdown_complete();

    shell.pop().expect("pop the top");
    let events = shell.shutdown_complete_with(true);
    assert_eq!(shell.screen_count(), 1);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, UiShellEvent::LayoutInit { .. }))
    );
}

#[test]
fn the_shell_refuses_an_empty_path_a_full_stack_and_an_overlapping_operation() {
    let mut shell = UiShell::new();
    assert_eq!(
        shell.push(screen(""), false).err(),
        Some(UiShellError::EmptyPath)
    );

    for index in 0..UI_MAX_SHELL_STACK {
        shell
            .push(screen(&format!("Menus/Synth{index}.wnd")), false)
            .expect("push within the bound");
        // Every push but the first waits on the previous top's shutdown.
        shell.shutdown_complete();
    }
    assert_eq!(shell.screen_count(), UI_MAX_SHELL_STACK);
    assert_eq!(
        shell.push(screen("Menus/SynthOver.wnd"), false).err(),
        Some(UiShellError::StackFull {
            path: "Menus/SynthOver.wnd".to_owned().into_boxed_str(),
            limit: UI_MAX_SHELL_STACK,
        })
    );

    // A second operation while one is pending is refused rather than losing the first.
    shell.pop().expect("pop the top");
    assert_eq!(shell.pop().err(), Some(UiShellError::OperationPending));
    assert_eq!(
        shell.push(screen("Menus/Synth0.wnd"), false).err(),
        Some(UiShellError::OperationPending)
    );
}

#[test]
fn hiding_the_shell_hides_every_screen_and_updates_run_from_the_top_down() {
    let mut shell = UiShell::new();
    shell
        .push(screen("Menus/MainMenu.wnd"), false)
        .expect("first push");
    shell
        .push(screen("Menus/OptionsMenu.wnd"), false)
        .expect("second push");
    shell.shutdown_complete();

    let events = shell.hide(true);
    assert_eq!(
        events,
        vec![UiShellEvent::VisibilityChanged { hidden: true }]
    );
    assert!(shell.is_hidden());
    for entry in shell.screens() {
        assert!(entry.layout().is_hidden());
        for root in entry.layout().roots() {
            assert!(entry.layout().control(*root).is_hidden());
        }
    }
    shell.hide(false);
    assert!(!shell.is_hidden());
    assert!(!shell.top().expect("a top").layout().is_hidden());

    // `Shell::update` runs every screen's update starting at the top index and counting down.
    let updates = shell.update();
    let order: Vec<usize> = updates
        .iter()
        .map(|event| match event {
            UiShellEvent::LayoutUpdate { screen, .. } => screen.index(),
            other => panic!("expected an update event, got {other:?}"),
        })
        .collect();
    assert_eq!(order, vec![1, 0]);
}

#[test]
fn the_shell_searches_every_screen_front_to_back_in_one_layered_pass() {
    let mut shell = UiShell::new();
    shell
        .push(screen("Menus/MainMenu.wnd"), false)
        .expect("first push");
    shell
        .push(screen("Menus/OptionsMenu.wnd"), false)
        .expect("second push");
    shell.shutdown_complete();
    let bottom = shell
        .find_screen_by_path("Menus/MainMenu.wnd")
        .expect("bottom screen");
    let top = shell.top_id().expect("a top");

    // Both screens cover the point; the front of the draw order answers.
    let (screen_id, _) = shell.hit_test(UiPoint::new(50, 50)).expect("a hit");
    assert_eq!(screen_id, top);

    // Bringing the bottom screen forward changes only the draw order, not the stack.
    shell.bring_forward(bottom);
    assert_eq!(shell.top_id(), Some(top));
    let (screen_id, _) = shell.hit_test(UiPoint::new(50, 50)).expect("a hit");
    assert_eq!(screen_id, bottom);

    // A hidden screen takes no input, so the search falls through to the one behind it.
    shell
        .screen_mut(bottom)
        .expect("bottom screen")
        .layout_mut()
        .hide(true);
    let (screen_id, _) = shell.hit_test(UiPoint::new(50, 50)).expect("a hit");
    assert_eq!(screen_id, top);

    // Outside every screen nothing is hit.
    assert!(shell.hit_test(UiPoint::new(700, 500)).is_none());
}

#[test]
fn showing_and_hiding_the_shell_run_the_tops_callbacks_without_moving_the_stack() {
    let mut shell = UiShell::new();
    assert!(shell.show_shell(true).is_empty());
    assert!(shell.hide_shell().is_empty());
    shell
        .push(screen("Menus/MainMenu.wnd"), false)
        .expect("first push");

    let events = shell.show_shell(true);
    assert!(matches!(events[0], UiShellEvent::LayoutInit { .. }));
    assert_eq!(shell.screen_count(), 1);
    assert!(shell.show_shell(false).is_empty());

    // `Shell::hideShell` passes an immediate pop even though it pops nothing.
    let events = shell.hide_shell();
    assert!(matches!(
        events[0],
        UiShellEvent::LayoutShutdown {
            immediate: true,
            ..
        }
    ));
    assert_eq!(shell.screen_count(), 1);
}

#[test]
fn a_screen_is_found_by_its_path_case_insensitively() {
    let mut shell = UiShell::new();
    shell
        .push(screen("Menus/MainMenu.wnd"), false)
        .expect("first push");
    let top = shell.top_id().expect("a top");
    assert_eq!(shell.find_screen_by_path("Menus/MainMenu.wnd"), Some(top));
    assert_eq!(shell.find_screen_by_path("menus/mainmenu.wnd"), Some(top));
    assert_eq!(shell.find_screen_by_path("Menus/OptionsMenu.wnd"), None);
    assert_eq!(shell.find_screen_by_path(""), None);
}

/// A synthetic layout carrying one window per transition style under test, plus the companion windows
/// the main-menu styles pair with.
fn transition_layout() -> String {
    let panel = draw_data("SynthPanel");
    format!(
        r#"FILE_VERSION = 2;
STARTLAYOUTBLOCK
  LAYOUTINIT = [NONE];
  LAYOUTUPDATE = [NONE];
  LAYOUTSHUTDOWN = [NONE];
ENDLAYOUTBLOCK
WINDOW
  WINDOWTYPE = USER;
  SCREENRECT = UPPERLEFT: 100 100,
               BOTTOMRIGHT: 200 140,
               CREATIONRESOLUTION: 800 600;
  NAME = "SynthTrans.wnd:Fader";
  STATUS = ENABLED+IMAGE;
  ENABLEDDRAWDATA = {panel};
END
WINDOW
  WINDOWTYPE = PUSHBUTTON;
  SCREENRECT = UPPERLEFT: 300 100,
               BOTTOMRIGHT: 400 140,
               CREATIONRESOLUTION: 800 600;
  NAME = "SynthTrans.wnd:Button";
  STATUS = ENABLED+IMAGE;
  ENABLEDDRAWDATA = {panel};
END
WINDOW
  WINDOWTYPE = USER;
  SCREENRECT = UPPERLEFT: 100 200,
               BOTTOMRIGHT: 160 260,
               CREATIONRESOLUTION: 800 600;
  NAME = "SynthTrans.wnd:Logo";
  STATUS = ENABLED+IMAGE;
  ENABLEDDRAWDATA = {panel};
END
WINDOW
  WINDOWTYPE = USER;
  SCREENRECT = UPPERLEFT: 100 200,
               BOTTOMRIGHT: 220 320,
               CREATIONRESOLUTION: 800 600;
  NAME = "SynthTrans.wnd:LogoMedium";
  STATUS = ENABLED;
END
WINDOW
  WINDOWTYPE = STATICTEXT;
  SCREENRECT = UPPERLEFT: 400 200,
               BOTTOMRIGHT: 500 230,
               CREATIONRESOLUTION: 800 600;
  NAME = "SynthTrans.wnd:Label";
  STATUS = ENABLED;
  TEXT = "Synth";
END
WINDOW
  WINDOWTYPE = STATICTEXT;
  SCREENRECT = UPPERLEFT: 400 250,
               BOTTOMRIGHT: 500 280,
               CREATIONRESOLUTION: 800 600;
  NAME = "SynthTrans.wnd:Score";
  STATUS = ENABLED;
  TEXT = "4";
END
"#
    )
}

/// A synthetic transition file exercising the styles the tests below assert on.
fn transition_definitions() -> &'static [u8] {
    br"WindowTransition SynthIn
  Window
    WinName = SynthTrans.wnd:Fader
    Style   = WINFADE
    FrameDelay = 0
  END
  Window
    WinName = SynthTrans.wnd:Button
    Style   = BUTTONFLASH
    FrameDelay = 2
  END
END
WindowTransition SynthText
  Window
    WinName = SynthTrans.wnd:Label
    Style   = TYPETEXT
    FrameDelay = 0
  END
  Window
    WinName = SynthTrans.wnd:Score
    Style   = COUNTUP
    FrameDelay = 0
  END
END
WindowTransition SynthGrow
  Window
    WinName = SynthTrans.wnd:Logo
    Style   = MAINMENUMEDIUMSCALEUP
    FrameDelay = 0
  END
END
WindowTransition SynthMissing
  Window
    WinName = SynthTrans.wnd:Absent
    Style   = WINFADE
    FrameDelay = 0
  END
END
WindowTransition SynthOnce
  Window
    WinName = SynthTrans.wnd:Fader
    Style   = WINFADE
    FrameDelay = 0
  END
  FireOnce = Yes
END
"
}

/// Builds a one-screen shell over the transition fixture, plus a handler over its definitions.
fn transition_fixture() -> (UiShell, UiTransitionHandler) {
    let mut shell = UiShell::new();
    shell
        .push(
            UiScreen::new(
                "Menus/SynthTrans.wnd",
                instantiate_source(&transition_layout()),
            ),
            false,
        )
        .expect("push the transition fixture");
    let definitions =
        parse_window_transitions_ini(transition_definitions(), UiIniLimits::default())
            .expect("decode the synthetic transition file");
    let handler = UiTransitionHandler::new(&definitions);
    (shell, handler)
}

/// Returns whether a control is currently hidden.
fn hidden(shell: &UiShell, name: &str) -> bool {
    let (screen, control) = shell
        .find_control_by_decorated_name(name)
        .expect("control is in the fixture");
    shell
        .layout(screen)
        .expect("screen")
        .control(control)
        .is_hidden()
}

/// Returns a control's displayed text.
fn label(shell: &UiShell, name: &str) -> String {
    let (screen, control) = shell
        .find_control_by_decorated_name(name)
        .expect("control is in the fixture");
    shell
        .layout(screen)
        .expect("screen")
        .control(control)
        .displayed_text()
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn a_group_hides_its_windows_at_the_start_and_shows_them_at_the_end() {
    let (mut shell, mut handler) = transition_fixture();
    handler.set_group(&mut shell, "SynthIn", false);

    // `init` runs the start frame backwards, which hides every window the group animates.
    assert!(hidden(&shell, "SynthTrans.wnd:Fader"));
    assert!(hidden(&shell, "SynthTrans.wnd:Button"));
    assert!(!handler.is_finished());
    assert_eq!(handler.current_group(), Some("SynthIn"));

    // The fade covers frames 1..9 and shows its window on 10; the button flash starts two frames
    // later and runs 17, so the group finishes on 19.
    assert_eq!(handler.group_total_frames("SynthIn"), 19);
    for _ in 0..19 {
        handler.update(&mut shell, 1.0);
    }
    assert!(!hidden(&shell, "SynthTrans.wnd:Fader"));
    assert!(!hidden(&shell, "SynthTrans.wnd:Button"));
    assert!(handler.is_finished());
}

#[test]
fn a_windows_frame_delay_shifts_its_own_frames_inside_the_group() {
    let (mut shell, mut handler) = transition_fixture();
    handler.set_group(&mut shell, "SynthIn", false);

    // The handler assigns its draw group before advancing, so one update already draws frame 1: the
    // fade is one frame in, and the button flash, delayed two frames, has drawn nothing yet.
    handler.update(&mut shell, 1.0);
    let draws = handler.draws();
    assert!(draws.iter().any(|draw| matches!(
        draw,
        UiTransitionDraw::ControlImage { color, .. } if color.channels() == [255, 255, 255, 25]
    )));
    assert!(
        !draws
            .iter()
            .any(|draw| matches!(draw, UiTransitionDraw::Rect { .. }))
    );

    // Two frames later the button flash reaches its own first frame, which washes the button white.
    handler.update(&mut shell, 1.0);
    handler.update(&mut shell, 1.0);
    let draws = handler.draws();
    assert!(draws.iter().any(|draw| matches!(
        draw,
        UiTransitionDraw::Rect { fill: Some(fill), .. } if fill.channels() == [255, 255, 255, 75]
    )));
}

#[test]
fn one_update_runs_every_whole_frame_the_accumulator_crossed() {
    let (mut shell, mut handler) = transition_fixture();
    handler.set_group(&mut shell, "SynthIn", false);

    // A scale below one frame accumulates without stepping, exactly as the source's `Real` frame
    // counter does.
    let step = handler.update(&mut shell, 0.4);
    assert!(step.frames().is_empty());
    let step = handler.update(&mut shell, 0.4);
    assert!(step.frames().is_empty());
    let step = handler.update(&mut shell, 0.4);
    assert_eq!(step.frames(), [1]);

    // A scale above one frame steps every frame it passed, so no state is skipped.
    let step = handler.update(&mut shell, 3.0);
    assert_eq!(step.frames(), [2, 3, 4]);
}

#[test]
fn a_type_text_reveals_one_character_per_frame_and_a_count_up_rewrites_its_label() {
    let (mut shell, mut handler) = transition_fixture();
    handler.set_group(&mut shell, "SynthText", false);

    // "Synth" is five characters, so the type-text runs five frames rather than the style's declared
    // thirty, and the group's total shortens with it — `getTotalFrames` reads the armed length.
    assert_eq!(handler.group_total_frames("SynthText"), 5);
    handler.update(&mut shell, 1.0);
    assert!(handler.draws().iter().any(|draw| matches!(
        draw,
        UiTransitionDraw::TypedText { text, .. } if text == "S"
    )));
    handler.update(&mut shell, 1.0);
    handler.update(&mut shell, 1.0);
    assert!(handler.draws().iter().any(|draw| matches!(
        draw,
        UiTransitionDraw::TypedText { text, .. } if text == "Syn"
    )));

    // The count-up steps toward its authored value and lands exactly on the authored text.
    assert_eq!(label(&shell, "SynthTrans.wnd:Score"), "3");
    for _ in 0..6 {
        handler.update(&mut shell, 1.0);
    }
    assert_eq!(label(&shell, "SynthTrans.wnd:Score"), "4");
    // A count-up never hides its window: it rewrites the text in place.
    assert!(!hidden(&shell, "SynthTrans.wnd:Score"));
}

#[test]
fn a_scale_style_pairs_with_a_companion_window_and_hands_over_to_it() {
    let (mut shell, mut handler) = transition_fixture();
    handler.set_group(&mut shell, "SynthGrow", false);

    // The companion is the animated window's own name with `Medium` appended.
    let targets = handler.group_targets("SynthGrow");
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].1, TransitionStyle::MainMenuMediumScaleUp);
    assert!(targets[0].2.is_some());

    // Mid-flight both windows are hidden and the stand-in grows from the smaller rect toward the
    // larger one.
    handler.update(&mut shell, 1.0);
    assert!(hidden(&shell, "SynthTrans.wnd:Logo"));
    assert!(hidden(&shell, "SynthTrans.wnd:LogoMedium"));
    let draws = handler.draws();
    let UiTransitionDraw::ControlImage { rect, .. } = &draws[0] else {
        panic!("expected the grown stand-in, got {draws:?}");
    };
    // The 60x60 logo grows toward the 120x120 companion over three frames, so one frame in it is
    // twenty pixels wider and still centred on the same point.
    assert_eq!((rect.width, rect.height), (80, 80));
    assert_eq!((rect.x, rect.y), (90, 190));

    // At the end the animated window is hidden and the companion takes over.
    handler.update(&mut shell, 1.0);
    handler.update(&mut shell, 1.0);
    assert!(hidden(&shell, "SynthTrans.wnd:Logo"));
    assert!(!hidden(&shell, "SynthTrans.wnd:LogoMedium"));
}

#[test]
fn a_window_or_companion_that_does_not_resolve_is_reported_and_draws_nothing() {
    let (mut shell, mut handler) = transition_fixture();
    let step = handler.set_group(&mut shell, "SynthMissing", false);
    assert!(matches!(
        step.diagnostics()[0].kind(),
        UiTransitionDiagnosticKind::WindowNotFound { name } if &**name == "SynthTrans.wnd:Absent"
    ));
    assert_eq!(handler.group_targets("SynthMissing")[0].2, None);
    for _ in 0..12 {
        handler.update(&mut shell, 1.0);
    }
    assert!(handler.draws().is_empty());
}

#[test]
fn setting_a_second_group_reverses_the_first_and_queues_the_second() {
    let (mut shell, mut handler) = transition_fixture();
    handler.set_group(&mut shell, "SynthIn", false);
    for _ in 0..20 {
        handler.update(&mut shell, 1.0);
    }
    assert!(handler.is_finished());
    assert!(!hidden(&shell, "SynthTrans.wnd:Fader"));

    // A group that is not fire-once reverses instead of being dropped, and the next group waits.
    handler.set_group(&mut shell, "SynthGrow", false);
    assert_eq!(handler.current_group(), Some("SynthIn"));
    assert_eq!(handler.pending_group(), Some("SynthGrow"));
    assert!(handler.is_reversed());

    // Running it out backwards hides the window again and then the queued group takes over.
    for _ in 0..25 {
        handler.update(&mut shell, 1.0);
    }
    assert!(hidden(&shell, "SynthTrans.wnd:Fader"));
    assert_eq!(handler.current_group(), Some("SynthGrow"));
    assert_eq!(handler.pending_group(), None);
}

#[test]
fn a_fire_once_group_clears_itself_and_an_immediate_set_skips_what_was_running() {
    let (mut shell, mut handler) = transition_fixture();
    handler.set_group(&mut shell, "SynthOnce", false);
    for _ in 0..11 {
        handler.update(&mut shell, 1.0);
    }
    // Finishing a fire-once group drops it rather than leaving it current to be reversed.
    handler.update(&mut shell, 1.0);
    assert_eq!(handler.current_group(), None);
    assert!(!hidden(&shell, "SynthTrans.wnd:Fader"));

    // An immediate set jumps whatever was running to its end and starts the named group at once.
    handler.set_group(&mut shell, "SynthIn", false);
    handler.update(&mut shell, 1.0);
    assert!(hidden(&shell, "SynthTrans.wnd:Fader"));
    handler.set_group(&mut shell, "SynthGrow", true);
    assert_eq!(handler.current_group(), Some("SynthGrow"));
    assert_eq!(handler.pending_group(), None);
    // Skipping ran the fade's end frame, which showed its window.
    assert!(!hidden(&shell, "SynthTrans.wnd:Fader"));
}

#[test]
fn removing_and_reversing_a_group_by_name_follow_the_source_rules() {
    let (mut shell, mut handler) = transition_fixture();
    handler.set_group(&mut shell, "SynthIn", false);
    handler.update(&mut shell, 1.0);

    // Removing the current group skips it to its end and clears it.
    handler.remove(&mut shell, "SynthIn", false);
    assert_eq!(handler.current_group(), None);
    assert!(!hidden(&shell, "SynthTrans.wnd:Fader"));

    // Reversing a group that is not running starts it, skips it, and turns it around, which runs it
    // back out from its finished state.
    handler.reverse(&mut shell, "SynthIn");
    assert_eq!(handler.current_group(), Some("SynthIn"));
    assert!(handler.is_reversed());
    for _ in 0..25 {
        handler.update(&mut shell, 1.0);
    }
    assert!(hidden(&shell, "SynthTrans.wnd:Fader"));
    // A reversed group clears itself once it has run out.
    assert_eq!(handler.current_group(), None);

    // An unknown name is a no-op rather than a panic, unlike the source's unchecked dereference.
    assert!(
        handler
            .reverse(&mut shell, "SynthAbsent")
            .frames()
            .is_empty()
    );
    assert!(
        handler
            .remove(&mut shell, "SynthAbsent", true)
            .frames()
            .is_empty()
    );
}

#[test]
fn the_same_steps_produce_the_same_frames_and_draws() {
    let run = || {
        let (mut shell, mut handler) = transition_fixture();
        handler.set_group(&mut shell, "SynthIn", false);
        let mut frames = Vec::new();
        let mut draws = Vec::new();
        for _ in 0..25 {
            let step = handler.update(&mut shell, 0.7);
            frames.extend_from_slice(step.frames());
            draws.push(handler.draws());
        }
        (frames, draws)
    };
    assert_eq!(run(), run());
}

/// Two monitors, advertised out of order and with a duplicate, as a backend really might.
fn display_catalog() -> UiDisplayCatalog {
    let monitors = vec![
        UiMonitor::new("DISPLAY2", "Side", 1),
        UiMonitor::new("DISPLAY1", "Main", 0),
    ];
    let modes = vec![
        UiVideoMode::new("DISPLAY1", 1920, 1080, 59_940, Some(32), 3),
        UiVideoMode::new("DISPLAY1", 1920, 1080, 60_000, Some(32), 0),
        // The same mode advertised twice, which some backends do.
        UiVideoMode::new("DISPLAY1", 1920, 1080, 60_000, Some(32), 4),
        UiVideoMode::new("DISPLAY1", 1280, 720, 60_000, Some(32), 1),
        UiVideoMode::new("DISPLAY1", 2560, 1080, 144_000, Some(32), 2),
        // Below the presentable minimum, so no control ever offers it.
        UiVideoMode::new("DISPLAY1", 320, 240, 60_000, Some(16), 5),
        UiVideoMode::new("DISPLAY2", 1280, 1024, 75_000, None, 6),
    ];
    UiDisplayCatalog::new(monitors, modes).expect("a well-formed catalog")
}

#[test]
fn a_display_catalog_sorts_deduplicates_and_hides_what_cannot_be_presented() {
    let catalog = display_catalog();

    // Monitors sort by key, not by the order the adapter happened to enumerate them.
    assert_eq!(
        catalog
            .monitors()
            .iter()
            .map(UiMonitor::key)
            .collect::<Vec<_>>(),
        ["DISPLAY1", "DISPLAY2"]
    );
    // The duplicated 1920x1080 at 60 Hz appears once: seven advertised, six kept.
    assert_eq!(catalog.modes().len(), 6);

    // Resolutions are unique and ascending, and the 320x240 mode is not offered at all.
    assert_eq!(
        catalog.resolutions("DISPLAY1"),
        [(1280, 720), (1920, 1080), (2560, 1080)]
    );
    // 59.94 Hz and 60 Hz are distinct advertised rates and are not collapsed, because exclusive
    // fullscreen has to name one of them exactly.
    assert_eq!(
        catalog.refresh_rates("DISPLAY1", (1920, 1080)),
        [59_940, 60_000]
    );
    assert_eq!(catalog.refresh_rates("DISPLAY1", (1280, 720)), [60_000]);
    // A resolution the monitor does not advertise has no rates rather than borrowing another's.
    assert!(catalog.refresh_rates("DISPLAY1", (3840, 2160)).is_empty());

    // The desktop mode is the largest at the highest refresh, which is what borderless takes.
    let desktop = catalog.desktop_mode("DISPLAY1").expect("a desktop mode");
    assert_eq!(desktop.resolution(), (2560, 1080));
    assert_eq!(desktop.refresh_millihertz(), 144_000);

    // A monitor whose modes carry no refresh is reported rather than given a fabricated rate.
    let no_refresh = UiDisplayCatalog::new(
        vec![UiMonitor::new("DISPLAY1", "Main", 0)],
        vec![UiVideoMode::new("DISPLAY1", 1920, 1080, 0, None, 0)],
    )
    .expect("a catalog with no advertised refresh");
    assert_eq!(
        no_refresh.capabilities(),
        [UiDisplayCapability::NoRefreshRates {
            monitor: "DISPLAY1".to_owned()
        }]
    );
    assert!(
        no_refresh
            .refresh_rates("DISPLAY1", (1920, 1080))
            .is_empty()
    );
    // The well-formed catalog reports no gap for either monitor.
    assert!(catalog.capabilities().is_empty());
}

#[test]
fn each_window_mode_decides_what_the_player_may_actually_choose() {
    let catalog = display_catalog();
    let base = UiDisplaySelection {
        monitor: "DISPLAY1".to_owned(),
        window_mode: UiWindowMode::Windowed,
        resolution: (1600, 900),
        refresh_millihertz: 0,
        scale: UiScaleChoice::Automatic,
    };

    // Windowed keeps a client size the monitor never advertised — a window may be any size — and
    // reports the desktop's refresh rather than pretending to have selected one.
    let windowed = base.resolve(&catalog).expect("a windowed selection");
    assert_eq!(windowed.resolution, (1600, 900));
    assert_eq!(windowed.refresh_millihertz, 144_000);
    assert!(!UiWindowMode::Windowed.selects_refresh());

    // Borderless overrides both with the desktop mode whatever was asked for.
    let borderless = UiDisplaySelection {
        window_mode: UiWindowMode::BorderlessDesktop,
        resolution: (1280, 720),
        refresh_millihertz: 60_000,
        ..base.clone()
    }
    .resolve(&catalog)
    .expect("a borderless selection");
    assert_eq!(borderless.resolution, (2560, 1080));
    assert_eq!(borderless.refresh_millihertz, 144_000);
    assert!(!UiWindowMode::BorderlessDesktop.selects_resolution());

    // Exclusive fullscreen must name an advertised pair, and is the only mode that can fail on
    // refresh: the resolution exists, that rate on it does not.
    let exclusive = UiDisplaySelection {
        window_mode: UiWindowMode::ExclusiveFullscreen,
        resolution: (1920, 1080),
        refresh_millihertz: 59_940,
        ..base.clone()
    };
    assert_eq!(
        exclusive.resolve(&catalog).expect("an advertised pair"),
        exclusive
    );
    assert_eq!(
        UiDisplaySelection {
            refresh_millihertz: 120_000,
            ..exclusive.clone()
        }
        .resolve(&catalog),
        Err(UiSelectionError::UnknownRefreshRate {
            refresh_millihertz: 120_000
        })
    );
    assert_eq!(
        UiDisplaySelection {
            resolution: (3840, 2160),
            ..exclusive.clone()
        }
        .resolve(&catalog),
        Err(UiSelectionError::UnknownResolution {
            width: 3840,
            height: 2160
        })
    );
    // The 320x240 mode is advertised but below the minimum, so it is refused as unpresentable
    // rather than as unknown.
    assert_eq!(
        UiDisplaySelection {
            resolution: (320, 240),
            ..exclusive
        }
        .resolve(&catalog),
        Err(UiSelectionError::NotPresentable {
            width: 320,
            height: 240
        })
    );
    // A monitor that is gone — unplugged since the preference was written — is named, not guessed.
    assert_eq!(
        UiDisplaySelection {
            monitor: "DISPLAY9".to_owned(),
            ..base
        }
        .resolve(&catalog),
        Err(UiSelectionError::UnknownMonitor {
            key: "DISPLAY9".into()
        })
    );
}

/// A selection the fixture catalog accepts, for the transaction tests.
fn selection(width: u32, height: u32, refresh: u32) -> UiDisplaySelection {
    UiDisplaySelection {
        monitor: "DISPLAY1".to_owned(),
        window_mode: UiWindowMode::ExclusiveFullscreen,
        resolution: (width, height),
        refresh_millihertz: refresh,
        scale: UiScaleChoice::Automatic,
    }
}

#[test]
fn an_unconfirmed_mode_change_rolls_back_on_the_callers_own_clock() {
    let catalog = display_catalog();
    let accepted = selection(1280, 720, 60_000);
    let mut transaction = UiDisplayTransaction::new(accepted.clone());
    assert!(!transaction.is_awaiting_confirmation());

    let requested = selection(1920, 1080, 59_940);
    assert_eq!(
        transaction
            .request(&catalog, &requested, 1_000)
            .expect("an advertised pair"),
        UiDisplayOutcome::AwaitingConfirmation {
            deadline_ms: 1_000 + UI_DISPLAY_CONFIRM_TIMEOUT_MS
        }
    );
    assert!(transaction.is_awaiting_confirmation());
    // The new mode is not accepted yet, so nothing may be persisted from it.
    assert_eq!(transaction.accepted(), &accepted);

    // Polling inside the window does nothing at all, so a frame loop may call it every frame.
    assert_eq!(transaction.poll(2_000), UiDisplayOutcome::Idle);
    assert_eq!(transaction.poll(16_000), UiDisplayOutcome::Idle);

    // One millisecond past the deadline it reverts, and says how long it waited.
    assert_eq!(
        transaction.poll(16_001),
        UiDisplayOutcome::RolledBack {
            reason: UiRollbackReason::TimedOut { elapsed_ms: 15_001 },
            restored: Box::new(accepted.clone())
        }
    );
    assert_eq!(transaction.accepted(), &accepted);
    assert!(!transaction.is_awaiting_confirmation());
    // Nothing is pending any more, so a further poll is idle rather than rolling back twice.
    assert_eq!(transaction.poll(99_000), UiDisplayOutcome::Idle);
}

#[test]
fn only_a_confirmed_mode_becomes_the_accepted_one() {
    let catalog = display_catalog();
    let accepted = selection(1280, 720, 60_000);
    let mut transaction = UiDisplayTransaction::new(accepted).with_timeout_ms(5_000);

    // Confirming inside the window commits, and that is the only outcome a preference may follow.
    let requested = selection(1920, 1080, 60_000);
    transaction
        .request(&catalog, &requested, 0)
        .expect("an advertised pair");
    assert_eq!(
        transaction.confirm(4_999),
        UiDisplayOutcome::Confirmed {
            accepted: Box::new(requested.clone())
        }
    );
    assert_eq!(transaction.accepted(), &requested);

    // A confirmation that arrives after the deadline is refused: the dialog it answers is gone.
    let later = selection(2560, 1080, 144_000);
    transaction
        .request(&catalog, &later, 10_000)
        .expect("an advertised pair");
    assert_eq!(
        transaction.confirm(15_001),
        UiDisplayOutcome::RolledBack {
            reason: UiRollbackReason::TimedOut { elapsed_ms: 5_001 },
            restored: Box::new(requested.clone())
        }
    );
    assert_eq!(transaction.accepted(), &requested);

    // Declining and platform failure both revert to the same accepted selection.
    transaction
        .request(&catalog, &later, 20_000)
        .expect("an advertised pair");
    assert_eq!(
        transaction.decline(),
        UiDisplayOutcome::RolledBack {
            reason: UiRollbackReason::Declined,
            restored: Box::new(requested.clone())
        }
    );
    transaction
        .request(&catalog, &later, 30_000)
        .expect("an advertised pair");
    assert_eq!(
        transaction.fail("surface reconfiguration failed"),
        UiDisplayOutcome::RolledBack {
            reason: UiRollbackReason::Failed {
                detail: "surface reconfiguration failed".to_owned()
            },
            restored: Box::new(requested.clone())
        }
    );
    assert_eq!(transaction.accepted(), &requested);

    // A refused request applies nothing and leaves the accepted selection alone, so a player who
    // picks an impossible pair is not left with a broken display.
    assert!(
        transaction
            .request(&catalog, &selection(1920, 1080, 1), 40_000)
            .is_err()
    );
    assert!(!transaction.is_awaiting_confirmation());
    assert_eq!(transaction.accepted(), &requested);
    // Confirming with nothing pending is idle, not a commit of whatever was last requested.
    assert_eq!(transaction.confirm(41_000), UiDisplayOutcome::Idle);
}

#[test]
fn a_scale_choice_round_trips_only_through_the_steps_a_menu_offers() {
    for choice in [
        UiScaleChoice::Automatic,
        UiScaleChoice::Fixed(75),
        UiScaleChoice::Fixed(200),
    ] {
        assert_eq!(
            UiScaleChoice::from_row_name(&choice.row_name()),
            Some(choice)
        );
    }
    // A percentage no step offers is refused rather than silently kept, so a hand-edited preference
    // cannot introduce a scale the menu could never show back to the player.
    assert_eq!(UiScaleChoice::from_row_name("133"), None);
    assert_eq!(UiScaleChoice::from_row_name(""), None);
    for mode in [
        UiWindowMode::Windowed,
        UiWindowMode::BorderlessDesktop,
        UiWindowMode::ExclusiveFullscreen,
    ] {
        assert_eq!(UiWindowMode::from_row_name(mode.row_name()), Some(mode));
    }
    assert_eq!(UiWindowMode::from_row_name("fullscreen"), None);
}
