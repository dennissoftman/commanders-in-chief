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

struct WaterVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
}

@group(0) @binding(0) var opaque_scene: texture_2d<f32>;
@group(0) @binding(1) var terrain_world: texture_2d<f32>;
@group(0) @binding(2) var<uniform> camera: Camera;
@group(0) @binding(3) var caustic_frames: texture_2d_array<f32>;
@group(0) @binding(4) var caustic_sampler: sampler;
@group(0) @binding(5) var standing_water_texture: texture_2d<f32>;
@group(0) @binding(6) var standing_water_sampler: sampler;
@group(0) @binding(7) var water_sky_texture: texture_2d<f32>;
@group(0) @binding(8) var water_sky_sampler: sampler;
@group(0) @binding(9) var water_environment_texture: texture_2d<f32>;
@group(0) @binding(10) var water_environment_sampler: sampler;

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
    let source_scroll = camera.water_motion.xy * camera.camera_position_time.w * 1000.0;
    let normal = wave_normal(input.world_position.xy + source_scroll, camera.camera_position_time.w);
    let refract_offset = vec2<i32>(round(normal.xy * 5.0));
    let refract_pixel = clamp(pixel + refract_offset, vec2<i32>(0), dimensions - vec2<i32>(1));
    let refracted_scene = textureLoad(opaque_scene, refract_pixel, 0).rgb;
    let bed = textureLoad(terrain_world, pixel, 0);
    var thickness = 60.0;
    if (bed.a > 0.5) { thickness = max(input.world_position.z - bed.z, 0.0); }
    let depth_opacity = mix(
        camera.water_material.x,
        1.0,
        smoothstep(0.0, camera.water_material.y, thickness)
    );
    let shore_width = max(camera.water_material.y * 0.08, 0.65);
    let shore_coverage = smoothstep(0.02, shore_width, thickness);
    let source_surface = mix(
        vec4<f32>(1.0),
        camera.water_surface,
        camera.water_motion.z
    );
    if (camera.water_motion.w < 0.5) {
        let river_origin = camera.camera_position_time.w * 0.06;
        let wobble = vec2<f32>(
            0.02 * cos(river_origin * 11.0)
                * sin(river_origin * 25.0 + input.world_position.x * 0.078539816),
            0.02 * cos(river_origin * 5.0)
                * sin(river_origin * 25.0 + input.world_position.y * 0.078539816)
        );
        let uv = input.world_position.xy / 150.0 + wobble;
        let surface = textureSample(standing_water_texture, standing_water_sampler, uv);
        let alpha = clamp(
            surface.a * source_surface.a * depth_opacity * shore_coverage,
            0.0,
            1.0
        );
        return vec4<f32>(surface.rgb * source_surface.rgb, alpha);
    }
    let transmittance = exp(-vec3<f32>(0.060, 0.032, 0.022) * thickness);
    let shallow_color = mix(
        vec3<f32>(0.035, 0.18, 0.21),
        camera.water_surface.rgb,
        camera.water_motion.z
    );
    let deep_color = mix(
        vec3<f32>(0.012, 0.09, 0.13),
        camera.water_surface.rgb * vec3<f32>(0.32, 0.48, 0.58),
        camera.water_motion.z
    );
    let water_body = mix(
        shallow_color,
        deep_color,
        smoothstep(3.0, 18.0, thickness)
    );
    let authored_environment = textureSample(
        water_environment_texture,
        water_environment_sampler,
        input.world_position.xy / 150.0 + source_scroll
    ).rgb;
    let authored_water_body = mix(water_body, authored_environment * source_surface.rgb, 0.18);
    let attenuated_scene = refracted_scene * transmittance;
    var transmission = mix(attenuated_scene, authored_water_body, depth_opacity);
    if (bed.a > 0.5) {
        let caustic_depth = smoothstep(0.35, 0.9, thickness)
            * (1.0 - smoothstep(8.0, 18.0, thickness));
        let caustic = sampled_caustic(bed.xy, camera.camera_position_time.w) * caustic_depth;
        transmission += vec3<f32>(0.75, 1.0, 0.85) * camera.water_material.w * caustic;
    }
    let view_direction = normalize(camera.camera_position_time.xyz - input.world_position);
    let fresnel = 0.02 + 0.98 * pow(1.0 - max(dot(normal, view_direction), 0.0), 5.0);
    var highlight = vec3<f32>(0.0);
    for (var index = 0; index < 3; index += 1) {
        let light = camera.terrain_lights[index];
        let direction_length = length(light.source_direction.xyz);
        if (direction_length > 0.00001) {
            let light_direction = -light.source_direction.xyz / direction_length;
            let reflected_light = reflect(-light_direction, normal);
            let response = pow(max(dot(reflected_light, view_direction), 0.0), 180.0);
            highlight += light.diffuse.rgb * response * 1.25;
        }
    }
    let reflected_view = reflect(-view_direction, normal);
    let sky_uv = vec2<f32>(
        fract(0.5 + atan2(reflected_view.y, reflected_view.x) / 6.28318530718),
        clamp(0.5 - asin(clamp(reflected_view.z, -1.0, 1.0)) / 3.14159265359, 0.0, 1.0)
    );
    let authored_sky = textureSample(water_sky_texture, water_sky_sampler, sky_uv).rgb;
    let preview_sky = mix(
        vec3<f32>(0.10, 0.20, 0.28),
        vec3<f32>(0.34, 0.48, 0.62),
        max(normal.z, 0.0)
    );
    let reflected_pixel = clamp(
        pixel + vec2<i32>(round(reflected_view.xy * 18.0)),
        vec2<i32>(0),
        dimensions - vec2<i32>(1)
    );
    let screen_reflection = textureLoad(opaque_scene, reflected_pixel, 0).rgb;
    let sky_reflection = mix(preview_sky, authored_sky, 0.65);
    let bounded_reflection = mix(sky_reflection, screen_reflection, 0.22);
    var color = mix(transmission, bounded_reflection, fresnel) + highlight;
    let shore_haze = 1.0 - smoothstep(0.2, 2.8, thickness);
    let shore_crest = smoothstep(0.08, 0.45, thickness)
        * (1.0 - smoothstep(1.35, 2.8, thickness));
    let foam_field = 0.5
        + 0.25 * sin(dot(input.world_position.xy, vec2<f32>(0.16, -0.11)) + camera.camera_position_time.w * 1.3)
        + 0.25 * sin(dot(input.world_position.xy, vec2<f32>(-0.07, 0.19)) - camera.camera_position_time.w * 0.9);
    let foam = shore_haze * 0.08 + shore_crest * smoothstep(0.35, 0.82, foam_field) * 0.34;
    color = mix(color, vec3<f32>(0.72, 0.82, 0.80), foam);
    let alpha = clamp(source_surface.a * depth_opacity * shore_coverage, 0.0, 1.0);
    return vec4<f32>(color / (vec3<f32>(1.0) + color), alpha);
}
// Legacy standing-water texture scale, source tint/alpha, and depth-feather policy are derived
// from W3DWater.cpp in GeneralsGameCode revision
// 9f7abb866f5afd446db14149979e744c7216baaf (GPL-3.0 with Section 7 terms); see
// docs/provenance/map.md. The bounded screen/sky reflection and Modern branch remain original
// project work sampled only from explicit presentation time and caller-resolved textures.
