// Terrain G-buffer and shadow-depth passes.
//
// The grid is procedural, exactly as in `terrain_forward.wgsl`, and both passes share that crate's
// `Uniforms` block byte for byte so one uniform buffer drives all three pipelines. Elevation still
// comes from the height texture in the vertex shader, which is what keeps terrain deformation a
// texture write rather than a remesh.
//
// The G-buffer stores no world position. It is reconstructed from depth in the lighting pass -- see
// `world_from_depth` in `scene.wgsl` for why storing it was actively harmful.

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
    // Per layer: rgb colour multiplier, w roughness.
    palette: array<vec4<f32>, 8>,
    // Per layer: x world units per albedo repeat, yzw unused.
    detail: array<vec4<f32>, 8>,
    // xy the world wind vector, z scene time, w the previous frame's scene time. Terrain does not move, so
    // this pass reads none of it -- it is declared to reach the two entries after it.
    animation: vec4<f32>,
    // xy this frame's sub-pixel jitter as a clip-space offset, zw unused.
    jitter: vec4<f32>,
    // The previous frame's unjittered view-projection, for the motion target.
    previous_view_projection: mat4x4<f32>,
    // x nonzero when the virtual-texture bindings hold a real cache rather than placeholders. See
    // `TerrainRenderer::attach_pages`.
    virtual_config: vec4<u32>,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var height_texture: texture_2d<u32>;
@group(0) @binding(2) var weight_texture: texture_2d_array<f32>;
@group(0) @binding(3) var weight_sampler: sampler;
@group(0) @binding(4) var albedo_texture: texture_2d_array<f32>;
@group(0) @binding(5) var albedo_sampler: sampler;
// The virtual-texture cache: one page table per level, then the composed pages. Loaded and sampled only by
// the fragment stage, and only when `virtual_config.x` says they are real -- see `page_surface`.
@group(0) @binding(6) var page_table_fine: texture_2d<u32>;
@group(0) @binding(7) var page_table_coarse: texture_2d<u32>;
@group(0) @binding(8) var page_texture: texture_2d_array<f32>;

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
    // The clip positions this frame and last, carried undivided. See `motion_vector`.
    @location(2) current_clip: vec4<f32>,
    @location(3) previous_clip: vec4<f32>,
}

