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

struct ShadowCamera {
    view_projection: mat4x4<f32>,
    time: vec4<f32>,
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
@group(0) @binding(4) var primary_shadow: texture_depth_2d;
@group(0) @binding(5) var primary_shadow_sampler: sampler_comparison;
@group(0) @binding(6) var<uniform> shadow_camera: ShadowCamera;

fn shadow_visibility(world_position: vec3<f32>) -> f32 {
    let clip = shadow_camera.view_projection * vec4<f32>(world_position, 1.0);
    if clip.w <= 0.0 {
        return 1.0;
    }
    let projected = clip.xyz / clip.w;
    let uv = projected.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))
        || projected.z < 0.0 || projected.z > 1.0 {
        return 1.0;
    }
    let texel = 1.0 / vec2<f32>(textureDimensions(primary_shadow));
    var visible = 0.0;
    for (var y = -1; y <= 1; y += 1) {
        for (var x = -1; x <= 1; x += 1) {
            visible += textureSampleCompare(
                primary_shadow,
                primary_shadow_sampler,
                uv + vec2<f32>(f32(x), f32(y)) * texel,
                projected.z - 0.0015
            );
        }
    }
    return mix(0.35, 1.0, visible / 9.0);
}

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
    let view_direction = normalize(light_camera.camera_position_time.xyz - world.xyz);
    let primary_visibility = shadow_visibility(world.xyz);
    var color = vec3<f32>(0.0);
    for (var index = 0; index < 3; index += 1) {
        let light = light_camera.terrain_lights[index];
        color += albedo * light.ambient.rgb;
        let direction_length = length(light.source_direction.xyz);
        if (direction_length > 0.00001) {
            let visibility = select(1.0, primary_visibility, index == 0);
            let light_direction = -light.source_direction.xyz / direction_length;
            let diffuse_factor = max(dot(normal, light_direction), 0.0);
            color += albedo * light.diffuse.rgb * diffuse_factor * visibility;
            let half_vector = normalize(light_direction + view_direction);
            let specular = pow(
                max(dot(normal, half_vector), 0.0),
                mix(64.0, 8.0, normal_roughness.w)
            );
            color += light.diffuse.rgb * specular * 0.06 * visibility;
        }
    }
    return vec4<f32>(color, 1.0);
}

@group(1) @binding(0) var scene_color: texture_2d<f32>;
@group(1) @binding(1) var scene_sampler: sampler;

fn reinhard(hdr: vec3<f32>) -> vec3<f32> {
    return hdr / (vec3<f32>(1.0) + hdr);
}

// A contrast-adaptive sharpen in the spirit of AMD FidelityFX CAS: it boosts an unsharp-mask
// style detail term by an amount that scales down toward zero both near luminance extremes
// (avoids blooming/crushing) and at genuinely hard edges (avoids ringing on the very silhouette
// edges MSAA already resolved), so it only restores softer mid-contrast detail lost to mip/
// texture filtering — real MSAA has already handled geometric edge aliasing by this point.
const SHARPEN_STRENGTH: f32 = 0.6;

@fragment
fn composite_fragment(input: FullscreenOutput) -> @location(0) vec4<f32> {
    let inverse_viewport = 1.0 / light_camera.viewport.xy;
    let uv = (input.position.xy + vec2<f32>(0.5)) * inverse_viewport;
    let center = reinhard(textureSampleLevel(scene_color, scene_sampler, uv, 0.0).rgb);
    let north = reinhard(textureSampleLevel(
        scene_color,
        scene_sampler,
        uv + vec2<f32>(0.0, -inverse_viewport.y),
        0.0
    ).rgb);
    let south = reinhard(textureSampleLevel(
        scene_color,
        scene_sampler,
        uv + vec2<f32>(0.0, inverse_viewport.y),
        0.0
    ).rgb);
    let west = reinhard(textureSampleLevel(
        scene_color,
        scene_sampler,
        uv + vec2<f32>(-inverse_viewport.x, 0.0),
        0.0
    ).rgb);
    let east = reinhard(textureSampleLevel(
        scene_color,
        scene_sampler,
        uv + vec2<f32>(inverse_viewport.x, 0.0),
        0.0
    ).rgb);
    let minimum = min(center, min(min(north, south), min(west, east)));
    let maximum = max(center, max(max(north, south), max(west, east)));
    let peak = min(minimum, vec3<f32>(1.0) - maximum) / max(maximum, vec3<f32>(0.001));
    let amplitude = sqrt(clamp(peak, vec3<f32>(0.0), vec3<f32>(1.0))) * SHARPEN_STRENGTH;
    let neighbor_average = (north + south + west + east) * 0.25;
    let sharpened = center + (center - neighbor_average) * amplitude;
    return vec4<f32>(clamp(sharpened, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}

@group(0) @binding(7) var gbuffer_depth_ms: texture_depth_multisampled_2d;

@fragment
fn depth_resolve_fragment(input: FullscreenOutput) -> @builtin(frag_depth) f32 {
    let pixel = vec2<i32>(input.position.xy);
    return textureLoad(gbuffer_depth_ms, pixel, 0);
}
