// Composes one terrain page: the blended surface for a rectangle of cells, baked into a physical page of
// the virtual-texture cache.
//
// # Why this file was rewritten rather than wired up
//
// It previously composed pages from a *tile atlas* terrain model — per-cell material slots, blend masks
// with orientation and diagonal codes, a 32-pixel edge-tile sheet, a macro lattice. This project's terrain
// is a heightfield plus per-layer weight textures and has none of those things, so there was nothing to
// connect it to: every input it declared was a resource the engine does not build. The audit in
// LICENSING.md clears the old file of derivation and it was still written for a terrain this engine does
// not have. See that file's note on `terrain_virtual.wgsl`.
//
// # What a composed page is for
//
// The G-buffer's `surface()` blends up to eight layers per fragment, every fragment, every frame: eight
// weight samples and eight world-space albedo samples, of which all but two or three are usually multiplied
// by a zero weight. That work is *the same every frame* for a given piece of ground, because it depends only
// on the terrain data. Baking it into a page moves the cost from per-fragment-per-frame to per-page-once,
// and the residency bookkeeping in `terrain_virtual.rs` decides which pages are worth holding.
//
// The blend below therefore has to agree with `surface()` in `terrain_gbuffer.wgsl`. Tests read a composed
// page back and check the properties that decide whether the cache is usable — that a page over known ground
// holds that ground's surface, and that a page's border matches the interior of the page beside it — because
// those are the two things a comment cannot hold and a rendered comparison cannot isolate.
//
// # Why the page has a border
//
// A page is sampled with filtering, and a bilinear tap at a page's edge needs texels from beyond it. Without
// a margin those taps clamp, which puts a visible seam along every page boundary — and page boundaries move
// as the camera does, so the seams would crawl. The border is composed from the neighbouring cells' real
// data, so a tap across the edge reads what it would have read from the adjacent page.

// Must match `VIRTUAL_PAGE_BORDER` in `terrain_virtual.rs`; a test pins the pair.
const PAGE_BORDER: u32 = 8u;

const MAX_LAYERS: u32 = 8u;

// A prefix of the terrain uniform block. This pass binds the terrain's own buffer, so the fields it reads
// cannot disagree with what the G-buffer reads — the same trick `terrain_forward.wgsl` uses, and sound for
// the same reason: everything not declared here is appended after it. See `UNIFORM_BYTES` in `terrain.rs`.
struct Uniforms {
    view_projection: mat4x4<f32>,
    camera_position: vec4<f32>,
    light_direction: vec4<f32>,
    light_ambient: vec4<f32>,
    light_diffuse: vec4<f32>,
    // x horizontal scale, y world units per elevation step, z width, w height (in samples).
    terrain: vec4<f32>,
    // x layer count, yzw the chunk decomposition.
    layers: vec4<u32>,
    // Per layer: rgb colour multiplier, w roughness.
    palette: array<vec4<f32>, 8>,
    // Per layer: x world units per albedo repeat, yzw unused.
    detail: array<vec4<f32>, 8>,
}

