struct ShadowCamera {
    view_projection: mat4x4<f32>,
    time: vec4<f32>,
}

@group(0) @binding(0) var<uniform> shadow_camera: ShadowCamera;

@vertex
fn terrain_shadow(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    return shadow_camera.view_projection * vec4<f32>(position, 1.0);
}
