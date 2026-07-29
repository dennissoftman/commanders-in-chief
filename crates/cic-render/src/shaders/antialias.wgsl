// A luma-gated, edge-oriented blend over the tone-mapped image.
//
// A composition chunk. Requires `scene.wgsl` for the fullscreen vertex stage and the output size.
//
// # Provenance
//
// This is FXAA in the sense that "FXAA" now names a technique — a single post pass that finds edges by
// luminance and blends across them — and it is not a port of anybody's implementation. Timothy Lottes'
// `fxaa3_11.h` was not consulted, and neither was any derivative of it. What follows was derived from
// the problem: the construction of the step detector, the orientation test, the blend weight, and every
// constant here are this file's own. See LICENSING.md for why that distinction is tracked at all.
//
// # What it does, and in what order
//
// 1. Reads a 3x3 neighbourhood and takes a perceptual luma of each tap.
// 2. Leaves the pixel alone unless the neighbourhood's luma range clears a threshold with both an
//    absolute and a relative term.
// 3. Decides whether the edge runs closer to horizontal or to vertical, with a Sobel pair.
// 4. Weighs how much the pixel looks like a *step* rather than a point on a smooth ramp.
// 5. Blends it toward the average of its two neighbours across the edge, by that weight.
//
// # What it cannot do
//
// It has no search along the edge, so it does not know how far the staircase it is standing on runs.
// That bounds how much of a long, shallow silhouette it can recover: it softens each step rather than
// reconstructing the line. It also cannot tell a one-pixel bright feature from a one-pixel aliasing
// artifact, because at that size there is no difference in the image — a genuinely thin highlight is
// dimmed along with the sparkle. Both are the price of the cost class, and the answer to both is TAA,
// which is a quality tier and not a fix for this pass.

@group(1) @binding(0) var resolved_color: texture_2d<f32>;
@group(1) @binding(1) var resolved_sampler: sampler;

// Below this luma range the neighbourhood is flat enough to leave alone.
//
// An absolute floor, in the perceptual space the luma below is measured in. Its job is to keep the pass
// off gradients — a sky ramp, a lit slope — where a blend does nothing but remove the dithering the rest
// of the chain deliberately put there.
const EDGE_FLOOR: f32 = 0.045;

// The share of the neighbourhood's peak luma that also has to be spanned before an edge is declared.
//
// Contrast is judged against local brightness because vision is: a step of a twentieth reads as an edge
// in shadow and as nothing at all across sunlit ground. Without this term the threshold is either too
// eager in the highlights or blind in the shade, and no single absolute value is both.
const EDGE_RELATIVE: f32 = 0.11;

// How far toward the neighbour average a full step is allowed to move.
//
// Not 1.0. A step pixel is genuinely one side of the edge, and replacing it outright with the average
// moves the silhouette by half a pixel — which reads as the whole image having been nudged, and turns a
// jagged edge into a jagged edge in a different place. Three quarters softens the step while leaving the
// edge where the geometry put it.
const BLEND_LIMIT: f32 = 0.75;

// Rec. 709 luma weights.
fn luma_of(colour: vec3<f32>) -> f32 {
    // Square-rooted, which is a cheap stand-in for the display transfer curve. The texture this pass
    // reads is sRGB, so the hardware has already decoded it to linear on the way in — and an edge
    // detector run on linear values is far more sensitive in the highlights than in the shadows, which
    // is exactly the wrong way round for deciding what a viewer will see as a step.
    return sqrt(dot(colour, vec3<f32>(0.2126, 0.7152, 0.0722)));
}

fn tap(uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(resolved_color, resolved_sampler, uv, 0.0).rgb;
}

// How much of a step this pixel is, given its two neighbours across the edge.
//
// The construction: on a flat surface, and on a *linear* ramp, the centre is the mean of the two
// neighbours and this is zero. On a hard step the centre sits with one neighbour while the other is a
// full range away, so the numerator reaches the range and the result is a half. On a lone pixel
// differing from both — which is what sub-pixel specular glitter is — it reaches the range twice over
// and the result saturates at one.
//
// That ordering is the point. A blend proportional to *contrast* would soften every gradient in the
// frame; a blend proportional to this leaves ramps untouched, halves steps, and hits speckle hardest.
fn step_weight(centre: f32, first: f32, second: f32, range: f32) -> f32 {
    let deviation = abs(2.0 * centre - first - second);
    return clamp(deviation / (2.0 * range), 0.0, 1.0);
}

