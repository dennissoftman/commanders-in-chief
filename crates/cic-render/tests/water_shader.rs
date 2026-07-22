// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only

#[test]
fn water_shader_parses_and_validates_with_environment_reflections() {
    let source = include_str!("../src/water_viewer.wgsl");
    let module = naga::front::wgsl::parse_str(source).expect("water WGSL parses");
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .expect("water WGSL validates");
}

#[test]
fn road_shader_parses_and_validates_for_albedo_overlay() {
    let source = include_str!("../src/road_viewer.wgsl");
    let module = naga::front::wgsl::parse_str(source).expect("road WGSL parses");
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .expect("road WGSL validates");
}

#[test]
fn static_scenery_and_boundary_shaders_parse_and_validate() {
    for (name, source) in [
        ("static scenery", include_str!("../src/static_scenery.wgsl")),
        (
            "boundary fence",
            include_str!("../src/boundary_viewer.wgsl"),
        ),
    ] {
        let module = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|error| panic!("{name} WGSL parses: {error}"));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|error| panic!("{name} WGSL validates: {error}"));
    }
}
