struct Material {
    values: vec4<f32>,
}

@group(0) @binding(0)
var base_color_texture: texture_2d<f32>;

@group(0) @binding(1)
var base_color_sampler: sampler;

@group(0) @binding(2)
var<uniform> material: Material;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) texcoord: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) texcoord: vec2<f32>,
}

@vertex
fn vertex_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.position = vec4<f32>(input.position, 1.0);
    output.color = input.color;
    output.texcoord = input.texcoord;
    return output;
}

@fragment
fn fragment_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let color = input.color * textureSample(base_color_texture, base_color_sampler, input.texcoord);
    if color.a < material.values.x {
        discard;
    }
    return color;
}
