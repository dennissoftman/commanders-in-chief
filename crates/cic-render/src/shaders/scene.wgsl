// Scene bindings shared by every pass that consumes the G-buffer.
//
// A composition chunk, not a standalone shader: it declares the uniforms, the G-buffer bindings, the
// fullscreen vertex stage, and the depth-to-world reconstruction, and every program that reads the
// G-buffer starts with it. See `shader.rs` for why the shaders are composed at all.

struct DirectionalLight {
    ambient: vec4<f32>,
    diffuse: vec4<f32>,
    // Unit direction the light travels *along*, so a receiver is lit by `-source_direction`.
    // A zero vector marks an unused slot.
    source_direction: vec4<f32>,
}

/// The number of light slots the pass reads. Slot 0 is the primary and is the only shadowed one; the
/// rest are unshadowed fills standing in for sky and bounce until an irradiance probe exists.
const LIGHT_COUNT: i32 = 3;

struct SceneCamera {
    view_projection: mat4x4<f32>,
    // Inverse of `view_projection`, for reconstructing a pixel's world position from scene depth.
    inverse_view_projection: mat4x4<f32>,
    // xyz camera position, w unused.
    camera_position: vec4<f32>,
    // xy the size the chain *renders* at in pixels, zw its reciprocal.
    //
    // Every G-buffer, occlusion, lighting and water pass is this size, and every pixel coordinate they
    // load or reconstruct from is in it. At a resolution scale other than one it is not the size of the
    // image the caller receives -- that is `output` below.
    viewport: vec4<f32>,
    lights: array<DirectionalLight, 3>,
    // rgb the fog colour, w its density per world unit at the reference elevation.
    //
    // The colour is derived on the CPU from the sky's horizon colour rather than authored separately, so
    // the two cannot disagree. They must not: fog that is a different colour from the sky it fades into
    // puts a visible band along the horizon exactly where the terrain silhouette meets it.
    fog: vec4<f32>,
    // x the height falloff in world units, y the elevation the density is quoted at, zw reserved.
    fog_params: vec4<f32>,
    // x coverage, y world units across one cloud cell, z shadow strength, w edge softness.
    clouds: vec4<f32>,
    // xy the cloud pattern's drift in world units, zw reserved.
    cloud_drift: vec4<f32>,
    // x surface wetness, y lying snow, zw reserved.
    //
    // Both act on the G-buffer in the lighting pass rather than in the shaders that wrote it. Terrain and
    // models arrive there as albedo, normal and roughness, which is precisely what wetness and snow modify,
    // so one implementation covers both — where doing it at the source would mean the same logic in two
    // shaders reading two different uniform blocks.
    weather: vec4<f32>,
    // xy the size of the caller's target in pixels, zw its reciprocal. Equal to `viewport` at a
    // resolution scale of one.
    //
    // Only the two passes downstream of the scene read this: the composite, whose filtered read of the
    // HDR target *is* the downsample, and the antialias pass that runs on its result. Everything
    // upstream is in render pixels and must keep using `viewport`, since that is the size of the
    // textures it is loading from.
    //
    // Appended at the end of the block deliberately. `terrain_ao.wgsl` binds this same buffer through a
    // struct declaring only the fields it reads, which is sound exactly as long as that declaration
    // stays a *prefix* of this one -- so a field inserted above would silently misalign every field
    // after it there.
    output: vec4<f32>,
}

// `params` packs the world units spanned by the full normalized depth range in `y` and the world
// units covered by one shadow texel in `z`. `x` and `w` are reserved. Both scales are per cascade,
// since the fitted frusta differ by more than an order of magnitude.
struct ShadowCascade {
    view_projection: mat4x4<f32>,
    params: vec4<f32>,
}

/// Four cascades rather than more. An RTS camera has a bounded height range, so the depth interval
/// needing shadows is far narrower than a free-flight camera's, and a fifth cascade would fit a
/// frustum slice the camera cannot reach.
const SHADOW_CASCADE_COUNT: i32 = 4;

struct ShadowCamera {
    cascades: array<ShadowCascade, 4>,
}

struct FullscreenOutput {
    @builtin(position) position: vec4<f32>,
}

/// A single oversized triangle covering the viewport, so no vertex buffer is needed.
@vertex
fn fullscreen_vertex(@builtin(vertex_index) vertex_index: u32) -> FullscreenOutput {
    let x = f32(i32(vertex_index) - 1) * 3.0;
    let y = f32(i32(vertex_index & 1u) * 2 - 1) * 3.0;
    var output: FullscreenOutput;
    output.position = vec4<f32>(x, y, 0.0, 1.0);
    return output;
}

@group(0) @binding(0) var g_albedo: texture_2d<f32>;
@group(0) @binding(1) var g_normal: texture_2d<f32>;
// Geometry coverage in `r`: below 0.5 no geometry was drawn, 1.0 is opaque geometry, and anything
// above 1.0 carries that much emissive strength.
@group(0) @binding(2) var g_coverage: texture_2d<f32>;
@group(0) @binding(3) var<uniform> camera: SceneCamera;
@group(0) @binding(4) var primary_shadow: texture_depth_2d_array;
@group(0) @binding(5) var primary_shadow_sampler: sampler_comparison;
@group(0) @binding(6) var<uniform> shadow_camera: ShadowCamera;
@group(0) @binding(7) var ambient_occlusion: texture_2d<f32>;
// The scene depth buffer, read directly. There is no multisample resolve step: this pass is not
// multisampled, so the depth attachment the G-buffer wrote is sampleable as-is.
@group(0) @binding(8) var scene_depth: texture_depth_2d;

// Reconstructs a G-buffer pixel's world position from its depth.
//
// The G-buffer used to carry world position in an `Rgba16Float` target, and a half float has ten
// mantissa bits: past 1024 world units the representable step is a whole unit, and past 2048 it is
// two. On a map a few thousand units across that snapped every receiver onto a lattice, and because
// the shadow map holds smooth depth, roughly half of each lattice cell projected behind its own
// stored depth -- striped self-shadowing across the whole terrain that no bias or filter setting
// could reach, since the error was an order of magnitude larger than a shadow texel.
//
// Depth carries the same information without that loss. At this projection's 1.0 near plane the
// reconstruction error is about `distance^2 * 6e-8` world units: five thousandths of a unit at 300
// units out and a seventh of a unit at the far edge of the shadowed range, against the one-to-two
// units the old target lost everywhere. It also costs less bandwidth than it saves, because the
// depth already had to be resolved for the forward passes.
fn world_from_depth(pixel: vec2<i32>, depth: f32) -> vec3<f32> {
    // `viewport.zw` holds the reciprocal viewport, and clip space is y-up while pixels are y-down.
    let uv = (vec2<f32>(pixel) + vec2<f32>(0.5)) * camera.viewport.zw;
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let homogeneous = camera.inverse_view_projection * vec4<f32>(ndc, depth, 1.0);
    return homogeneous.xyz / homogeneous.w;
}

fn world_at(pixel: vec2<i32>) -> vec3<f32> {
    return world_from_depth(pixel, textureLoad(scene_depth, pixel, 0));
}
