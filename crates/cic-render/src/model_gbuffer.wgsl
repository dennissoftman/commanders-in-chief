// Instanced model G-buffer and shadow-depth passes.
//
// Writes the same three G-buffer targets as `terrain_gbuffer.wgsl` and shares its depth attachment, so
// models and terrain occlude each other correctly and the deferred lighting pass cannot tell them
// apart. That is the point of a G-buffer: one lighting pass, however many kinds of geometry wrote into
// it.
//
// The material index arrives per *vertex* rather than as bound state. Every vertex of a primitive
// carries the same index, so a model's primitives concatenate into one buffer pair and the whole model
// draws in a single instanced call. See `model.rs` for what that avoids.

struct Uniforms {
    view_projection: mat4x4<f32>,
    camera_position: vec4<f32>,
    light_direction: vec4<f32>,
    light_ambient: vec4<f32>,
    light_diffuse: vec4<f32>,
    terrain: vec4<f32>,
    layers: vec4<u32>,
    palette: array<vec4<f32>, 8>,
}

/// Base colour in `base_color`; metallic in `factors.x`, roughness in `factors.y`, the base-colour
/// array slice in `factors.z`, and whether that slice holds anything in `factors.w`.
struct Material {
    base_color: vec4<f32>,
    factors: vec4<f32>,
}

// Group 0 is the terrain group, bound already for its view-projection. Models read only the transform
// from it, but sharing the group means one uniform buffer drives every pass in the frame and the
// camera cannot disagree between them.
@group(0) @binding(0) var<uniform> uniforms: Uniforms;

// Group 1 is the shadow cascade and group 2 the materials, kept apart because one module declares
// both and a group/binding pair must be unique within it. The G-buffer pipeline leaves group 1 empty
// and the shadow pipeline leaves group 2 empty -- which the graphics API allows explicitly, its
// pipeline layout taking an optional layout per slot for exactly this.
struct ShadowCascadeView {
    view_projection: mat4x4<f32>,
}

@group(1) @binding(0) var<uniform> cascade: ShadowCascadeView;

@group(2) @binding(0) var<storage, read> materials: array<Material>;
// One slice per image the model carried, in source order. Bound once for the whole model, which is
// what lets every material have its own picture without a bind group change between primitives.
@group(2) @binding(1) var base_color_texture: texture_2d_array<f32>;
@group(2) @binding(2) var base_color_sampler: sampler;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) material: u32,
    // A 4x4 instance transform, one column per attribute: there is no matrix vertex format.
    @location(4) transform_0: vec4<f32>,
    @location(5) transform_1: vec4<f32>,
    @location(6) transform_2: vec4<f32>,
    @location(7) transform_3: vec4<f32>,
    @location(8) tint: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) tint: vec4<f32>,
    // Flat, because a material index must not be interpolated between vertices — a fragment halfway
    // between materials 2 and 5 would read material 3.
    @location(2) @interpolate(flat) material: u32,
    @location(3) uv: vec2<f32>,
}

struct GBufferOutput {
    @location(0) albedo: vec4<f32>,
    @location(1) normal_roughness: vec4<f32>,
    @location(2) coverage: f32,
}

fn instance_transform(input: VertexInput) -> mat4x4<f32> {
    return mat4x4<f32>(
        input.transform_0,
        input.transform_1,
        input.transform_2,
        input.transform_3,
    );
}

/// Transforms a normal by the transform's upper 3x3.
///
/// Exact for rotation and uniform scale, which is all `ModelInstance` can express. A general transform
/// would need the inverse transpose; offering only the correct case is cheaper than computing an
/// inverse per vertex to support a case the API does not allow.
fn instance_normal(input: VertexInput, normal: vec3<f32>) -> vec3<f32> {
    let basis = mat3x3<f32>(
        input.transform_0.xyz,
        input.transform_1.xyz,
        input.transform_2.xyz,
    );
    return normalize(basis * normal);
}

@vertex
fn gbuffer_vertex(input: VertexInput) -> VertexOutput {
    let world = instance_transform(input) * vec4<f32>(input.position, 1.0);
    var output: VertexOutput;
    output.clip_position = uniforms.view_projection * world;
    output.normal = instance_normal(input, input.normal);
    output.tint = input.tint;
    output.material = input.material;
    output.uv = input.uv;
    return output;
}

@fragment
fn gbuffer_fragment(input: VertexOutput) -> GBufferOutput {
    let material = materials[input.material];
    // Sampled whether or not this material has a texture, then discarded by the `select` below if it
    // does not. The branch that looks obviously cheaper is not available: `textureSample` derives its
    // mip level from screen-space derivatives, which exist only in uniform control flow, and the
    // material index varies per fragment. Guarding the call would leave the mip level of every
    // *textured* fragment undefined -- the surfaces that matter, not the ones being skipped.
    let sampled = textureSample(
        base_color_texture,
        base_color_sampler,
        input.uv,
        i32(material.factors.z),
    );
    let base = select(vec3<f32>(1.0), sampled.rgb, material.factors.w > 0.5);
    var output: GBufferOutput;
    output.albedo = vec4<f32>(material.base_color.rgb * base * input.tint.rgb, 1.0);
    // Unlike terrain, a model's normal is not flipped toward the viewer: a model has a genuine inside
    // and outside, and forcing its normals to face the camera would light the interior of a hull as
    // though it were the exterior.
    output.normal_roughness = vec4<f32>(normalize(input.normal), material.factors.y);
    output.coverage = 1.0;
    return output;
}

/// Depth-only pass, rendered once per shadow cascade.
///
/// No fragment stage: models are opaque here, so the depth write is the entire output. Alpha-tested
/// foliage would need one, and that is a reason to add it then rather than to carry the cost now.
@vertex
fn shadow_vertex(input: VertexInput) -> @builtin(position) vec4<f32> {
    let world = instance_transform(input) * vec4<f32>(input.position, 1.0);
    return cascade.view_projection * world;
}
