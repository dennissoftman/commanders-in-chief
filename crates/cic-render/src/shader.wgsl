struct Pose {
    transform: mat4x4<f32>,
}

@group(0) @binding(0)
var<uniform> pose: Pose;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
}

@vertex
fn vertex_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-0.5, -0.5),
        vec2<f32>( 0.5, -0.5),
        vec2<f32>( 0.0,  0.5),
    );
    var output: VertexOutput;
    output.position = pose.transform * vec4<f32>(positions[index], 0.0, 1.0);
    output.color = vec4<f32>(0.25, 0.5, 0.75, 1.0);
    return output;
}

@fragment
fn fragment_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return input.color;
}
