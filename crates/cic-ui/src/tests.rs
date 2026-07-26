// Commanders in Chief
// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Retained-runtime tests over original synthetic layouts. No retail data appears here.

use cic_formats::{WndColor, WndDrawDataSlot, WndLimits, parse_wnd};

use crate::{
    UiClipPolicy, UiControlFamily, UiControlId, UiControlKind, UiEvent, UiFrameItem, UiGadgetRole,
    UiKey, UiLayout, UiLayoutError, UiLimits, UiMouseButton, UiPoint, UiPresentation, UiRect,
    UiScalePolicy, UiStatus, UiTextAlign, UiViewport,
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
