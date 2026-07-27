// Deferred lighting over the terrain G-buffer, plus the tone-mapping composite.
//
// Lighting is deferred because the shadow and occlusion terms are screen-space: both need the whole
// depth buffer resolved before any pixel can be lit, which a forward pass cannot provide.

struct DirectionalLight {
    ambient: vec4<f32>,
    diffuse: vec4<f32>,
    // Unit direction the light travels *along*, so a receiver is lit by `-source_direction`.
    // A zero vector marks an unused slot.
    source_direction: vec4<f32>,
}

/// The number of light slots the pass reads. Slot 0 is the primary and is the only shadowed one; the
/// rest are unshadowed fills standing in for sky and bounce until an irradiance probe exists.
const LIGHT_COUNT: i32 = 3;

struct SceneCamera {
    view_projection: mat4x4<f32>,
    // Inverse of `view_projection`, for reconstructing a pixel's world position from scene depth.
    inverse_view_projection: mat4x4<f32>,
    // xyz camera position, w unused.
    camera_position: vec4<f32>,
    // xy viewport size in pixels, zw its reciprocal.
    viewport: vec4<f32>,
    lights: array<DirectionalLight, 3>,
}

// `params` packs the world units spanned by the full normalized depth range in `y` and the world
// units covered by one shadow texel in `z`. `x` and `w` are reserved. Both scales are per cascade,
// since the fitted frusta differ by more than an order of magnitude.
struct ShadowCascade {
    view_projection: mat4x4<f32>,
    params: vec4<f32>,
}

/// Four cascades rather than more. An RTS camera has a bounded height range, so the depth interval
/// needing shadows is far narrower than a free-flight camera's, and a fifth cascade would fit a
/// frustum slice the camera cannot reach.
const SHADOW_CASCADE_COUNT: i32 = 4;

struct ShadowCamera {
    cascades: array<ShadowCascade, 4>,
}

struct FullscreenOutput {
    @builtin(position) position: vec4<f32>,
}

/// A single oversized triangle covering the viewport, so no vertex buffer is needed.
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
// Geometry coverage in `r`: below 0.5 no geometry was drawn, 1.0 is opaque geometry, and anything
// above 1.0 carries that much emissive strength.
@group(0) @binding(2) var g_coverage: texture_2d<f32>;
@group(0) @binding(3) var<uniform> camera: SceneCamera;
@group(0) @binding(4) var primary_shadow: texture_depth_2d_array;
@group(0) @binding(5) var primary_shadow_sampler: sampler_comparison;
@group(0) @binding(6) var<uniform> shadow_camera: ShadowCamera;
@group(0) @binding(7) var ambient_occlusion: texture_2d<f32>;
// The scene depth buffer, read directly. There is no multisample resolve step: this pass is not
// multisampled, so the depth attachment the G-buffer wrote is sampleable as-is.
@group(0) @binding(8) var scene_depth: texture_depth_2d;

// Reconstructs a G-buffer pixel's world position from its depth.
//
// The G-buffer used to carry world position in an `Rgba16Float` target, and a half float has ten
// mantissa bits: past 1024 world units the representable step is a whole unit, and past 2048 it is
// two. On a map a few thousand units across that snapped every receiver onto a lattice, and because
// the shadow map holds smooth depth, roughly half of each lattice cell projected behind its own
// stored depth -- striped self-shadowing across the whole terrain that no bias or filter setting
// could reach, since the error was an order of magnitude larger than a shadow texel.
//
// Depth carries the same information without that loss. At this projection's 1.0 near plane the
// reconstruction error is about `distance^2 * 6e-8` world units: five thousandths of a unit at 300
// units out and a seventh of a unit at the far edge of the shadowed range, against the one-to-two
// units the old target lost everywhere. It also costs less bandwidth than it saves, because the
// depth already had to be resolved for the forward passes.
fn world_from_depth(pixel: vec2<i32>, depth: f32) -> vec3<f32> {
    // `viewport.zw` holds the reciprocal viewport, and clip space is y-up while pixels are y-down.
    let uv = (vec2<f32>(pixel) + vec2<f32>(0.5)) * camera.viewport.zw;
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let homogeneous = camera.inverse_view_projection * vec4<f32>(ndc, depth, 1.0);
    return homogeneous.xyz / homogeneous.w;
}

fn world_at(pixel: vec2<i32>) -> vec3<f32> {
    return world_from_depth(pixel, textureLoad(scene_depth, pixel, 0));
}

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
// fill light contributes its own unoccluded ambient, so leaving ambient whole in
// shade caps achievable shadow contrast around ten percent and cast shadows read as absent.
const SHADOW_DIRECT_FLOOR: f32 = 0.08;
// Lowered from a larger value once the ambient term itself grew: the floor is a *fraction*, so a
// generous fraction of a realistic skylight ambient leaves shadows nearly as bright as open ground.
// The two have to be tuned together, which is the trap this constant sits in.
const SHADOW_AMBIENT_FLOOR: f32 = 0.32;

// Ambient occlusion attenuates every light's ambient share, including the accent fills the sun's
// shadow deliberately leaves alone: occlusion answers "how much of the sky can this point see",
// which the directional shadow cannot, since that cannot tell a shadowed open field from an
// enclosed corner. Clamped rather than applied whole so interiors darken without going black.
const AO_AMBIENT_FLOOR: f32 = 0.25;

