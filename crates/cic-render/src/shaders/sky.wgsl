// What is behind everything: an analytic gradient, or a captured environment when one is bound.
//
// A composition chunk. Requires `scene.wgsl` for the camera and the depth reconstruction, and is
// required by `atmosphere.wgsl` -- which takes TAU and the fog's horizon reasoning from here -- and by
// `reflection.wgsl`, which asks this what a mirror sees.
//
// # Why this is a provider group rather than two more bindings on the scene
//
// Group 0 is the G-buffer and the camera, and five programs bind it whether they read every entry or
// not. Putting an environment texture there would hand the composite and both antialias resolves a sky
// none of them samples, which is the mistake the shadow cascades already made and were moved out of for.
// So the sky is group 3, declared here, bound by the two passes that ask a direction what colour it is:
// the lighting resolve, for the background and for a metal's reflection, and the water pass.
//
// A group is always bound, because a pipeline layout is fixed at creation. With no environment loaded
// the texture is one texel and `sky.params.z` is zero, and every function below takes the analytic
// branch -- which is not a fallback bolted on but the renderer's original sky, moved here unchanged.

const TAU: f32 = 6.2831853;
const PI: f32 = 3.14159265;

// The analytic sky, as the two colours everything that needs one fades between.
//
// Named constants rather than literals at the point of use, because the water pass reflects this same
// sky. A reflection of a sky that is not the sky on screen is obvious in a screenshot and invisible
// from any assertion, so the two are made incapable of disagreeing rather than kept in step by hand.
const SKY_ZENITH: vec3<f32> = vec3<f32>(0.025, 0.04, 0.065);
const SKY_HORIZON: vec3<f32> = vec3<f32>(0.12, 0.20, 0.30);

struct SkyEnvironment {
    // x a multiplier on the stored radiance, y the yaw in radians, z 1.0 when an image is bound and
    // 0.0 when the analytic sky is in force, w the highest mip level the image has.
    params: vec4<f32>,
    // x radians of longitude one base-level texel spans, y and zw reserved.
    //
    // Here rather than derived from `textureDimensions` because it is what turns an angle into a mip
    // level, and that conversion is the whole of `sky_reflection`.
    scale: vec4<f32>,
}

@group(3) @binding(0) var sky_image: texture_2d<f32>;
@group(3) @binding(1) var sky_sampler: sampler;
@group(3) @binding(2) var<uniform> sky: SkyEnvironment;

fn sky_is_captured() -> bool {
    return sky.params.z >= 0.5;
}

// Where a world direction lands in an equirectangular image.
//
// Longitude around the world's vertical axis into `u`, polar angle from straight up into `v`, which is
// the projection every HDRI is distributed in. `v` runs 0 at the zenith to 1 at the nadir because the
// image's first row is the top, and a reader that flips it renders the ground overhead -- a mistake that
// looks like a lighting fault rather than a coordinate one.
//
// The yaw is added to the longitude rather than applied as a rotation to the direction, which is the
// same thing for a rotation about the vertical axis and one addition instead of a matrix. `u` may leave
// `0..1` doing it; the sampler repeats in that axis precisely so it can.
fn sky_direction_uv(direction: vec3<f32>) -> vec2<f32> {
    let unit = normalize(direction);
    let longitude = atan2(unit.y, unit.x) + sky.params.y;
    return vec2<f32>(
        longitude / TAU + 0.5,
        acos(clamp(unit.z, -1.0, 1.0)) / PI
    );
}

// The captured environment's radiance along a direction, at an explicit mip level.
//
// `textureSampleLevel` rather than `textureSample`, and the level is always stated rather than derived,
// for two independent reasons that happen to want the same call.
//
// **The meridian.** `u` is built from `atan2`, which jumps by a full turn where the seam falls. Hardware
// mip selection reads the screen-space derivative of the coordinate, so along that one column it sees a
// derivative of about one *whole texture* and picks the smallest level -- a vertical line of 1x1 grey
// down the sky, in a place no amount of filtering quality would fix.
//
// **Non-uniform control flow.** The water pass discards, and a derivative taken after a discard is
// undefined. An explicit level is defined everywhere.
fn sky_radiance(direction: vec3<f32>, level: f32) -> vec3<f32> {
    let uv = sky_direction_uv(direction);
    let clamped = clamp(level, 0.0, max(sky.params.w, 0.0));
    return textureSampleLevel(sky_image, sky_sampler, uv, clamped).rgb * sky.params.x;
}

