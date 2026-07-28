// Tone mapping and the contrast-adaptive sharpen, into the caller's target.
//
// A composition chunk. Requires `scene.wgsl` for the fullscreen vertex stage and the viewport.

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
