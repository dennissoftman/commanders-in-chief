// `params` packs the presentation time in `x`, the world units spanned by the full normalized
// depth range in `y`, and the world units covered by one shadow texel in `z`. `w` is reserved.
struct ShadowCamera {
    view_projection: mat4x4<f32>,
    params: vec4<f32>,
}

@group(0) @binding(0) var<uniform> shadow_camera: ShadowCamera;

@vertex
fn terrain_shadow(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    return shadow_camera.view_projection * vec4<f32>(position, 1.0);
}
