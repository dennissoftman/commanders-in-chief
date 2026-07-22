struct Camera {
    view_projection: mat4x4<f32>,
    camera_position_time: vec4<f32>,
    viewport: vec4<f32>,
    detail_fade_caustics: vec4<f32>,
    water_material: vec4<f32>,
}

struct WaterVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
}

@group(0) @binding(0) var opaque_scene: texture_2d<f32>;
@group(0) @binding(1) var terrain_world: texture_2d<f32>;
@group(0) @binding(2) var<uniform> camera: Camera;
@group(0) @binding(3) var caustic_frames: texture_2d_array<f32>;
@group(0) @binding(4) var caustic_sampler: sampler;

@vertex
fn water_vertex(@location(0) position: vec3<f32>) -> WaterVertexOutput {
    var output: WaterVertexOutput;
    output.position = camera.view_projection * vec4<f32>(position, 1.0);
    output.world_position = position;
    return output;
}

fn wave_normal(position: vec2<f32>, time: f32) -> vec3<f32> {
    let phase_a = dot(position, vec2<f32>(0.026, 0.017)) + time * 0.75;
    let phase_b = dot(position, vec2<f32>(-0.013, 0.031)) - time * 0.52;
    let slope = vec2<f32>(
        cos(phase_a) * 0.11 - cos(phase_b) * 0.05,
        cos(phase_a) * 0.07 + cos(phase_b) * 0.12
    );
    return normalize(vec3<f32>(-slope, 1.0));
}

fn sampled_caustic(position: vec2<f32>, time: f32) -> f32 {
    let frame_count = max(i32(camera.detail_fade_caustics.z), 1);
    let frames_per_second = max(camera.detail_fade_caustics.w, 1.0);
    let frame = i32(floor(time * frames_per_second)) % frame_count;
    let sample = textureSample(caustic_frames, caustic_sampler, position / 96.0, frame).r;
    return smoothstep(0.28, 0.47, sample);
}

@fragment
fn water_fragment(input: WaterVertexOutput) -> @location(0) vec4<f32> {
    let dimensions = vec2<i32>(textureDimensions(opaque_scene));
    let pixel = clamp(vec2<i32>(input.position.xy), vec2<i32>(0), dimensions - vec2<i32>(1));
    let normal = wave_normal(input.world_position.xy, camera.camera_position_time.w);
    let refract_offset = vec2<i32>(round(normal.xy * 5.0));
    let refract_pixel = clamp(pixel + refract_offset, vec2<i32>(0), dimensions - vec2<i32>(1));
    let refracted_scene = textureLoad(opaque_scene, refract_pixel, 0).rgb;
    let bed = textureLoad(terrain_world, pixel, 0);
    var thickness = 60.0;
    if (bed.a > 0.5) { thickness = max(input.world_position.z - bed.z, 0.0); }
    let transmittance = exp(-vec3<f32>(0.060, 0.032, 0.022) * thickness);
    let depth_opacity = mix(
        camera.water_material.z,
        camera.water_material.x,
        smoothstep(0.15, camera.water_material.y, thickness)
    );
    let water_body = mix(
        vec3<f32>(0.035, 0.18, 0.21),
        vec3<f32>(0.012, 0.09, 0.13),
        smoothstep(3.0, 18.0, thickness)
    );
    let attenuated_scene = refracted_scene * transmittance;
    var transmission = mix(attenuated_scene, water_body, depth_opacity);
    if (bed.a > 0.5) {
        let caustic_depth = smoothstep(0.35, 0.9, thickness)
            * (1.0 - smoothstep(8.0, 18.0, thickness));
        let caustic = sampled_caustic(bed.xy, camera.camera_position_time.w) * caustic_depth;
        transmission += vec3<f32>(0.75, 1.0, 0.85) * camera.water_material.w * caustic;
    }
    let view_direction = normalize(camera.camera_position_time.xyz - input.world_position);
    let fresnel = 0.02 + 0.98 * pow(1.0 - max(dot(normal, view_direction), 0.0), 5.0);
    let light_direction = normalize(vec3<f32>(-0.45, -0.35, 0.82));
    let reflected_light = reflect(-light_direction, normal);
    let highlight = pow(max(dot(reflected_light, view_direction), 0.0), 180.0);
    let sky_reflection = mix(vec3<f32>(0.10, 0.20, 0.28), vec3<f32>(0.34, 0.48, 0.62), max(normal.z, 0.0));
    var color = mix(transmission, sky_reflection, fresnel) + vec3<f32>(highlight * 1.25);
    let shore_haze = 1.0 - smoothstep(0.2, 2.8, thickness);
    let shore_crest = smoothstep(0.08, 0.45, thickness)
        * (1.0 - smoothstep(1.35, 2.8, thickness));
    let foam_field = 0.5
        + 0.25 * sin(dot(input.world_position.xy, vec2<f32>(0.16, -0.11)) + camera.camera_position_time.w * 1.3)
        + 0.25 * sin(dot(input.world_position.xy, vec2<f32>(-0.07, 0.19)) - camera.camera_position_time.w * 0.9);
    let foam = shore_haze * 0.08 + shore_crest * smoothstep(0.35, 0.82, foam_field) * 0.34;
    color = mix(color, vec3<f32>(0.72, 0.82, 0.80), foam);
    return vec4<f32>(color / (vec3<f32>(1.0) + color), 1.0);
}
