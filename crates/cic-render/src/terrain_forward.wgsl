// Forward terrain pass.
//
// The grid is procedural: there is no vertex buffer. A draw of `cells * 6` vertices derives its
// sample coordinate from `vertex_index` alone, and elevation comes from the height texture in the
// vertex shader rather than from baked positions.
//
// That is a deliberate choice with two payoffs. Terrain deformation becomes a texture write instead
// of a remesh and a buffer upload; and layer weights, which decide surface appearance, are likewise
// writable at runtime -- so grading a road across the map is a write into the weight array, not a
// rebuild of terrain geometry.
//
// Normals are computed here from central differences on the height texture, for the same reason:
// a normal buffer would have to be regenerated on every edit.

const MAX_LAYERS: u32 = 8u;

struct Uniforms {
    view_projection: mat4x4<f32>,
    // xyz camera position, w unused.
    camera_position: vec4<f32>,
    // xyz unit direction *toward* the light, w unused.
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
// An integer texture, not a normalized one: `R16Uint` is baseline-supported where `R16Unorm`
// needs an optional feature, and since elevations are only ever loaded -- never filtered --
// integer sampling costs nothing and removes a normalization round trip.
@group(0) @binding(1) var height_texture: texture_2d<u32>;
@group(0) @binding(2) var weight_texture: texture_2d_array<f32>;
@group(0) @binding(3) var weight_sampler: sampler;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) normal: vec3<f32>,
}

fn sample_count() -> vec2<i32> {
    return vec2<i32>(i32(uniforms.terrain.z), i32(uniforms.terrain.w));
}

/// Loads one sample's world elevation, clamping the coordinate so the differences used for normals
/// stay in range at the terrain edge instead of wrapping or reading zero.
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
    // Central differences: the gradient across two cells, so the divisor is twice the spacing.
    let slope = vec2<f32>((east - west) / (2.0 * spacing), (north - south) / (2.0 * spacing));
    return normalize(vec3<f32>(-slope.x, -slope.y, 1.0));
}

@vertex
fn vertex_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let samples = sample_count();
    let cells_x = u32(max(samples.x - 1, 1));

    let quad = vertex_index / 6u;
    let corner = vertex_index % 6u;
    let cell = vec2<u32>(quad % cells_x, quad / cells_x);

    // Two triangles per cell, counter-clockwise seen from +Z so front faces point up.
    var offsets = array<vec2<u32>, 6>(
        vec2<u32>(0u, 0u),
        vec2<u32>(1u, 0u),
        vec2<u32>(0u, 1u),
        vec2<u32>(1u, 0u),
        vec2<u32>(1u, 1u),
        vec2<u32>(0u, 1u),
    );
    let coordinate = vec2<i32>(cell + offsets[corner]);

    let spacing = uniforms.terrain.x;
    let height = elevation(coordinate);
    let world_position = vec3<f32>(
        f32(coordinate.x) * spacing,
        f32(coordinate.y) * spacing,
        height,
    );

    var output: VertexOutput;
    output.clip_position = uniforms.view_projection * vec4<f32>(world_position, 1.0);
    output.world_position = world_position;
    // Divide by the sample count minus one, so the last sample lands exactly on 1.0 rather than
    // slightly inside it -- an off-by-one here shears every layer across the map.
    output.uv = vec2<f32>(coordinate) / max(vec2<f32>(samples - vec2<i32>(1)), vec2<f32>(1.0));
    output.normal = surface_normal(coordinate);
    return output;
}

/// Blends the layer palette by per-sample weight.
///
/// Weights are normalized by their own total rather than assumed to sum to one, so a partially
/// painted map does not darken toward black where coverage is incomplete.
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
        // No layer covers this sample. A neutral surface is more useful than black: it keeps the
        // terrain's shape readable in a capture rather than hiding an authoring gap as a void.
        return vec3<f32>(0.32, 0.30, 0.27);
    }
    return accumulated / total;
}

@fragment
fn fragment_main(input: VertexOutput) -> @location(0) vec4<f32> {
    var normal = normalize(input.normal);
    if (normal.z < 0.0) {
        normal = -normal;
    }
    let albedo = surface_albedo(input.uv);
    let incidence = max(dot(normal, normalize(uniforms.light_direction.xyz)), 0.0);
    let lit = uniforms.light_ambient.rgb + uniforms.light_diffuse.rgb * incidence;
    // Clamped rather than tone mapped. This pass is not HDR -- albedo and the light terms are both
    // near unity -- so a Reinhard curve here would only compress contrast that was never out of
    // range, flattening exactly the slope shading the pass exists to show. Tone mapping belongs with
    // the deferred path, where accumulated lights genuinely can exceed one.
    let colour = clamp(albedo * lit, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(colour, 1.0);
}
