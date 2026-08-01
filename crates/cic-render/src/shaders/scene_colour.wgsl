// The lit scene as it stood before the water pass drew into it.
//
// A composition chunk, and the smallest one here: two bindings and two helpers. It exists as its own
// chunk rather than as three lines in `water.wgsl` because *two* independent features read it — the
// screen-space reflection provider and refraction — and one of the two is swapped out at pipeline
// build time. A binding declared inside a chunk that can be substituted is a binding that disappears
// when the substitution happens.
//
// # Why group 4, and why a copy at all
//
// Water blends *into* the HDR target, so it cannot sample it: one pass cannot both write a texture
// and read it. Everything water wants to know about what is behind or around it therefore comes from
// a copy taken between the lighting pass and the water pass. See `DeferredTargets::scene_colour`.
//
// Group 4 because groups 0 to 3 are all taken by the time water draws -- the scene and its G-buffer,
// water's own uniform, the shadow cascades, and the environment -- and because this is genuinely a
// fifth thing rather than something that belongs to one of the four. It is not on the scene group for
// the reason the shadow cascades were moved off it: five programs bind group 0 and only one of them
// reads this. Reaching five bind groups is why `gpu.rs` asks the adapter for its own limit instead of
// taking wgpu's default of four, which is Vulkan's guaranteed minimum rather than a considered figure.
@group(4) @binding(0) var scene_colour: texture_2d<f32>;
@group(4) @binding(1) var scene_colour_sampler: sampler;

// The lit scene at a pixel, filtered.
//
// Sampled rather than loaded, unlike every other read the water pass makes of a screen-space texture.
// Both callers want a value *between* pixels: a refraction offsets by a fraction of a pixel and a
// reflection march lands wherever it lands, and `textureLoad` on either would quantise the result to
// the pixel grid and put a staircase along every wave.
fn scene_colour_at(uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(scene_colour, scene_colour_sampler, uv, 0.0).rgb;
}

// Where a world position lands on screen, in texture coordinates, and whether it is on screen at all.
//
// `w` carries the clip-space depth so a caller marching a ray can compare against the depth buffer
// without projecting twice. A position behind the camera comes back with a negative homogeneous
// coordinate and would otherwise project to a plausible-looking point in front of it, which is the
// classic screen-space artefact of a reflection appearing where the geometry causing it is behind the
// viewer -- so that case is reported rather than divided through.
struct ScreenPoint {
    uv: vec2<f32>,
    depth: f32,
    on_screen: bool,
}

fn project_to_screen(world_position: vec3<f32>) -> ScreenPoint {
    var point: ScreenPoint;
    let clip = camera.view_projection * vec4<f32>(world_position, 1.0);
    if (clip.w <= 0.0) {
        point.uv = vec2<f32>(0.0);
        point.depth = 0.0;
        point.on_screen = false;
        return point;
    }
    let ndc = clip.xyz / clip.w;
    // Clip space is y-up and texture coordinates are y-down.
    point.uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    point.depth = ndc.z;
    point.on_screen = point.uv.x >= 0.0 && point.uv.x <= 1.0
        && point.uv.y >= 0.0 && point.uv.y <= 1.0;
    return point;
}
