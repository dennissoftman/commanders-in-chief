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

// `params` packs the presentation time in `x`, the world units spanned by the full normalized
// depth range in `y`, and the world units covered by one shadow texel in `z`. `w` is reserved.
// Both scales are per cascade, since the fitted frusta differ by more than an order of magnitude.
struct ShadowCascade {
    view_projection: mat4x4<f32>,
    params: vec4<f32>,
}

const SHADOW_CASCADE_COUNT: i32 = 5;

struct ShadowCamera {
    cascades: array<ShadowCascade, 5>,
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
@group(0) @binding(4) var primary_shadow: texture_depth_2d_array;
@group(0) @binding(5) var primary_shadow_sampler: sampler_comparison;
@group(0) @binding(6) var<uniform> shadow_camera: ShadowCamera;
@group(0) @binding(8) var ambient_occlusion: texture_2d<f32>;

// Both bias terms are expressed in world units and converted with the light frustum's own
// depth range, so widening the fitted frustum for a larger map no longer inflates the bias in
// world space. Nearly all of the slack is taken laterally by offsetting the receiver along its
// normal — the depth slack that remains is a fraction of a texel, which keeps contact shadows
// attached instead of peter-panning them away. Slope-dependent acne is handled by the shadow
// pipelines' rasterizer slope-scaled bias rather than a second term here.
const SHADOW_NORMAL_OFFSET_TEXELS: f32 = 1.5;
const SHADOW_DEPTH_SLACK_TEXELS: f32 = 0.5;

// How much of the primary light survives in full shade. The direct term is nearly extinguished;
// its ambient share only drops part way, standing in for the sky light that still reaches a
// shadowed surface. Ambient has to be attenuated at all because it dominates the sum: the three
// source terrain lights each contribute their own unoccluded ambient, so leaving ambient whole in
// shade caps achievable shadow contrast around ten percent and cast shadows read as absent.
const SHADOW_DIRECT_FLOOR: f32 = 0.08;
const SHADOW_AMBIENT_FLOOR: f32 = 0.45;

// Ambient occlusion attenuates every light's ambient share, including the accent fills the sun's
// shadow deliberately leaves alone: occlusion answers "how much of the sky can this point see",
// which the directional shadow cannot, since that cannot tell a shadowed open field from an
// enclosed corner. Clamped rather than applied whole so interiors darken without going black.
const AO_AMBIENT_FLOOR: f32 = 0.25;

// Peak highlight strength, reached only by a fully smooth material.
const SPECULAR_STRENGTH: f32 = 0.06;

// Fraction of a cascade's half-extent, measured inward from its outer boundary, over which its
// result crossfades into the next cascade. The kernel spans a fixed three texels, so its
// world-space width scales with the cascade's texel extent and penumbrae are an order of magnitude
// wider in the outermost cascade than the innermost. Selecting one cascade outright therefore puts
// a visible line on screen where sharp shadows meet blurry ones; fading between them over a band
// turns that line into a gradient, which is what makes the transition imperceptible.
const SHADOW_CASCADE_BLEND: f32 = 0.18;

struct CascadeSample {
    visibility: f32,
    // 1.0 when this cascade contains the receiver at all.
    covered: f32,
    // 0.0 in the cascade's interior, ramping to 1.0 at its outer boundary.
    edge: f32,
}

// `textureSampleCompareLevel` rather than `textureSampleCompare`: the caller's per-pixel loop exit
// makes this non-uniform control flow, which the implicit-derivative variant does not permit.
// Shadow maps never want a mip anyway.
fn sample_cascade(index: i32, world_position: vec3<f32>, normal: vec3<f32>) -> CascadeSample {
    var result: CascadeSample;
    result.visibility = 1.0;
    result.covered = 0.0;
    result.edge = 0.0;
    let cascade = shadow_camera.cascades[index];
    let texel_world = cascade.params.z;
    let depth_range = max(cascade.params.y, 1.0);
    let offset_position = world_position + normal * texel_world * SHADOW_NORMAL_OFFSET_TEXELS;
    let clip = cascade.view_projection * vec4<f32>(offset_position, 1.0);
    if clip.w <= 0.0 {
        return result;
    }
    let projected = clip.xyz / clip.w;
    let uv = projected.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))
        || projected.z < 0.0 || projected.z > 1.0 {
        return result;
    }
    let depth_slack = texel_world * SHADOW_DEPTH_SLACK_TEXELS / depth_range;
    let texel = 1.0 / vec2<f32>(textureDimensions(primary_shadow).xy);
    var visible = 0.0;
    for (var y = -1; y <= 1; y += 1) {
        for (var x = -1; x <= 1; x += 1) {
            visible += textureSampleCompareLevel(
                primary_shadow,
                primary_shadow_sampler,
                uv + vec2<f32>(f32(x), f32(y)) * texel,
                index,
                projected.z - depth_slack
            );
        }
    }
    result.visibility = visible / 9.0;
    result.covered = 1.0;
    let centered = abs(uv * 2.0 - vec2<f32>(1.0));
    let inset = 1.0 - max(centered.x, centered.y);
    result.edge = 1.0 - clamp(inset / SHADOW_CASCADE_BLEND, 0.0, 1.0);
    return result;
}

