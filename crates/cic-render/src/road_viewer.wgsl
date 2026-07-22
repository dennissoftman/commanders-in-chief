struct DirectionalLight {
    ambient: vec4<f32>,
    diffuse: vec4<f32>,
    source_direction: vec4<f32>,
}

struct Camera {
    view_projection: mat4x4<f32>,
    camera_position_time: vec4<f32>,
    viewport: vec4<f32>,
    detail_fade_caustics: vec4<f32>,
    water_material: vec4<f32>,
    water_surface: vec4<f32>,
    water_motion: vec4<f32>,
    terrain_lights: array<DirectionalLight, 3>,
}

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) normal: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

struct GBufferOutput {
    @location(0) albedo: vec4<f32>,
    @location(1) normal_roughness: vec4<f32>,
    @location(2) world_position: vec4<f32>,
}

@group(0) @binding(0) var road_texture: texture_2d<f32>;
@group(0) @binding(1) var road_sampler: sampler;
@group(0) @binding(2) var<uniform> camera: Camera;

@vertex
fn vertex_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.position = camera.view_projection * vec4<f32>(input.position, 1.0);
    output.uv = input.uv;
    return output;
}

@fragment
fn fragment_main(input: VertexOutput) -> GBufferOutput {
    var output: GBufferOutput;
    output.albedo = textureSample(road_texture, road_sampler, input.uv);
    // The pipeline disables writes for these attachments. Roads retain the underlying terrain
    // geometry buffers while alpha-compositing their authored sheet into albedo.
    output.normal_roughness = vec4<f32>(0.0);
    output.world_position = vec4<f32>(0.0);
    return output;
}