struct GBufferOutput {
    // Albedo in `rgb`, the metallic factor in `a`. Terrain is a dielectric everywhere -- soil, gravel,
    // asphalt and snow are all insulators -- so it writes zero, and the lighting pass's metallic path
    // reduces to exactly what it computed before that channel carried anything.
    @location(0) albedo: vec4<f32>,
    // World normal in `xyz`, roughness in `w`.
    @location(1) normal_roughness: vec4<f32>,
    // Geometry coverage: below 0.5 nothing drew, 1.0 is opaque geometry, and anything above 1.0
    // carries that much emissive strength. Terrain never emits, so it writes exactly 1.0.
    @location(2) coverage: f32,
    // Texture-coordinate motion since the previous frame. Written unconditionally, and read only by the
    // temporal resolve -- so with that resolve off this costs the write and nothing else.
    @location(3) motion: vec2<f32>,
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

/// One vertex of the grid: its sample coordinate, and whether that coordinate is on the terrain at all.
struct GridSample {
    coordinate: vec2<i32>,
    inside: bool,
}

/// Resolves a vertex within a chunk, plus the chunk it belongs to, to a terrain sample coordinate.
///
/// The chunk arrives as the *instance index*, which is what keeps culling free of any new binding: the
/// CPU decides which chunks to draw and expresses it as instance ranges, and this turns each index back
/// into a grid origin using the counts already in the terrain uniform. `layers.y` is the number of chunks
/// across and `layers.z` the cells along a chunk's edge.
///
/// `inside` is false for a vertex past the terrain's edge, which happens in the last chunk of a row or
/// column whenever the cell count is not a multiple of the chunk size. The caller collapses those to a
/// degenerate triangle. Clamping them instead -- which is what `elevation` does to a coordinate it is
/// handed -- would stretch the final row of cells across the ground that is not there.
fn grid_sample(vertex_index: u32, chunk_index: u32) -> GridSample {
    let samples = sample_count();
    let cells = vec2<u32>(u32(max(samples.x - 1, 1)), u32(max(samples.y - 1, 1)));
    let chunks_across = max(uniforms.layers.y, 1u);
    let chunk_cells = max(uniforms.layers.z, 1u);

    let quad = vertex_index / 6u;
    let corner = vertex_index % 6u;
    let local = vec2<u32>(quad % chunk_cells, quad / chunk_cells);
    let origin = vec2<u32>(chunk_index % chunks_across, chunk_index / chunks_across) * chunk_cells;
    let cell = origin + local;

    var offsets = array<vec2<u32>, 6>(
        vec2<u32>(0u, 0u),
        vec2<u32>(1u, 0u),
        vec2<u32>(0u, 1u),
        vec2<u32>(1u, 0u),
        vec2<u32>(1u, 1u),
        vec2<u32>(0u, 1u),
    );
    var output: GridSample;
    output.coordinate = vec2<i32>(cell + offsets[corner]);
    output.inside = cell.x < cells.x && cell.y < cells.y;
    return output;
}

/// A clip position no triangle survives: behind the near plane, which this projection puts at zero.
///
/// Used for a vertex outside the terrain. All six vertices of an out-of-range quad get the same value, so
/// the primitive is degenerate as well as clipped.
fn discarded_vertex() -> vec4<f32> {
    return vec4<f32>(0.0, 0.0, -1.0, 1.0);
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
fn gbuffer_vertex(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    let sample = grid_sample(vertex_index, instance_index);
    let coordinate = sample.coordinate;
    let samples = sample_count();
    let world = world_position(coordinate);
    var output: VertexOutput;
    output.clip_position = uniforms.view_projection * vec4<f32>(world, 1.0);
    if !sample.inside {
        output.clip_position = discarded_vertex();
    }
    output.current_clip = output.clip_position;
    // Terrain geometry does not move, so its motion is the camera's alone -- the same world position under
    // the previous view. A heightfield edit between frames is the one exception, and it is a texture write
    // rather than a transform, so nothing here could know about it; the resolve's neighbourhood clamp is
    // what keeps that from ghosting.
    output.previous_clip = uniforms.previous_view_projection * vec4<f32>(world, 1.0);
    output.uv = vec2<f32>(coordinate) / max(vec2<f32>(samples - vec2<i32>(1)), vec2<f32>(1.0));
    output.normal = surface_normal(coordinate);
    return output;
}

/// World XY of a fragment, recovered from its normalized grid coordinate. Identical to the forward
/// pass's, deliberately: the two must agree on where a detail texture tiles or the same terrain
/// captured through each path would not match.
fn world_xy(uv: vec2<f32>) -> vec2<f32> {
    let cells = max(
        vec2<f32>(uniforms.terrain.z, uniforms.terrain.w) - vec2<f32>(1.0),
        vec2<f32>(1.0),
    );
    return uv * cells * uniforms.terrain.x;
}

/// Blends the layer surfaces by per-sample weight. Albedo in `xyz`, roughness in `w`.
///
/// See `terrain_forward.wgsl` for why every layer is sampled regardless of its weight, and why the
/// albedo coordinate is world-space rather than `uv`.
fn surface(uv: vec2<f32>) -> vec4<f32> {
    let count = min(uniforms.layers.x, MAX_LAYERS);
    let world = world_xy(uv);
    var accumulated = vec3<f32>(0.0);
    var roughness = 0.0;
    var total = 0.0;
    for (var index = 0u; index < count; index = index + 1u) {
        let weight = textureSample(weight_texture, weight_sampler, uv, i32(index)).r;
        let tile = world / uniforms.detail[index].x;
        let detail = textureSample(albedo_texture, albedo_sampler, tile, i32(index)).rgb;
        accumulated = accumulated + uniforms.palette[index].rgb * detail * weight;
        roughness = roughness + uniforms.palette[index].w * weight;
        total = total + weight;
    }
    if (total <= 0.0001) {
        return vec4<f32>(0.32, 0.30, 0.27, 0.88);
    }
    return vec4<f32>(accumulated / total, roughness / total);
}

// Page geometry, duplicated across the language boundary from `terrain_virtual.rs`. A test pins each pair,
// because a disagreement here does not fail to compile -- it samples the right page at the wrong place.
const PAGE_BORDER: f32 = 8.0;
const PAGE_EXTENT: f32 = 272.0;
// Levels a page carries, which the border fixes rather than the interior -- see `VIRTUAL_PAGE_BORDER`.
const PAGE_MIPS: f32 = 4.0;
// Cells a page spans, and texels it holds per cell, at each level. Their product is the page interior, which
// is what makes the two levels the same memory at different densities: 8 cells at 32 texels and 16 at 16.
const PAGE_FINE_CELLS: f32 = 8.0;
const PAGE_FINE_DENSITY: f32 = 32.0;
const PAGE_COARSE_CELLS: f32 = 16.0;
const PAGE_COARSE_DENSITY: f32 = 16.0;

/// What a page lookup found, or that it found nothing.
struct PageSample {
    surface: vec4<f32>,
    resident: bool,
}

/// The mip level to read a page at, from the screen footprint of one of its texels.
///
/// A page has a chain now, and something has to pick a level from it. The sampler cannot: `textureSample`
/// takes its level from screen-space derivatives, which are defined only in uniform control flow, and page
/// residency varies per fragment -- so the derivatives are taken once at the top of the fragment, where the
/// control flow *is* uniform, and the level is computed here and passed to `textureSampleLevel`.
///
/// `derivative` is how far the terrain coordinate moves across one pixel. Multiplying by the cell count and
/// the page's density converts that to page texels, and the base-two logarithm of the longer of the two axes
/// is the level at which one page texel covers one pixel -- which is the point past which the base level
/// aliases. `max` of the two axes rather than a per-axis level, because that is what the hardware does for the
/// direct blend this path has to agree with: an anisotropic footprint is resolved by blurring along its short
/// axis, and doing anything better needs a device capability the headless suite cannot rely on.
fn page_level(derivative_x: vec2<f32>, derivative_y: vec2<f32>, density: f32) -> f32 {
    let cells = max(
        vec2<f32>(uniforms.terrain.z, uniforms.terrain.w) - vec2<f32>(1.0),
        vec2<f32>(1.0),
    );
    let texels = cells * density;
    let footprint = max(
        length(derivative_x * texels),
        length(derivative_y * texels),
    );
    // Floored at the base rather than allowed negative: magnification has no level above zero to come from.
    // Capped at the last level the chain holds, because `textureSampleLevel` would clamp there anyway and a
    // stated bound is what makes the depth's disagreement with `VIRTUAL_PAGE_MIPS` a test rather than a guess.
    return clamp(log2(max(footprint, 0.0001)), 0.0, PAGE_MIPS - 1.0);
}

/// Reads one page, given the table entry for the page containing `cell`.
///
/// `entry` is the physical layer plus one, so zero means not resident -- which is what a cleared table reads
/// as, and what the one-by-one placeholder reads as when no cache is attached. One code path for "the cache
/// has not staged this yet", "the cache evicted it", and "there is no cache", because the fragment wants the
/// same answer in all three.
fn page_lookup(
    entry: u32,
    cell: vec2<f32>,
    cells_per_page: f32,
    density: f32,
    level: f32,
) -> PageSample {
    var output: PageSample;
    output.surface = vec4<f32>(0.0);
    output.resident = entry != 0u;
    if (!output.resident) {
        return output;
    }
    // Where in its page this cell sits, then where that lands in the page's texels. The half-texel the
    // compose pass added when it wrote is *not* re-added here: this converts a texel index to a normalized
    // coordinate, and a texel's centre is at its index plus a half, so the two halves cancel.
    let local = cell - floor(cell / cells_per_page) * cells_per_page;
    let texel = PAGE_BORDER + local * density;
    let page_uv = texel / PAGE_EXTENT;
    let stored = textureSampleLevel(
        page_texture,
        weight_sampler,
        page_uv,
        i32(entry - 1u),
        level,
    );
    // Colour is sRGB-encoded and roughness is not, which is the same split the G-buffer's own albedo target
    // has.
    output.surface = vec4<f32>(linear_from_srgb(stored.rgb), stored.a);
    return output;
}

/// The composed surface for a terrain coordinate, preferring the finer page level.
///
/// Finer first because that is the whole point of two levels: a page at 32 texels per cell is the one staged
/// for ground near the camera, and falling back to the coarse level where it is absent is a graceful
/// degradation rather than a compromise.
///
/// The two derivatives are the caller's, taken where the control flow is uniform -- see `page_level`.
fn page_surface(uv: vec2<f32>, derivative_x: vec2<f32>, derivative_y: vec2<f32>) -> PageSample {
    var output: PageSample;
    output.surface = vec4<f32>(0.0);
    output.resident = false;
    if (uniforms.virtual_config.x == 0u) {
        return output;
    }
    let cells = max(
        vec2<f32>(uniforms.terrain.z, uniforms.terrain.w) - vec2<f32>(1.0),
        vec2<f32>(1.0),
    );
    let cell = clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)) * cells;

