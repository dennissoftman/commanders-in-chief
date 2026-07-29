// Renders retained-UI quads. Positions arrive in viewport pixels with the origin at the top left, and
// are converted to clip space here so no CPU-side transform is needed.
//
// Colours arrive **linear** and premultiplication-free. The theme authors them as sRGB bytes and the
// paint layer removes the encoding, because the target is sRGB-encoded and the hardware applies the
// transfer function on write -- passing the bytes through as though they were linear is what makes every
// surface too bright.

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
        // The glyph atlas holds *coverage*, one channel of it, so it modulates alpha and never colour.
        // Multiplying the colour instead would darken every antialiased edge toward black rather than
        // fading it toward the background, which reads as text with a dirty outline.
        let coverage = textureSample(page, page_sampler, input.uv).r;
        return vec4<f32>(input.color.rgb, input.color.a * coverage);
    }
    return input.color;
}
