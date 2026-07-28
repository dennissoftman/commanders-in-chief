// The sky, and later the fog and cloud terms that share its reasoning.
//
// A composition chunk. Requires nothing but is required by anything that shades an outdoor surface.

// The sky, as the two colours everything that needs one fades between.
//
// Named constants rather than literals at the point of use, because the water pass reflects this same
// sky. A reflection of a sky that is not the sky on screen is obvious in a screenshot and invisible
// from any assertion, so the two are made incapable of disagreeing rather than kept in step by hand.
const SKY_ZENITH: vec3<f32> = vec3<f32>(0.025, 0.04, 0.065);
const SKY_HORIZON: vec3<f32> = vec3<f32>(0.12, 0.20, 0.30);

// The sky along a world direction, for a reflection. The screen gradient below is the same two colours
// mixed by height in the frame instead, which is all a background needs.
fn sky_colour(direction: vec3<f32>) -> vec3<f32> {
    return mix(SKY_HORIZON, SKY_ZENITH, clamp(direction.z, 0.0, 1.0));
}
