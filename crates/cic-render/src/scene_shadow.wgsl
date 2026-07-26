struct Material {
    values: vec4<f32>,
}

// `params` packs the presentation time in `x`, the world units spanned by the full normalized
// depth range in `y`, and the world units covered by one shadow texel in `z`. `w` is reserved.
struct ShadowCamera {
    view_projection: mat4x4<f32>,
    params: vec4<f32>,
}

@group(0) @binding(0) var base_color_texture: texture_2d<f32>;
@group(0) @binding(1) var base_color_sampler: sampler;
@group(0) @binding(2) var<uniform> material: Material;
@group(1) @binding(0) var<uniform> shadow_camera: ShadowCamera;

struct ShadowOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) texcoord: vec2<f32>,
}

struct SceneryInput {
    @location(0) position: vec3<f32>,
    @location(3) texcoord: vec2<f32>,
    @location(4) transform_row_0: vec4<f32>,
    @location(5) transform_row_1: vec4<f32>,
    @location(6) transform_row_2: vec4<f32>,
    @location(7) tree_sway_0: vec4<f32>,
    @location(8) tree_sway_1: vec4<f32>,
}

@vertex
fn scenery_shadow(input: SceneryInput) -> ShadowOutput {
    var local_position = input.position;
    if input.tree_sway_1.w > 0.5 {
        let period = max(input.tree_sway_1.x, 0.001);
        let phase = 6.28318530718 * shadow_camera.params.x / period * input.tree_sway_1.y;
        let angle = input.tree_sway_0.z + input.tree_sway_0.w * cos(phase);
        let sway = vec3<f32>(
            input.tree_sway_0.x * sin(angle),
            input.tree_sway_0.y * sin(angle),
            cos(angle) - 1.0
        ) * input.tree_sway_1.z;
        local_position += sway * max(input.position.z, 0.0);
    }
    let local = vec4<f32>(local_position, 1.0);
    let world = vec3<f32>(
        dot(input.transform_row_0, local),
        dot(input.transform_row_1, local),
        dot(input.transform_row_2, local)
    );
    var output: ShadowOutput;
    output.position = shadow_camera.view_projection * vec4<f32>(world, 1.0);
    output.texcoord = input.texcoord;
    return output;
}

// Stable per-texel hash. Derived only from the fragment position, so the pattern never varies
// between frames and the shadows it produces cannot shimmer.
fn shadow_alpha_hash(position: vec2<f32>) -> f32 {
    var hash = (u32(position.x) * 73856093u) ^ (u32(position.y) * 19349663u);
    hash ^= hash >> 13u;
    hash *= 1274126177u;
    return f32(hash & 0xFFFFu) / 65536.0;
}

@fragment
fn scenery_shadow_fragment(input: ShadowOutput) {
    let alpha = textureSample(base_color_texture, base_color_sampler, input.texcoord).a;
    let cutoff = material.values.x;
    if cutoff >= 0.5 {
        // The material declares an alpha test, so reproduce the hard cutout it asked for.
        if alpha < cutoff {
            discard;
        }
        return;
    }
    // Everything else is either opaque or blended, and a blended card has no meaningful cutoff:
    // tested against a near-zero one it writes depth across its whole surface, so a translucent
    // tree canopy casts a fully opaque silhouette. Comparing against a per-texel hash instead makes
    // a texel of opacity `a` occlude about `a` of the shadow samples, which the receiver's existing
    // PCF resolves into genuine partial shadow rather than a solid blob. Opaque surfaces are
    // unaffected: an alpha of one always wins, and an alpha of zero always loses, as before.
    if alpha < shadow_alpha_hash(floor(input.position.xy)) {
        discard;
    }
}
