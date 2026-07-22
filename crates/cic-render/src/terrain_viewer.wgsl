struct Camera {
    view_projection: mat4x4<f32>,
    camera_position_time: vec4<f32>,
    viewport: vec4<f32>,
    detail_fade_caustics: vec4<f32>,
    water_material: vec4<f32>,
}

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) world_position: vec3<f32>,
}

struct GBufferOutput {
    @location(0) albedo: vec4<f32>,
    @location(1) normal_roughness: vec4<f32>,
    @location(2) world_position: vec4<f32>,
}

@group(0) @binding(0) var terrain_texture: texture_2d<f32>;
@group(0) @binding(1) var terrain_sampler: sampler;
@group(0) @binding(2) var<uniform> camera: Camera;

@vertex
fn vertex_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.position = camera.view_projection * vec4<f32>(input.position, 1.0);
    output.uv = input.uv;
    output.world_position = input.position;
    return output;
}

fn surface_output(input: VertexOutput) -> GBufferOutput {
    var normal = normalize(cross(dpdx(input.world_position), dpdy(input.world_position)));
    if (normal.z < 0.0) { normal = -normal; }
    var output: GBufferOutput;
    output.albedo = textureSample(terrain_texture, terrain_sampler, input.uv);
    output.normal_roughness = vec4<f32>(normal, 0.88);
    output.world_position = vec4<f32>(input.world_position, 1.0);
    return output;
}

@fragment
fn fragment_main(input: VertexOutput) -> GBufferOutput {
    return surface_output(input);
}

@fragment
fn detail_fragment_main(input: VertexOutput) -> GBufferOutput {
    var output = surface_output(input);
    let edge_distance = min(input.uv, vec2<f32>(1.0) - input.uv);
    let coverage = smoothstep(0.0, camera.detail_fade_caustics.x, edge_distance.x)
        * smoothstep(0.0, camera.detail_fade_caustics.y, edge_distance.y)
        * camera.detail_fade_caustics.z;
    output.albedo.a *= coverage;
    output.normal_roughness.a *= coverage;
    output.world_position.a *= coverage;
    return output;
}