// The two attenuations above overlap rather than compose. A point the sun cannot reach because a
// hill stands in the way is usually also a point that sees less sky, so multiplying both floors
// charges twice for one occluder, and the product bottoms out several times deeper than either
// term alone ever reaches.
//
// Taking the deeper of the two instead of their product keeps whichever term has the better claim on
// a given pixel and stops the other adding to it. The floors themselves are unchanged, so the worst
// case a single term can produce is exactly what it was before.
fn primary_ambient_scale(shadow_visibility: f32, occlusion: f32) -> f32 {
    return min(mix(SHADOW_AMBIENT_FLOOR, 1.0, shadow_visibility), occlusion);
}

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

// Incidence below which a receiver stops being shadowed at all.
//
// A surface facing away from a light cannot be meaningfully shadowed by it, and the depth comparison
// there is least trustworthy: on a face nearly parallel to the light, the far side of a solid is
// laterally rather than deeply offset, so the near and far depths fall within a texel of each other and
// the test flickers. The direct term hides that, being near zero at such incidence -- but the *ambient*
// term is shadow-attenuated too and does not depend on incidence, so the flicker surfaces there as
// diagonal striping across the face.
//
// Fading the attenuation out as incidence approaches zero removes it at its cause rather than by
// inflating a bias, and costs nothing that was visible: the geometry it stops shadowing receives no
// direct light anyway.
const SHADOW_INCIDENCE_FADE: f32 = 0.22;

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
    let coverage = textureLoad(g_coverage, pixel, 0).r;
    if (coverage < 0.5) {
        let horizon = clamp(input.position.y / camera.viewport.y, 0.0, 1.0);
        return vec4<f32>(mix(vec3<f32>(0.025, 0.04, 0.065), vec3<f32>(0.12, 0.20, 0.30), horizon), 1.0);
    }
    let world = world_at(pixel);
    let albedo = textureLoad(g_albedo, pixel, 0).rgb;
    let normal_roughness = textureLoad(g_normal, pixel, 0);
    let normal = normalize(normal_roughness.xyz);
    let view_direction = normalize(camera.camera_position.xyz - world);
    var primary_visibility = shadow_visibility(world, normal);
    // Fade toward fully lit at grazing incidence to the primary light. See SHADOW_INCIDENCE_FADE.
    let primary = camera.lights[0].source_direction.xyz;
    let primary_length = length(primary);
    if (primary_length > 0.00001) {
        let incidence = dot(normal, -primary / primary_length);
        let trust = smoothstep(0.0, SHADOW_INCIDENCE_FADE, incidence);
        primary_visibility = mix(1.0, primary_visibility, trust);
    }
    let occlusion = mix(
        AO_AMBIENT_FLOOR,
        1.0,
        textureLoad(ambient_occlusion, pixel, 0).r
    );
    var color = vec3<f32>(0.0);
    for (var index = 0; index < LIGHT_COUNT; index += 1) {
        let light = camera.lights[index];
        // Slot 0 is the primary and is the only shadowed light, in both its ambient and direct
        // shares. The rest are fills: shadowing them would darken the scene twice for one
        // occluder and defeat the purpose of having them.
        let shadowed = index == 0;
        // Only the primary light is both shadowed and occluded, so it is the only one where the two
        // could compound; the accent fills take occlusion alone and are unaffected by this.
        let ambient_scale = select(
            occlusion,
            primary_ambient_scale(primary_visibility, occlusion),
            shadowed
        );
        color += albedo * light.ambient.rgb * ambient_scale;
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
            // material has no highlight at all. A fixed strength instead gives every surface a
            // sheen regardless of what its material declared, once per light.
            let half_vector = normalize(light_direction + view_direction);
            let specular = pow(
                max(dot(normal, half_vector), 0.0),
                mix(64.0, 8.0, normal_roughness.w)
            );
            let specular_strength = SPECULAR_STRENGTH * (1.0 - normal_roughness.w);
            color += light.diffuse.rgb * specular * specular_strength * visibility;
        }
    }
    // Self-illumination, decoded from the G-buffer coverage channel. Added after the light loop so
    // it survives full shade, which is the whole point of a lamp: the emitted term takes its hue
    // from the material's own albedo, and the intensity is the material's emissive strength.
    color += albedo * max(coverage - 1.0, 0.0);
    return vec4<f32>(color, 1.0);
}

@group(1) @binding(0) var scene_color: texture_2d<f32>;
@group(1) @binding(1) var scene_sampler: sampler;

// Scene exposure, applied before the tone curve.
//
// Reinhard maps 1.0 to 0.5, so an unexposed scene whose brightest surfaces sit near unity lands
// entirely in the lower half of the range and reads as flat and grey -- the same mistake as tone
// mapping the forward pass at all. Exposing first puts fully lit ground near the top of the range and
// leaves the curve doing what it is for: rolling off the values that genuinely exceed one.
const EXPOSURE: f32 = 1.6;

fn reinhard(hdr: vec3<f32>) -> vec3<f32> {
    let exposed = hdr * EXPOSURE;
    return exposed / (vec3<f32>(1.0) + exposed);
}

// A contrast-adaptive sharpen in the spirit of AMD FidelityFX CAS: it boosts an unsharp-mask
// style detail term by an amount that scales down toward zero both near luminance extremes
// (avoids blooming/crushing) and at genuinely hard edges (avoids ringing on silhouette
// edges), so it only restores softer mid-contrast detail lost to texture filtering. It is not an
// antialiasing pass and does not pretend to be one.
const SHARPEN_STRENGTH: f32 = 0.6;

@fragment
fn composite_fragment(input: FullscreenOutput) -> @location(0) vec4<f32> {
    let inverse_viewport = camera.viewport.zw;
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