// Returns the raw unoccluded fraction in `0..=1`. Callers apply their own floor, because opaque
// terrain and the transmissive water surface keep different amounts of light in full shade.
//
// Cascades are fitted smallest first, so the first one that contains the receiver is also the
// densest one that does; testing them in order is both the selection rule and the coverage test.
// A receiver outside every cascade is beyond the shadowed distance and reads as fully lit. Near a
// cascade's outer boundary the next cascade is sampled too and the two are crossfaded, so only the
// band pays for a second lookup.
fn shadow_visibility(world_position: vec3<f32>, normal: vec3<f32>) -> f32 {
    for (var index = 0; index < SHADOW_CASCADE_COUNT; index += 1) {
        let current = sample_cascade(index, world_position, normal);
        if current.covered < 0.5 {
            continue;
        }
        if current.edge <= 0.0 || index + 1 >= SHADOW_CASCADE_COUNT {
            return current.visibility;
        }
        let outer = sample_cascade(index + 1, world_position, normal);
        if outer.covered < 0.5 {
            return current.visibility;
        }
        return mix(current.visibility, outer.visibility, current.edge);
    }
    return 1.0;
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
    let primary_visibility = shadow_visibility(world.xyz, normal);
    let occlusion = mix(
        AO_AMBIENT_FLOOR,
        1.0,
        textureLoad(ambient_occlusion, pixel, 0).r
    );
    var color = vec3<f32>(0.0);
    for (var index = 0; index < 3; index += 1) {
        let light = light_camera.terrain_lights[index];
        // Lights 1 and 2 are the source's accent fills and stay unoccluded; only the primary
        // light is shadowed, in both its ambient and direct shares.
        let shadowed = index == 0;
        let ambient_scale = select(
            1.0,
            mix(SHADOW_AMBIENT_FLOOR, 1.0, primary_visibility),
            shadowed
        );
        color += albedo * light.ambient.rgb * ambient_scale * occlusion;
        let direction_length = length(light.source_direction.xyz);
        if (direction_length > 0.00001) {
            let visibility = select(
                1.0,
                mix(SHADOW_DIRECT_FLOOR, 1.0, primary_visibility),
                shadowed
            );
            let light_direction = -light.source_direction.xyz / direction_length;
            let diffuse_factor = max(dot(normal, light_direction), 0.0);
            color += albedo * light.diffuse.rgb * diffuse_factor * visibility;
            // Highlight strength falls off with roughness, not just its width, so a fully rough
            // material has no highlight at all. A fixed strength gave every surface in the scene a
            // sheen regardless of what its source material declared, and did so once per light.
            let half_vector = normalize(light_direction + view_direction);
            let specular = pow(
                max(dot(normal, half_vector), 0.0),
                mix(64.0, 8.0, normal_roughness.w)
            );
            let specular_strength = SPECULAR_STRENGTH * (1.0 - normal_roughness.w);
            color += light.diffuse.rgb * specular * specular_strength * visibility;
        }
    }
    // W3D self-illumination, decoded from the G-buffer coverage channel (see
    // `gbuffer_coverage_emissive` in `static_scenery.wgsl`). It is added after the light loop so
    // it survives full shade, which is the whole point of a lamp: the emitted term takes its hue
    // from the material's own albedo, and the intensity is the material's emissive strength.
    color += albedo * max(world.a - 1.0, 0.0);
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
