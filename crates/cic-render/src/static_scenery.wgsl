struct Material {
    values: vec4<f32>,
}

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

@group(0) @binding(0) var base_color_texture: texture_2d<f32>;
@group(0) @binding(1) var base_color_sampler: sampler;
@group(0) @binding(2) var<uniform> material: Material;
@group(1) @binding(0) var<uniform> camera: Camera;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) texcoord: vec2<f32>,
    @location(4) transform_row_0: vec4<f32>,
    @location(5) transform_row_1: vec4<f32>,
    @location(6) transform_row_2: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) texcoord: vec2<f32>,
}

struct GBufferOutput {
    @location(0) albedo: vec4<f32>,
    @location(1) normal_roughness: vec4<f32>,
    @location(2) world_position: vec4<f32>,
}

@vertex
fn vertex_main(input: VertexInput) -> VertexOutput {
    let local = vec4<f32>(input.position, 1.0);
    let world = vec3<f32>(
        dot(input.transform_row_0, local),
        dot(input.transform_row_1, local),
        dot(input.transform_row_2, local)
    );
    let world_normal = normalize(vec3<f32>(
        dot(input.transform_row_0.xyz, input.normal),
        dot(input.transform_row_1.xyz, input.normal),
        dot(input.transform_row_2.xyz, input.normal)
    ));
    var output: VertexOutput;
    output.position = camera.view_projection * vec4<f32>(world, 1.0);
    output.world_position = world;
    output.normal = world_normal;
    output.color = input.color;
    output.texcoord = input.texcoord;
    return output;
}

@fragment
fn fragment_main(input: VertexOutput) -> GBufferOutput {
    let color = input.color * textureSample(base_color_texture, base_color_sampler, input.texcoord);
    if color.a < max(material.values.x, 0.0039) {
        discard;
    }
    var output: GBufferOutput;
    output.albedo = color;
    output.normal_roughness = vec4<f32>(normalize(input.normal), 0.72);
    output.world_position = vec4<f32>(input.world_position, 1.0);
    return output;
}
