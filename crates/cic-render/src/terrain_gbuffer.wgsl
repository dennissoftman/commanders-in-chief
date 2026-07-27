// Terrain G-buffer and shadow-depth passes.
//
// The grid is procedural, exactly as in `terrain_forward.wgsl`, and both passes share that crate's
// `Uniforms` block byte for byte so one uniform buffer drives all three pipelines. Elevation still
// comes from the height texture in the vertex shader, which is what keeps terrain deformation a
// texture write rather than a remesh.
//
// The G-buffer stores no world position. It is reconstructed from depth in the lighting pass -- see
// `world_from_depth` in `terrain_deferred.wgsl` for why storing it was actively harmful.

const MAX_LAYERS: u32 = 8u;

struct Uniforms {
    view_projection: mat4x4<f32>,
    camera_position: vec4<f32>,
    light_direction: vec4<f32>,
    light_ambient: vec4<f32>,
    light_diffuse: vec4<f32>,
    // x horizontal scale, y world units per elevation step, z width, w height (in samples).
    terrain: vec4<f32>,
    // x layer count, yzw unused.
    layers: vec4<u32>,
    palette: array<vec4<f32>, 8>,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var height_texture: texture_2d<u32>;
@group(0) @binding(2) var weight_texture: texture_2d_array<f32>;
@group(0) @binding(3) var weight_sampler: sampler;

// The cascade being rendered, for the depth-only pass. A separate group so the terrain group can be
// bound once and this one swapped per cascade.
struct ShadowCascadeView {
    view_projection: mat4x4<f32>,
}

@group(1) @binding(0) var<uniform> cascade: ShadowCascadeView;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) normal: vec3<f32>,
}

struct GBufferOutput {
    @location(0) albedo: vec4<f32>,
    // World normal in `xyz`, roughness in `w`.
    @location(1) normal_roughness: vec4<f32>,
    // Geometry coverage: below 0.5 nothing drew, 1.0 is opaque geometry, and anything above 1.0
    // carries that much emissive strength. Terrain never emits, so it writes exactly 1.0.
    @location(2) coverage: f32,
}

fn sample_count() -> vec2<i32> {
    return vec2<i32>(i32(uniforms.terrain.z), i32(uniforms.terrain.w));
}

fn elevation(coordinate: vec2<i32>) -> f32 {
    let limit = sample_count() - vec2<i32>(1);
    let clamped = clamp(coordinate, vec2<i32>(0), limit);
    return f32(textureLoad(height_texture, clamped, 0).r) * uniforms.terrain.y;
}

fn surface_normal(coordinate: vec2<i32>) -> vec3<f32> {
    let spacing = uniforms.terrain.x;
    let west = elevation(coordinate + vec2<i32>(-1, 0));
    let east = elevation(coordinate + vec2<i32>(1, 0));
    let south = elevation(coordinate + vec2<i32>(0, -1));
    let north = elevation(coordinate + vec2<i32>(0, 1));
    let slope = vec2<f32>((east - west) / (2.0 * spacing), (north - south) / (2.0 * spacing));
    return normalize(vec3<f32>(-slope.x, -slope.y, 1.0));
}

/// Resolves a vertex index to its terrain sample coordinate.
fn grid_coordinate(vertex_index: u32) -> vec2<i32> {
    let samples = sample_count();
    let cells_x = u32(max(samples.x - 1, 1));
    let quad = vertex_index / 6u;
    let corner = vertex_index % 6u;
    let cell = vec2<u32>(quad % cells_x, quad / cells_x);
    var offsets = array<vec2<u32>, 6>(
        vec2<u32>(0u, 0u),
        vec2<u32>(1u, 0u),
        vec2<u32>(0u, 1u),
        vec2<u32>(1u, 0u),
        vec2<u32>(1u, 1u),
        vec2<u32>(0u, 1u),
    );
    return vec2<i32>(cell + offsets[corner]);
}

fn world_position(coordinate: vec2<i32>) -> vec3<f32> {
    let spacing = uniforms.terrain.x;
    return vec3<f32>(
        f32(coordinate.x) * spacing,
        f32(coordinate.y) * spacing,
        elevation(coordinate),
    );
}

@vertex
fn gbuffer_vertex(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let coordinate = grid_coordinate(vertex_index);
    let samples = sample_count();
    var output: VertexOutput;
    output.clip_position = uniforms.view_projection * vec4<f32>(world_position(coordinate), 1.0);
    output.uv = vec2<f32>(coordinate) / max(vec2<f32>(samples - vec2<i32>(1)), vec2<f32>(1.0));
    output.normal = surface_normal(coordinate);
    return output;
}

fn surface_albedo(uv: vec2<f32>) -> vec3<f32> {
    let count = min(uniforms.layers.x, MAX_LAYERS);
    var accumulated = vec3<f32>(0.0);
    var total = 0.0;
    for (var index = 0u; index < count; index = index + 1u) {
        let weight = textureSample(weight_texture, weight_sampler, uv, i32(index)).r;
        accumulated = accumulated + uniforms.palette[index].rgb * weight;
        total = total + weight;
    }
    if (total <= 0.0001) {
        return vec3<f32>(0.32, 0.30, 0.27);
    }
    return accumulated / total;
}

@fragment
fn gbuffer_fragment(input: VertexOutput) -> GBufferOutput {
    var normal = normalize(input.normal);
    if (normal.z < 0.0) {
        normal = -normal;
    }
    var output: GBufferOutput;
    output.albedo = vec4<f32>(surface_albedo(input.uv), 1.0);
    // Terrain is a rough dielectric. A single constant for now; a per-layer roughness belongs with
    // the material textures that replace the flat palette.
    output.normal_roughness = vec4<f32>(normal, 0.88);
    output.coverage = 1.0;
    return output;
}

/// Depth-only pass, rendered once per shadow cascade.
///
/// Deliberately has no fragment shader: terrain is fully opaque, so the rasterizer's depth write is
/// the entire output and a fragment stage would only cost bandwidth.
@vertex
fn shadow_vertex(@builtin(vertex_index) vertex_index: u32) -> @builtin(position) vec4<f32> {
    return cascade.view_projection * vec4<f32>(world_position(grid_coordinate(vertex_index)), 1.0);
}