/// One page to compose: `page` holds the cell origin in `xy`, the cells the page spans in `z`, and the
/// physical layer to write in `w`. `detail` holds the page's texels per cell in `x`.
struct PageJob {
    page: vec4<u32>,
    detail: vec4<u32>,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var weight_texture: texture_2d_array<f32>;
@group(0) @binding(2) var weight_sampler: sampler;
@group(0) @binding(3) var albedo_texture: texture_2d_array<f32>;
@group(0) @binding(4) var albedo_sampler: sampler;
@group(0) @binding(5) var<storage, read> jobs: array<PageJob>;
// `rgba8unorm` because WebGPU has no sRGB *storage* format. That is not merely a format substitution: the
// blend below is in linear light, and eight bits of linear is visibly banded in the darks — which is the
// entire reason sRGB storage exists. So the pass encodes on write and whoever reads a page decodes, moving
// the transfer function out of the sampler and into the `transfer` chunk the three passes on that route share.
//
// A single mip level, because a storage binding has exactly one. This pass writes the base and
// `terrain_reduce.wgsl` fills the levels below it from what this wrote.
@group(0) @binding(6) var composed: texture_storage_2d_array<rgba8unorm, write>;

fn sample_count() -> vec2<f32> {
    return vec2<f32>(uniforms.terrain.z, uniforms.terrain.w);
}

/// The mip level to read a layer's albedo at, for a page of the given texel density.
///
/// A compute shader has no screen-space derivatives, so the level a fragment shader gets for free has to be
/// derived here — and deriving it is not a workaround but the more correct answer, because a page's density
/// is a property of the page rather than of whoever looks at it. One page texel covers
/// `horizontal_scale / pixels_per_cell` world units; one albedo texel covers `detail_scale / albedo_width`.
/// The base-two logarithm of the ratio is the level at which one albedo texel matches one page texel, which
/// is exactly the point past which sampling level zero would alias.
///
/// Floored at zero: a page denser than its albedo wants magnification, and there is no level above the base
/// to magnify from.
fn albedo_level(index: u32, pixels_per_cell: f32) -> f32 {
    let albedo_width = f32(max(textureDimensions(albedo_texture).x, 1u));
    let page_texel_world = uniforms.terrain.x / pixels_per_cell;
    let albedo_texel_world = max(uniforms.detail[index].x, 0.0001) / albedo_width;
    return max(log2(page_texel_world / albedo_texel_world), 0.0);
}

/// The blended layer surface at a normalized terrain coordinate: albedo in `xyz`, roughness in `w`.
///
/// The same blend as `surface()` in `terrain_gbuffer.wgsl`, including the fallback for a coordinate where
/// every weight is zero. Every layer is sampled regardless of its weight for the reason stated there, and
/// the albedo coordinate is world-space rather than `uv` so a repeat is a real size.
///
/// The one difference is the mip level, and it is a difference in kind rather than a discrepancy: see
/// `albedo_level`.
fn surface(uv: vec2<f32>, pixels_per_cell: f32) -> vec4<f32> {
    let count = min(uniforms.layers.x, MAX_LAYERS);
    let cells = max(sample_count() - vec2<f32>(1.0), vec2<f32>(1.0));
    let world = uv * cells * uniforms.terrain.x;
    var accumulated = vec3<f32>(0.0);
    var roughness = 0.0;
    var total = 0.0;
    for (var index = 0u; index < count; index = index + 1u) {
        let weight = textureSampleLevel(weight_texture, weight_sampler, uv, i32(index), 0.0).r;
        let tile = world / uniforms.detail[index].x;
        let level = albedo_level(index, pixels_per_cell);
        let detail = textureSampleLevel(albedo_texture, albedo_sampler, tile, i32(index), level).rgb;
        accumulated = accumulated + uniforms.palette[index].rgb * detail * weight;
        roughness = roughness + uniforms.palette[index].w * weight;
        total = total + weight;
    }
    if (total <= 0.0001) {
        return vec4<f32>(0.32, 0.30, 0.27, 0.88);
    }
    return vec4<f32>(accumulated / total, roughness / total);
}

// The write below encodes through `srgb_from_linear` from the `transfer` chunk, which the reduce pass and the
// G-buffer share — see that file for why the transfer function lives in the shaders rather than in a sampler.

// One workgroup per 8x8 block of page texels, and one dispatch depth per job. `z` indexes the job rather
// than the physical layer, because the jobs are the compacted list of pages that actually need composing —
// dispatching over the whole cache would be up to 256 layers of no work.
@compute @workgroup_size(8, 8, 1)
fn compose_page(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let extent = textureDimensions(composed);
    if (invocation.x >= extent.x || invocation.y >= extent.y) {
        return;
    }
    let job = jobs[invocation.z];
    let origin = vec2<f32>(f32(job.page.x), f32(job.page.y));
    let pixels_per_cell = f32(max(job.detail.x, 1u));

    // Page texel to terrain cell. The border is *negative* in page-local cell space and past
    // `cells_per_page` on the far side, which is exactly what makes a filtered tap across the page edge read
    // the neighbouring ground rather than a clamped copy of this page's own last row.
    let local = (vec2<f32>(invocation.xy) - f32(PAGE_BORDER) + vec2<f32>(0.5)) / pixels_per_cell;
    let cell = origin + local;

    // Clamped to the terrain, because a page at the map's edge has a border that leaves it. Clamping there
    // is correct rather than a compromise: there is no ground beyond the edge, and the alternative — leaving
    // it undefined — would put whatever the allocation held into a filtered tap.
    let uv = clamp(
        cell / max(sample_count() - vec2<f32>(1.0), vec2<f32>(1.0)),
        vec2<f32>(0.0),
        vec2<f32>(1.0),
    );
    let blended = surface(uv, pixels_per_cell);
    // Roughness rides in alpha. The G-buffer needs both and a page has four channels, so splitting them
    // across two textures would double the cache's memory to carry one byte.
    textureStore(
        composed,
        vec2<i32>(invocation.xy),
        i32(job.page.w),
        vec4<f32>(srgb_from_linear(blended.rgb), blended.w),
    );
}
