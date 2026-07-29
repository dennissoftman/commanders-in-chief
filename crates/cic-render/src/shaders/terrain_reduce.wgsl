// Reduces one composed terrain page by half, which is what turns the virtual-texture path from correct into
// better.
//
// # What a page without a chain does wrong
//
// A page holds one density. The G-buffer's own fallback — the direct layer blend — samples an albedo array
// that has a full mip chain, so it minifies gracefully; a single-level page does not, and terrain seen at a
// shallow angle therefore aliases *worse* through the cache than without it. That is the opposite of what a
// virtual texture is for, and it is why this pass is a precondition for using the cache rather than an
// optimisation of it.
//
// # Why the reduction is a 2x2 box and not a wider kernel
//
// A box filter over the exact 2x2 footprint is what a mip level *means*: the average of the area the coarser
// texel covers. A wider kernel would blur across footprints, and the sampler's trilinear blend between
// levels already interpolates between them.
//
// # Why the border survives it
//
// The border is what lets a filtered tap at a page edge read the neighbouring ground instead of a clamped
// copy of this page's own last row. Halving the page halves the border with it, and because the border is a
// power of two the interior's boundary stays on a texel boundary at every level the chain carries — see
// `VIRTUAL_PAGE_BORDER` in `terrain_virtual.rs`, which is also what fixes how deep the chain goes. Get that
// wrong and the seam the border exists to prevent returns at every level below the base, which is the one
// failure mode of this pass that a frame at close range would not show.
//
// # Why the average is taken in linear light
//
// A page stores sRGB-encoded colour. The transfer curve is concave, so the mean of two encoded values sits
// *above* the encoding of their mean: averaging stored bytes makes a high-contrast page pale as it recedes,
// a gradient the eye reads as fog nobody added. This project has the same note against its CPU mip
// generation for the same reason. Roughness rides in alpha and is a linear measurement already, so it
// averages as stored.

/// One page job, matching `terrain_virtual.wgsl` and `VirtualPageJob` in `terrain_virtual.rs`.
///
/// Only `page.w` — the physical layer — is read here: which ground a page holds was settled by the compose
/// pass, and this pass reduces whatever that wrote. Dispatching over the *jobs* rather than over the whole
/// cache is what keeps the cost proportional to the pages that actually changed, which in the steady state
/// is none of them.
struct PageJob {
    page: vec4<u32>,
    detail: vec4<u32>,
}

@group(0) @binding(0) var<storage, read> jobs: array<PageJob>;
// The level being read, as a *sampled* array with one mip level rather than a read-only storage binding:
// `textureLoad` with integer coordinates is the exact 2x2 fetch this wants, it needs no sampler, and it
// avoids asking a software adapter for a storage-read capability the pass has no use for.
@group(0) @binding(1) var source: texture_2d_array<f32>;
@group(0) @binding(2) var destination: texture_storage_2d_array<rgba8unorm, write>;

@compute @workgroup_size(8, 8, 1)
fn reduce_page(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let extent = textureDimensions(destination);
    if (invocation.x >= extent.x || invocation.y >= extent.y) {
        return;
    }
    let layer = i32(jobs[invocation.z].page.w);
    // Clamped rather than assumed even. The levels this chain carries all halve exactly — there is a test on
    // that — so the clamp never fires; it is here because a texture load has to be total, which is the same
    // rule every decoder in this project follows.
    let last = vec2<i32>(textureDimensions(source)) - vec2<i32>(1);
    let base = vec2<i32>(invocation.xy) * 2;

    var colour = vec3<f32>(0.0);
    var roughness = 0.0;
    for (var y = 0; y < 2; y = y + 1) {
        for (var x = 0; x < 2; x = x + 1) {
            let at = min(base + vec2<i32>(x, y), last);
            let taken = textureLoad(source, at, layer, 0);
            colour = colour + linear_from_srgb(taken.rgb);
            roughness = roughness + taken.a;
        }
    }
    textureStore(
        destination,
        vec2<i32>(invocation.xy),
        layer,
        vec4<f32>(srgb_from_linear(colour * 0.25), roughness * 0.25),
    );
}
