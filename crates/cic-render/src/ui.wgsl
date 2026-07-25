// Commanders in Chief
// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only
//
// Renders retained-UI quads. Positions arrive in viewport pixels with the origin at the top left,
// matching WND coordinates, and are converted to clip space here so no CPU-side transform is
// needed. Colour is straight (non-premultiplied) RGBA to match the source's stored channel bytes.

struct Viewport {
    size: vec2<f32>,
    padding: vec2<f32>,
};

@group(0) @binding(0) var<uniform> viewport: Viewport;
@group(1) @binding(0) var page: texture_2d<f32>;
@group(1) @binding(1) var page_sampler: sampler;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) textured: u32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    // An integer varying has no meaningful interpolation, and WGSL requires saying so.
    @location(2) @interpolate(flat) textured: u32,
};

@vertex
fn vertex_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    let normalized = input.position / viewport.size;
    output.clip_position = vec4<f32>(
        normalized.x * 2.0 - 1.0,
        1.0 - normalized.y * 2.0,
        0.0,
        1.0
    );
    output.uv = input.uv;
    output.color = input.color;
    output.textured = input.textured;
    return output;
}

@fragment
fn fragment_main(input: VertexOutput) -> @location(0) vec4<f32> {
    if input.textured == 1u {
        return textureSample(page, page_sampler, input.uv) * input.color;
    }
    return input.color;
}