// The sky along a world direction, for a reflection.
//
// The analytic branch is the screen gradient's two colours mixed by the direction's height instead,
// which is all a reflection ever had.
fn sky_colour(direction: vec3<f32>) -> vec3<f32> {
    if (sky_is_captured()) {
        return sky_radiance(direction, 0.0);
    }
    return mix(SKY_HORIZON, SKY_ZENITH, clamp(direction.z, 0.0, 1.0));
}

// The sky averaged over a cone of directions, for a surface that reflects into more than one.
//
// `cone` is the half-angle in radians that one pixel's reflection actually spans. A mirror passes zero
// and gets the sharp lookup; anything else states how wide its lobe is and gets the mip level whose
// texels are that size, which is what a mip chain is for.
//
// # Why this takes an angle rather than a roughness
//
// Because roughness is not the only thing that widens a reflection, and on water it is not even the
// larger one. A lake's material roughness is small — that is why it mirrors — but its *surface slope*
// is not, and a pixel covering several wave crests reflects into every direction those crests face.
// Against the analytic sky none of this mattered: that sky is a gradient in one variable, so averaging
// over a cone of it gives almost exactly its value at the centre. Against a captured environment,
// ignoring the cone renders a lake as **coloured speckle** — neighbouring pixels take single texels
// ten degrees apart, one from the orange horizon and the next from the blue zenith. It looked like a
// sampling bug in the water pass and was a missing convolution.
//
// So the caller computes its own cone from whatever widens it, and this converts an angle to a level.
//
// # What this is not
//
// Not a prefiltered environment. The chain was built by halving the image, not by convolving it with a
// reflection lobe, so it is a plausible blur of the right *width* rather than the right *shape*, and an
// equirectangular halving spans a much wider solid angle near the poles than at the horizon. Both are
// reasons a later prefilter would replace this rather than tune it.
fn sky_reflection(direction: vec3<f32>, cone: f32) -> vec3<f32> {
    if (!sky_is_captured()) {
        return mix(SKY_HORIZON, SKY_ZENITH, clamp(direction.z, 0.0, 1.0));
    }
    // How many base-level texels the cone spans, as a mip level. Floored at one texel, since a cone
    // narrower than a texel cannot ask for less than the base image.
    let texels = max(cone / max(sky.scale.x, 1.0e-6), 1.0);
    return sky_radiance(direction, log2(texels));
}

// The direction a pixel's view ray travels, in world space.
//
// Reconstructed through the same inverse view-projection every other pass uses, at the far plane. That
// matters more than it sounds: the matrix carries the temporal resolve's sub-pixel jitter, so the sky
// is sampled at the same offset as the geometry in front of it and the accumulation converges instead
// of fighting a background that never moved.
fn sky_view_direction(pixel: vec2<i32>) -> vec3<f32> {
    return normalize(world_from_depth(pixel, 1.0) - camera.camera_position.xyz);
}

// What a pixel with no geometry behind it shows.
//
// **The analytic branch is a screen-space gradient and stays one.** It runs from the zenith colour at
// the top of the frame to the horizon colour at the bottom, and so does not respond to where the camera
// is pointing -- pitching up does not bring more zenith into view. That is a real limitation and it is
// kept deliberately: every committed reference capture was rendered through this expression, and
// changing it to a direction lookup would alter all of them at once, which would destroy the evidence
// that binding an environment changed nothing else. An environment is the answer for a scene that needs
// a sky that turns with the camera, and it is exactly the branch below.
fn sky_background(position: vec2<f32>) -> vec3<f32> {
    if (sky_is_captured()) {
        return sky_radiance(sky_view_direction(vec2<i32>(position)), 0.0);
    }
    // Pixel y grows downward, so this runs from the zenith at the top of the frame to the horizon at
    // the bottom.
    let horizon = clamp(position.y / camera.viewport.y, 0.0, 1.0);
    return mix(SKY_ZENITH, SKY_HORIZON, horizon);
}
