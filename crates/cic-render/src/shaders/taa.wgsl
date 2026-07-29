// The temporal resolve: accumulate this frame into the history, and clamp what the history is allowed to
// say.
//
// A composition chunk. Requires `scene.wgsl` for the fullscreen vertex stage and the output size.
//
// # What this pass is actually doing
//
// The projection is offset by a different sub-pixel amount each frame, so the sequence of frames is a
// sequence of samples at different positions inside each pixel. Averaging them is a higher sampling rate
// bought over time rather than over area — which is why it addresses every class of aliasing the resolution
// scale does, at a fraction of the cost, and why the whole difficulty is in deciding *which* history pixel
// corresponds to this one.
//
// Two things answer that. The motion target says where each surface point was, so a moving camera or a
// swaying plant is followed rather than smeared. And the neighbourhood clamp below says what the history is
// allowed to claim even when the motion vector is right — because a motion vector describes a surface that
// was visible, and the surface that *is* visible may be a different one that the first was hiding.
//
// # Why the clamp is in YCoCg
//
// The clamp is a box in colour space, and the box's shape decides what survives. In RGB the three axes are
// strongly correlated on real images, so the box is a poor fit to the actual distribution of a
// neighbourhood: it is loose along the diagonal, which is where luminance error lives, and tight across it,
// where a small hue difference gets clipped for no benefit. YCoCg decorrelates luminance from the two
// chroma axes, so the same box is tight where the eye is sensitive and loose where it is not. The transform
// is four adds and two shifts' worth of arithmetic and needs no matrix.

@group(1) @binding(0) var current_color: texture_2d<f32>;
@group(1) @binding(1) var current_sampler: sampler;
// The accumulated result of every previous frame, in a float format. See `HISTORY_FORMAT` in `deferred.rs`
// for why it is not the output format.
//
// A plain 2D view of one layer rather than the whole array. Binding the array would put both layers in the
// sampled usage while one of them is a colour attachment, and a colour attachment is an exclusive usage over
// the layers it covers -- so the pass would be refused. Two bind groups, one per layer, is what the ping-pong
// costs; the alternative was two separate textures, which is the same object count with a looser guarantee
// that the two stay the same size and format.
@group(1) @binding(2) var history_color: texture_2d<f32>;
// Texture-coordinate motion, at *render* resolution. Sampled by coordinate rather than loaded by index, so
// a resolution scale needs no arithmetic here.
@group(1) @binding(3) var motion_vectors: texture_2d<f32>;
@group(1) @binding(4) var<uniform> temporal: TemporalParameters;

struct TemporalParameters {
    // x the share of the result taken from the history, y whether a history exists at all, zw unused.
    //
    // Which layer to read is not here: it selects a *bind group*, so the CPU resolves it when recording the
    // pass rather than the shader resolving it per fragment.
    blend: vec4<f32>,
}

// How much of the result comes from the history when the pixel is not moving.
//
// The equilibrium this sets is what decides both how well the frame converges and how long a stale sample
// lingers. At 0.9 each frame contributes a tenth, so eight jitter phases are averaged with weights that have
// decayed by about 0.43 across one period -- enough that every phase still counts, and little enough that a
// pixel the clamp had to reject recovers in a handful of frames.
//
// Higher was tried in principle and is where temporal blur comes from: at 0.97 a phase from thirty frames
// ago still carries a tenth of its weight, and on this camera thirty frames is a visible pan.
const HISTORY_WEIGHT: f32 = 0.9;

// How much of the history to give up per texture-coordinate unit of motion.
//
// A moving pixel's history is less trustworthy for a reason that is not about the motion vector being wrong:
// the history was itself filtered, so following it re-filters an already-filtered sample and the error
// compounds along the path. Scaled by the *screen* motion rather than the world motion, because that is what
// decides how many bilinear taps the value has been through.
//
// The coefficient is large because the units are: a pixel crossing a tenth of the screen in one frame is
// very fast, and 12.0 takes 0.9 down to 0.3 at a hundredth of the screen -- about ten pixels a frame at this
// resolution, which is a brisk pan.
const MOTION_REJECTION: f32 = 12.0;

