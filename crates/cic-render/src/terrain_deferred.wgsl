struct Camera {
    view_projection: mat4x4<f32>,
    camera_position_time: vec4<f32>,
    viewport: vec4<f32>,
    detail_fade_caustics: vec4<f32>,
    water_material: vec4<f32>,
}

struct FullscreenOutput {
    @builtin(position) position: vec4<f32>,
}

@vertex
fn fullscreen_vertex(@builtin(vertex_index) vertex_index: u32) -> FullscreenOutput {
    let x = f32(i32(vertex_index) - 1) * 3.0;
    let y = f32(i32(vertex_index & 1u) * 2 - 1) * 3.0;
    var output: FullscreenOutput;
    output.position = vec4<f32>(x, y, 0.0, 1.0);
    return output;
}

@group(0) @binding(0) var g_albedo: texture_2d<f32>;
@group(0) @binding(1) var g_normal: texture_2d<f32>;
@group(0) @binding(2) var g_world: texture_2d<f32>;
@group(0) @binding(3) var<uniform> light_camera: Camera;

@fragment
fn lighting_fragment(input: FullscreenOutput) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(input.position.xy);
    let world = textureLoad(g_world, pixel, 0);
    if (world.a < 0.5) {
        let horizon = clamp(input.position.y / light_camera.viewport.y, 0.0, 1.0);
        return vec4<f32>(mix(vec3<f32>(0.025, 0.04, 0.065), vec3<f32>(0.12, 0.20, 0.30), horizon), 1.0);
    }
    let albedo = textureLoad(g_albedo, pixel, 0).rgb;
    let normal_roughness = textureLoad(g_normal, pixel, 0);
    let normal = normalize(normal_roughness.xyz);
    let light_direction = normalize(vec3<f32>(-0.45, -0.35, 0.82));
    let diffuse = max(dot(normal, light_direction), 0.0);
    let view_direction = normalize(light_camera.camera_position_time.xyz - world.xyz);
    let half_vector = normalize(light_direction + view_direction);
    let specular = pow(max(dot(normal, half_vector), 0.0), mix(64.0, 8.0, normal_roughness.w)) * 0.06;
    let color = albedo * (0.38 + 0.62 * diffuse) + vec3<f32>(specular);
    return vec4<f32>(color, 1.0);
}

@group(1) @binding(0) var scene_color: texture_2d<f32>;

@fragment
fn composite_fragment(input: FullscreenOutput) -> @location(0) vec4<f32> {
    let hdr = textureLoad(scene_color, vec2<i32>(input.position.xy), 0).rgb;
    let mapped = hdr / (vec3<f32>(1.0) + hdr);
    return vec4<f32>(mapped, 1.0);
}
