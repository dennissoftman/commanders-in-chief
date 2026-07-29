// The sRGB transfer function, in both directions.
//
// # Why three passes need it in a shader rather than in a sampler
//
// A composed terrain page is stored as `rgba8unorm`, because WebGPU has no sRGB *storage* format and a
// storage binding is what a compute shader writes through. That is not a format substitution that can be
// waved through: the layer blend is in linear light, and eight bits of linear is visibly banded in the
// darks, which is the entire reason sRGB storage exists. So the encode moves out of the sampler and into
// whoever writes a page, and the decode into whoever reads one.
//
// Three passes are on that route — the compose pass encodes, the reduce pass decodes and re-encodes around
// its average, and the G-buffer decodes — and a second copy of a transfer function is the kind of
// duplication that drifts without failing to compile. One chunk, three programs.

/// Linear light from an sRGB-encoded value.
fn linear_from_srgb(value: vec3<f32>) -> vec3<f32> {
    let low = value / 12.92;
    let high = pow((value + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(high, low, value <= vec3<f32>(0.04045));
}

/// An sRGB-encoded value from linear light, clamped for an eight-bit store.
///
/// The direction is worth stating because getting it backwards is invisible in a unit test and obvious in a
/// frame: a page's layer albedo is sampled through an sRGB texture, so the blend that feeds this is
/// *linear*, and this is the encode on the way into the store rather than a decode of something already
/// encoded.
fn srgb_from_linear(value: vec3<f32>) -> vec3<f32> {
    let low = value * 12.92;
    let high = 1.055 * pow(max(value, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return clamp(
        select(high, low, value <= vec3<f32>(0.0031308)),
        vec3<f32>(0.0),
        vec3<f32>(1.0),
    );
}