/// Luminance and the two chroma differences, which is a rotation of RGB with no matrix.
fn to_ycocg(color: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        0.25 * color.r + 0.5 * color.g + 0.25 * color.b,
        0.5 * color.r - 0.5 * color.b,
        -0.25 * color.r + 0.5 * color.g - 0.25 * color.b,
    );
}

fn from_ycocg(color: vec3<f32>) -> vec3<f32> {
    let luma = color.x;
    let orange = color.y;
    let green = color.z;
    return vec3<f32>(
        luma + orange - green,
        luma + green,
        luma - orange - green,
    );
}

/// The resolve writes the same colour twice: once for the viewer and once for the next frame to read.
///
/// Two attachments rather than a pass followed by a copy. The copy would be a full-resolution read and
/// write of an image the pass already has in registers, and it would sit between the resolve and the
/// present for no purpose. The formats differ — see `HISTORY_FORMAT` — which is exactly what a second
/// attachment allows and a copy would not.
struct TemporalOutput {
    @location(0) presented: vec4<f32>,
    @location(1) history: vec4<f32>,
}

fn resolved(color: vec3<f32>) -> TemporalOutput {
    var output: TemporalOutput;
    output.presented = vec4<f32>(color, 1.0);
    output.history = vec4<f32>(color, 1.0);
    return output;
}

@fragment
fn taa_fragment(input: FullscreenOutput) -> TemporalOutput {
    let inverse_output = camera.output.zw;
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
    let uv = input.position.xy * inverse_output;
    let center = textureSampleLevel(current_color, current_sampler, uv, 0.0).rgb;

    // No history yet: the first frame of a sequence, or the first after a resize rebuilt the targets. The
    // current frame is the whole answer, and saying so explicitly beats blending against whatever the
    // freshly allocated texture happens to hold.
    if (temporal.blend.y < 0.5) {
        return resolved(center);
    }

    let motion = textureSampleLevel(motion_vectors, current_sampler, uv, 0.0).xy;
    let history_uv = uv + motion;
    // Reprojected off screen. There is no history for a surface that was not on screen last frame, and
    // clamping the coordinate instead would stretch the frame's edge inward -- a smear along whichever
    // border the camera is moving away from.
    if (any(history_uv < vec2<f32>(0.0)) || any(history_uv > vec2<f32>(1.0))) {
        return resolved(center);
    }

    // The neighbourhood this frame allows. Nine taps rather than five: the cross alone misses the diagonal,
    // and a one-pixel diagonal feature is exactly what a jittered projection moves in and out of a pixel.
    var minimum = to_ycocg(center);
    var maximum = minimum;
    for (var y = -1; y <= 1; y += 1) {
        for (var x = -1; x <= 1; x += 1) {
            let offset = vec2<f32>(f32(x), f32(y)) * inverse_output;
            let neighbour = to_ycocg(
                textureSampleLevel(current_color, current_sampler, uv + offset, 0.0).rgb
            );
            minimum = min(minimum, neighbour);
            maximum = max(maximum, neighbour);
        }
    }

    let stored = textureSampleLevel(history_color, current_sampler, history_uv, 0.0).rgb;
    // Clamped rather than rejected. A history outside the box is not necessarily wrong about everything --
    // pulling it to the nearest value this frame considers plausible keeps whatever it had right, where
    // discarding it throws away the accumulation and reintroduces the aliasing for that pixel.
    let history = from_ycocg(clamp(to_ycocg(stored), minimum, maximum));

    // Trust falls with screen motion. See MOTION_REJECTION.
    let travelled = length(motion);
    let weight = temporal.blend.x * exp(-MOTION_REJECTION * travelled);
    return resolved(mix(center, history, clamp(weight, 0.0, 1.0)));
}
