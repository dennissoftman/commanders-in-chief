// Commanders in Chief
// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only

//! A surface-free capture of the synthetic all-families layout.
//!
//! Every composed gadget family appears once, over original synthetic pages, so the composition
//! geometry is exercised end to end through the same boundary a user-owned capture uses. No retail
//! data appears here. Setting `CIC_UI_CAPTURE_PNG` to a path writes the capture out, which is how
//! the composition was looked at rather than only asserted.

use std::collections::BTreeMap;

use cic_formats::{WndLimits, parse_wnd};
use cic_render::{
    HeadlessRenderer, RenderError, StagedUiFrame, UiImageBinding, UiStagingLimits, UiTextPolicy,
    UiTexturePage,
};
use cic_ui::{UiClipPolicy, UiLayout, UiLimits, UiPresentation, UiScalePolicy, UiViewport};

const SYNTHETIC_GADGETS: &str = include_str!("fixtures/synthetic-gadgets.wnd");

/// Builds one flat synthetic page holding every named image as its own region.
///
/// Regions are laid out left to right in a single row, so a binding's texture coordinates and its
/// declared pixel size are independent inputs to the composition, as they are for a real atlas.
fn page() -> (UiTexturePage, BTreeMap<String, UiImageBinding>) {
    // Each entry is a name, its size in pixels, and its opaque colour.
    let images: [(&str, [i32; 2], [u8; 3]); 5] = [
        ("SynthPanel", [64, 32], [40, 48, 64]),
        ("SynthEnd", [10, 24], [200, 170, 60]),
        ("SynthMiddle", [8, 24], [120, 100, 40]),
        ("SynthBox", [4, 6], [90, 150, 200]),
        ("SynthPicked", [4, 6], [220, 90, 90]),
    ];
    let width: i32 = images.iter().map(|(_, size, _)| size[0]).sum();
    let height: i32 = images.iter().map(|(_, size, _)| size[1]).max().unwrap_or(1);
    let (page_width, page_height) = (
        u32::try_from(width).expect("positive width"),
        u32::try_from(height).expect("positive height"),
    );

    let mut rgba = vec![0_u8; usize::try_from(width * height * 4).expect("positive page")];
    let mut bindings = BTreeMap::new();
    let mut left = 0_i32;
    for (name, size, color) in images {
        for y in 0..size[1] {
            for x in 0..size[0] {
                let offset = usize::try_from((y * width + left + x) * 4).expect("in-page offset");
                rgba[offset] = color[0];
                rgba[offset + 1] = color[1];
                rgba[offset + 2] = color[2];
                rgba[offset + 3] = 255;
            }
        }
        #[expect(
            clippy::cast_precision_loss,
            reason = "page extents are small pixel counts"
        )]
        let uv = [
            left as f32 / width as f32,
            0.0,
            (left + size[0]) as f32 / width as f32,
            size[1] as f32 / height as f32,
        ];
        bindings.insert(name.to_owned(), UiImageBinding { page: 0, uv, size });
        left += size[0];
    }
    (
        UiTexturePage::new(page_width, page_height, rgba).expect("synthetic page"),
        bindings,
    )
}

#[test]
fn every_composed_family_captures_deterministically() {
    let document =
        parse_wnd(SYNTHETIC_GADGETS.as_bytes(), WndLimits::default()).expect("decode layout");
    let viewport = UiViewport::new(800, 600).expect("positive viewport");
    let layout = UiLayout::instantiate(
        &document,
        UiPresentation::new(viewport, UiScalePolicy::Classic),
        UiLimits::default(),
    )
    .expect("instantiate layout");

    let (page, bindings) = page();
    let staged = StagedUiFrame::from_frame(
        &layout.frame(UiClipPolicy::None),
        // The fixture panel is authored at the creation resolution, so an 800x600 viewport draws it
        // at one screen pixel per authored pixel and every composed piece keeps its declared size.
        [800, 600],
        // No font is supplied, so text stages a visible placeholder rather than reaching for a
        // host face, which is what keeps this capture deterministic.
        UiTextPolicy::Placeholder,
        UiStagingLimits::default(),
        &|name| bindings.get(name).copied(),
    )
    .expect("stage frame");

    let renderer = match pollster::block_on(HeadlessRenderer::new()) {
        Ok(renderer) => renderer,
        Err(RenderError::RequestAdapter(error)) => {
            eprintln!("skipping GPU capture without a headless adapter: {error}");
            return;
        }
        Err(error) => panic!("initializing headless renderer: {error}"),
    };
    let capture = renderer
        .capture_ui_frame(
            &staged,
            std::slice::from_ref(&page),
            None,
            [0.0, 0.0, 0.0, 1.0],
        )
        .expect("capture frame");

    if let Ok(path) = std::env::var("CIC_UI_CAPTURE_PNG") {
        std::fs::write(path, encode_png(&capture)).expect("write capture");
    }

    // Two captures of the same explicit inputs agree byte for byte.
    let again = renderer
        .capture_ui_frame(
            &staged,
            std::slice::from_ref(&page),
            None,
            [0.0, 0.0, 0.0, 1.0],
        )
        .expect("capture frame");
    assert_eq!(capture.sha256(), again.sha256());
}

/// Encodes a capture as an eight-bit RGBA PNG.
fn encode_png(capture: &cic_render::Capture) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, capture.width(), capture.height());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("png header");
    writer.write_image_data(capture.rgba()).expect("png data");
    writer.finish().expect("png finish");
    bytes
}