    // Clamped to the table rather than tested against it. A coordinate exactly at the terrain's far edge
    // lands one page past the end, and clamping there reads the last page -- whose border covers that very
    // edge, because the compose pass composed it from the same clamped ground.
    let fine_page = vec2<i32>(min(
        cell / PAGE_FINE_CELLS,
        vec2<f32>(textureDimensions(page_table_fine)) - vec2<f32>(1.0),
    ));
    let fine = page_lookup(
        textureLoad(page_table_fine, fine_page, 0).x,
        cell,
        PAGE_FINE_CELLS,
        PAGE_FINE_DENSITY,
        page_level(derivative_x, derivative_y, PAGE_FINE_DENSITY),
    );
    if (fine.resident) {
        return fine;
    }
    let coarse_page = vec2<i32>(min(
        cell / PAGE_COARSE_CELLS,
        vec2<f32>(textureDimensions(page_table_coarse)) - vec2<f32>(1.0),
    ));
    return page_lookup(
        textureLoad(page_table_coarse, coarse_page, 0).x,
        cell,
        PAGE_COARSE_CELLS,
        PAGE_COARSE_DENSITY,
        page_level(derivative_x, derivative_y, PAGE_COARSE_DENSITY),
    );
}

@fragment
fn gbuffer_fragment(input: VertexOutput) -> GBufferOutput {
    var normal = normalize(input.normal);
    if (normal.z < 0.0) {
        normal = -normal;
    }
    // A composed page when one is resident, and the direct blend otherwise. The fallback is not
    // belt-and-braces: a cache is allowed to run out of slots, so a frame that depended on it having won
    // would turn a memory budget into a correctness requirement.
    //
    // Both branches are evaluated. That is the same constraint the model materials live under -- a mip level
    // comes from screen-space derivatives, which are undefined in non-uniform control flow, and page
    // residency varies per fragment. Guarding the blend would leave the mip level of every fragment that
    // *does* fall back undefined.
    //
    // The page path's derivatives are taken *here*, for the same reason and one step earlier: residency is
    // decided inside `page_surface`, so by the time a page is known to exist the control flow is no longer
    // uniform and a derivative taken there would be undefined. Taken at the top they are a plain value the
    // lookup can carry down.
    let derivative_x = dpdx(input.uv);
    let derivative_y = dpdy(input.uv);
    let page = page_surface(input.uv, derivative_x, derivative_y);
    let blended = surface(input.uv);
    let surface_value = select(blended, page.surface, page.resident);
    var output: GBufferOutput;
    output.albedo = vec4<f32>(surface_value.rgb, 0.0);
    // Roughness is per layer and blended by the same weights as the colour, so a gravel layer and a
    // wet-asphalt layer can meet on one fragment and each contribute its own specular response.
    output.normal_roughness = vec4<f32>(normal, surface_value.w);
    output.coverage = 1.0;
    output.motion = motion_vector(
        input.current_clip,
        input.previous_clip,
        uniforms.jitter.xy,
    );
    return output;
}

/// Depth-only pass, rendered once per shadow cascade.
///
/// Deliberately has no fragment shader: terrain is fully opaque, so the rasterizer's depth write is
/// the entire output and a fragment stage would only cost bandwidth.
@vertex
fn shadow_vertex(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> @builtin(position) vec4<f32> {
    let sample = grid_sample(vertex_index, instance_index);
    if !sample.inside {
        return discarded_vertex();
    }
    return cascade.view_projection * vec4<f32>(world_position(sample.coordinate), 1.0);
}
