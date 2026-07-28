// Tone mapping, the resolution downsample, and the contrast-adaptive sharpen.
//
// A composition chunk. Requires `scene.wgsl` for the fullscreen vertex stage and the two sizes.
//
// # Why this pass is where the resolution scale is resolved
//
// It is the only pass that reads the scene through a *filtering* sampler rather than loading texels by
// integer coordinate, and it is the last one before the image leaves the chain. So it is already doing
// the work: every fragment here is one pixel of the caller's target, and its `uv` addresses the HDR
// target in normalized coordinates whatever size that target happens to be. At a scale of two an output
// pixel's centre falls exactly on the corner shared by four render texels, so the bilinear read returns
// their exact average; at other ratios it is an approximation of the box filter, which is the standard
// trade and costs one tap instead of a variable-width kernel.
//
// The sharpen below runs at *output* resolution for the same reason it exists — it is restoring detail
// for a viewer, and its neighbourhood should be the pixels that viewer will see. Sharpening the render
// resolution before the downsample would put the boost into detail the downsample then averages away.

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
//
// This is the strength at a resolution scale of one. Above that it is divided by the scale — see
// `sharpen_strength`.
const SHARPEN_STRENGTH: f32 = 0.6;

// The sharpen strength for the scale this frame is rendering at: full below a scale of one, and off
// above it.
//
// The sharpen is a correction for *magnification*. It restores mid-contrast detail lost to texture
// filtering, and that loss is a function of how much texture footprint one output pixel covers — which a
// render larger than the output shrinks, selecting finer mip levels and then averaging several honest
// samples into each pixel instead of stretching one.
//
// Above a scale of one it does more than nothing, though: it works against the setting. Its amplitude
// deliberately backs off at hard edges to avoid ringing on silhouettes and rises on soft ones — and a
// supersampled silhouette is soft *by construction*, being genuine sub-pixel coverage the downsample
// just produced. Boosting those rebuilds the staircase the extra pixels were bought to remove.
//
// Measured on the shadowing-terrain fixture, over the two-pixel band along the sky boundary: a 1.5x
// render carried 34% more pixel-to-pixel step energy than a native one with the sharpen still active,
// and 4% more with it off. Halving it was tried first and left most of the excess in place, because the
// amplitude term is not a scalar the scale can compensate for — it is a rule that a soft edge wants
// sharpening, which stops being true the moment the softness is the antialiasing.
//
// Below one the strength is unchanged rather than raised. An upscale does lose more detail, but it has
// none left to find, and sharpening a magnified image amplifies the interpolation rather than the scene.
// At exactly one this returns the constant, so every committed reference stays byte-identical.
fn sharpen_strength() -> f32 {
    let scale = camera.viewport.x * camera.output.z;
    return select(0.0, SHARPEN_STRENGTH, scale <= 1.0);
}

@fragment
fn composite_fragment(input: FullscreenOutput) -> @location(0) vec4<f32> {
    // The output size, not the render size. This fragment is one pixel of the caller's target, and the
    // sampler resolves whatever ratio there is between that and the HDR target it reads.
    let inverse_viewport = camera.output.zw;
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
    let amplitude = sqrt(clamp(peak, vec3<f32>(0.0), vec3<f32>(1.0))) * sharpen_strength();
    let neighbor_average = (north + south + west + east) * 0.25;
    let sharpened = center + (center - neighbor_average) * amplitude;
    return vec4<f32>(clamp(sharpened, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
