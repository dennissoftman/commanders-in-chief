// Screen-space motion, shared by every pass that writes the motion target.
//
// A composition chunk. Declares one function and the reasoning behind it, and nothing else, so it composes
// ahead of both G-buffer programs.
//
// # What a motion vector is here
//
// The offset, in texture coordinates, from a fragment to where the same surface point sat in the previous
// frame. So the temporal resolve reads its history at `uv + motion` — an addition, with no sign to get
// wrong at the call site.
//
// # Why the jitter is subtracted
//
// The rasterized position is jittered by construction; that is the whole mechanism. A motion vector built
// from it would carry the sub-pixel shake, and the resolve would then sample its history offset by exactly
// the thing it is averaging out — so the accumulation would never converge and a static scene would shimmer
// at the jitter period. Removing it needs no second matrix: a sub-pixel jitter is a clip-space translation
// proportional to `w`, so `clip.xy - jitter * clip.w` recovers the unjittered position exactly.
//
// The previous position is already unjittered, because the previous view-projection is stored without one.

/// Texture-coordinate motion from a fragment's current and previous clip positions.
///
/// Both are perspective-divided here rather than in the vertex stage, because interpolating a divided
/// position across a triangle is wrong wherever the triangle is not parallel to the screen — the error is
/// largest on exactly the ground planes an RTS camera looks along.
fn motion_vector(current_clip: vec4<f32>, previous_clip: vec4<f32>, jitter: vec2<f32>) -> vec2<f32> {
    // A fragment behind the eye has no previous screen position worth reporting. Zero says "did not move",
    // which the resolve reads as "sample the history here" -- and its neighbourhood clamp is what stops that
    // being wrong.
    if (current_clip.w <= 0.0001 || previous_clip.w <= 0.0001) {
        return vec2<f32>(0.0);
    }
    let current = (current_clip.xy - jitter * current_clip.w) / current_clip.w;
    let previous = previous_clip.xy / previous_clip.w;
    // Normalized device coordinates are y-up and texture coordinates are y-down, so the vertical component
    // changes sign while the horizontal one does not. Halved because the device range is two units wide and
    // the texture range is one.
    return vec2<f32>(
        (previous.x - current.x) * 0.5,
        (current.y - previous.y) * 0.5,
    );
}
