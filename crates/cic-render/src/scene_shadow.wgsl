struct Material {
    values: vec4<f32>,
}

struct ShadowCamera {
    view_projection: mat4x4<f32>,
    time: vec4<f32>,
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
        let phase = 6.28318530718 * shadow_camera.time.x / period * input.tree_sway_1.y;
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

@fragment
fn scenery_shadow_fragment(input: ShadowOutput) {
    let alpha = textureSample(base_color_texture, base_color_sampler, input.texcoord).a;
    if alpha < max(material.values.x, 0.0039) {
        discard;
    }
}
