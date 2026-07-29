// Cascade selection and the shadow term.
//
// A composition chunk. Requires `scene.wgsl` for the camera and the world reconstruction.
//
// This exists as a chunk rather than as part of the lighting pass because two passes need it -- the
// deferred lighting resolve, which shades terrain and models out of the G-buffer, and the water
// surface -- and duplicating the bias, the depth slack, the blend band and the incidence fade across
// them is exactly the drift the composition step was added to prevent.
//
// # Why this chunk owns group 2
//
// Everything a cascaded shadow map needs is declared here rather than in `scene.wgsl`: the cascade
// structs, the count, the depth array, its comparison sampler and the uniform. `shadow_visibility` is
// then the *whole* interface a pass sees, and the technique behind it -- the resources as much as the
// arithmetic -- is replaceable without touching a binding any other pass depends on. A provider that
// ray-traced the term instead would declare its acceleration structure in this group, implement the
// same signature, and be substituted by naming a different chunk in `shader.rs`; nothing in group 0
// would move, and the composite and the two antialias resolves would not notice.
//
// Group 2 and not 1 because group 1 is where a pass keeps its *own* resources -- the water uniform,
// the composite's sampled image, the history layers -- and those are per-pass while this is shared by
// two. The lighting pipeline therefore declares a hole at group 1, which `wgpu` allows.

// `params` packs the world units spanned by the full normalized depth range in `y` and the world
// units covered by one shadow texel in `z`. `x` and `w` are reserved. Both scales are per cascade,
// since the fitted frusta differ by more than an order of magnitude.
struct ShadowCascade {
    view_projection: mat4x4<f32>,
    params: vec4<f32>,
}

// Four cascades rather than more. An RTS camera has a bounded height range, so the depth interval
// needing shadows is far narrower than a free-flight camera's, and a fifth cascade would fit a
// frustum slice the camera cannot reach.
const SHADOW_CASCADE_COUNT: i32 = 4;

struct ShadowCamera {
    cascades: array<ShadowCascade, 4>,
}

@group(2) @binding(0) var primary_shadow: texture_depth_2d_array;
@group(2) @binding(1) var primary_shadow_sampler: sampler_comparison;
@group(2) @binding(2) var<uniform> shadow_camera: ShadowCamera;

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