@fragment
fn antialias_fragment(input: FullscreenOutput) -> @location(0) vec4<f32> {
    // The output size, not the viewport: this pass runs after the composite has already downsampled to
    // the size the caller asked for, so a step here is one pixel of the final image.
    let step_uv = camera.output.zw;
    // `position.xy` is already the pixel *centre* -- the framebuffer coordinate of the top-left pixel is
    // (0.5, 0.5), not (0, 0). So this multiplies straight through, and adding a further half pixel would
    // sample half a pixel down and right of where this fragment is.
    //
    // It did, until a temporal resolve made the error impossible to miss: with the half-pixel offset in
    // place, an accumulation of a *static* frame never reached a fixed point, because each pass read its own
    // history offset from where it had written it and re-filtered it every frame. With the offset removed the
    // sequence is exactly stationary. The same error was in this pass and in the composite, where it cost a
    // half-pixel translation of every frame and, at a resolution scale of one, an average of two texels
    // instead of the single exact texel the downsample is supposed to return. Measured on the deferred
    // fixture: 1.5% of pixels differed by more than two, with a peak channel difference of 154.
    let uv = input.position.xy * step_uv;

    let centre_colour = tap(uv);
    let north_colour = tap(uv + vec2<f32>(0.0, -step_uv.y));
    let south_colour = tap(uv + vec2<f32>(0.0, step_uv.y));
    let west_colour = tap(uv + vec2<f32>(-step_uv.x, 0.0));
    let east_colour = tap(uv + vec2<f32>(step_uv.x, 0.0));

    let centre = luma_of(centre_colour);
    let north = luma_of(north_colour);
    let south = luma_of(south_colour);
    let west = luma_of(west_colour);
    let east = luma_of(east_colour);
    // The diagonals are needed for the orientation test and for the range, but never for the blend —
    // nothing is ever mixed along a diagonal — so only their luma is kept.
    let north_west = luma_of(tap(uv + vec2<f32>(-step_uv.x, -step_uv.y)));
    let north_east = luma_of(tap(uv + vec2<f32>(step_uv.x, -step_uv.y)));
    let south_west = luma_of(tap(uv + vec2<f32>(-step_uv.x, step_uv.y)));
    let south_east = luma_of(tap(uv + vec2<f32>(step_uv.x, step_uv.y)));

    let lowest = min(
        min(min(centre, north), min(south, west)),
        min(min(east, north_west), min(north_east, min(south_west, south_east)))
    );
    let highest = max(
        max(max(centre, north), max(south, west)),
        max(max(east, north_west), max(north_east, max(south_west, south_east)))
    );
    let range = highest - lowest;
    if (range < max(EDGE_FLOOR, highest * EDGE_RELATIVE)) {
        return vec4<f32>(centre_colour, 1.0);
    }

    // Sobel, so the tap directly across the edge counts double and the diagonals contribute at their
    // geometric weight rather than equally. `across_x` is large where luma varies along x, which is an
    // edge running *vertically*; `across_y` is its transpose.
    let across_x = (north_west + 2.0 * west + south_west) - (north_east + 2.0 * east + south_east);
    let across_y = (north_west + 2.0 * north + north_east) - (south_west + 2.0 * south + south_east);

    // The dominant axis, and the pair of neighbours the blend uses. An edge at 45 degrees gives the two
    // gradients equal magnitude and either axis is then partly along the edge rather than across it,
    // which is why a diagonal silhouette softens less than an axis-aligned one.
    var first = north;
    var second = south;
    var first_colour = north_colour;
    var second_colour = south_colour;
    if (abs(across_x) > abs(across_y)) {
        first = west;
        second = east;
        first_colour = west_colour;
        second_colour = east_colour;
    }

    let weight = step_weight(centre, first, second, range) * BLEND_LIMIT;
    let across = (first_colour + second_colour) * 0.5;
    return vec4<f32>(mix(centre_colour, across, weight), 1.0);
}
