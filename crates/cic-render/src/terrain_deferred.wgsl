// Deferred lighting over the terrain G-buffer, the water surface, and the tone-mapping composite.
//
// Lighting is deferred because the shadow and occlusion terms are screen-space: both need the whole
// depth buffer resolved before any pixel can be lit, which a forward pass cannot provide.
//
// Water shares this file rather than having its own because it needs `world_from_depth`,
// `sample_cascade`, `shadow_visibility`, and the sky colours verbatim. WGSL has no include mechanism,
// so a separate file would mean a second copy of the cascade selection — with its normal offset, its
// depth slack, its blend band, and its incidence fade — and that is exactly the kind of duplication
// that drifts. Water is also the reason `shadow_visibility` returns a raw fraction and leaves the
// shade floor to its caller: opaque terrain and a transmissive surface keep different amounts of
// light in full shade.

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
    // xy viewport size in pixels, zw its reciprocal.
    viewport: vec4<f32>,
    lights: array<DirectionalLight, 3>,
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

// Both bias terms are expressed in world units and converted with the light frustum's own
// depth range, so widening the fitted frustum for a larger map no longer inflates the bias in
// world space. Nearly all of the slack is taken laterally by offsetting the receiver along its
// normal — the depth slack that remains is a fraction of a texel, which keeps contact shadows
// attached instead of peter-panning them away. Slope-dependent acne is handled by the shadow
// pipelines' rasterizer slope-scaled bias rather than a second term here.
const SHADOW_NORMAL_OFFSET_TEXELS: f32 = 1.5;
const SHADOW_DEPTH_SLACK_TEXELS: f32 = 0.5;

// How much of the primary light survives in full shade. The direct term is nearly extinguished;
// its ambient share only drops part way, standing in for the sky light that still reaches a
// shadowed surface. Ambient has to be attenuated at all because it dominates the sum: the three
// fill light contributes its own unoccluded ambient, so leaving ambient whole in
// shade caps achievable shadow contrast around ten percent and cast shadows read as absent.
const SHADOW_DIRECT_FLOOR: f32 = 0.08;
// Lowered from a larger value once the ambient term itself grew: the floor is a *fraction*, so a
// generous fraction of a realistic skylight ambient leaves shadows nearly as bright as open ground.
// The two have to be tuned together, which is the trap this constant sits in.
const SHADOW_AMBIENT_FLOOR: f32 = 0.32;

// Ambient occlusion attenuates every light's ambient share, including the accent fills the sun's
// shadow deliberately leaves alone: occlusion answers "how much of the sky can this point see",
// which the directional shadow cannot, since that cannot tell a shadowed open field from an
// enclosed corner. Clamped rather than applied whole so interiors darken without going black.
const AO_AMBIENT_FLOOR: f32 = 0.25;

// The two attenuations above overlap rather than compose. A point the sun cannot reach because a
// hill stands in the way is usually also a point that sees less sky, so multiplying both floors
// charges twice for one occluder, and the product bottoms out several times deeper than either
// term alone ever reaches.
//
// Taking the deeper of the two instead of their product keeps whichever term has the better claim on
// a given pixel and stops the other adding to it. The floors themselves are unchanged, so the worst
// case a single term can produce is exactly what it was before.
fn primary_ambient_scale(shadow_visibility: f32, occlusion: f32) -> f32 {
    return min(mix(SHADOW_AMBIENT_FLOOR, 1.0, shadow_visibility), occlusion);
}

// Peak highlight strength, reached only by a fully smooth material.
const SPECULAR_STRENGTH: f32 = 0.06;

// Fraction of a cascade's half-extent, measured inward from its outer boundary, over which its
// result crossfades into the next cascade. The kernel spans a fixed three texels, so its
// world-space width scales with the cascade's texel extent and penumbrae are an order of magnitude
// wider in the outermost cascade than the innermost. Selecting one cascade outright therefore puts
// a visible line on screen where sharp shadows meet blurry ones; fading between them over a band
// turns that line into a gradient, which is what makes the transition imperceptible.
const SHADOW_CASCADE_BLEND: f32 = 0.18;

struct CascadeSample {
    visibility: f32,
    // 1.0 when this cascade contains the receiver at all.
    covered: f32,
    // 0.0 in the cascade's interior, ramping to 1.0 at its outer boundary.
    edge: f32,
}

// `textureSampleCompareLevel` rather than `textureSampleCompare`: the caller's per-pixel loop exit
// makes this non-uniform control flow, which the implicit-derivative variant does not permit.
// Shadow maps never want a mip anyway.
fn sample_cascade(index: i32, world_position: vec3<f32>, normal: vec3<f32>) -> CascadeSample {
    var result: CascadeSample;
    result.visibility = 1.0;
    result.covered = 0.0;
    result.edge = 0.0;
    let cascade = shadow_camera.cascades[index];
    let texel_world = cascade.params.z;
    let depth_range = max(cascade.params.y, 1.0);
    let offset_position = world_position + normal * texel_world * SHADOW_NORMAL_OFFSET_TEXELS;
    let clip = cascade.view_projection * vec4<f32>(offset_position, 1.0);
    if clip.w <= 0.0 {
        return result;
    }
    let projected = clip.xyz / clip.w;
    let uv = projected.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))
        || projected.z < 0.0 || projected.z > 1.0 {
        return result;
    }
    let depth_slack = texel_world * SHADOW_DEPTH_SLACK_TEXELS / depth_range;
    let texel = 1.0 / vec2<f32>(textureDimensions(primary_shadow).xy);
    var visible = 0.0;
    for (var y = -1; y <= 1; y += 1) {
        for (var x = -1; x <= 1; x += 1) {
            visible += textureSampleCompareLevel(
                primary_shadow,
                primary_shadow_sampler,
                uv + vec2<f32>(f32(x), f32(y)) * texel,
                index,
                projected.z - depth_slack
            );
        }
    }
    result.visibility = visible / 9.0;
    result.covered = 1.0;
    let centered = abs(uv * 2.0 - vec2<f32>(1.0));
    let inset = 1.0 - max(centered.x, centered.y);
    result.edge = 1.0 - clamp(inset / SHADOW_CASCADE_BLEND, 0.0, 1.0);
    return result;
}

// Incidence below which a receiver stops being shadowed at all.
//
// A surface facing away from a light cannot be meaningfully shadowed by it, and the depth comparison
// there is least trustworthy: on a face nearly parallel to the light, the far side of a solid is
// laterally rather than deeply offset, so the near and far depths fall within a texel of each other and
// the test flickers. The direct term hides that, being near zero at such incidence -- but the *ambient*
// term is shadow-attenuated too and does not depend on incidence, so the flicker surfaces there as
// diagonal striping across the face.
//
// Fading the attenuation out as incidence approaches zero removes it at its cause rather than by
// inflating a bias, and costs nothing that was visible: the geometry it stops shadowing receives no
// direct light anyway.
const SHADOW_INCIDENCE_FADE: f32 = 0.22;

// Returns the raw unoccluded fraction in `0..=1`. Callers apply their own floor, because opaque
// terrain and the transmissive water surface keep different amounts of light in full shade.
//
// Cascades are fitted smallest first, so the first one that contains the receiver is also the
// densest one that does; testing them in order is both the selection rule and the coverage test.
// A receiver outside every cascade is beyond the shadowed distance and reads as fully lit. Near a
// cascade's outer boundary the next cascade is sampled too and the two are crossfaded, so only the
// band pays for a second lookup.
fn shadow_visibility(world_position: vec3<f32>, normal: vec3<f32>) -> f32 {
    for (var index = 0; index < SHADOW_CASCADE_COUNT; index += 1) {
        let current = sample_cascade(index, world_position, normal);
        if current.covered < 0.5 {
            continue;
        }
        if current.edge <= 0.0 || index + 1 >= SHADOW_CASCADE_COUNT {
            return current.visibility;
        }
        let outer = sample_cascade(index + 1, world_position, normal);
        if outer.covered < 0.5 {
            return current.visibility;
        }
        return mix(current.visibility, outer.visibility, current.edge);
    }
    return 1.0;
}

// The sky, as the two colours everything that needs one fades between.
//
// Named constants rather than literals at the point of use, because the water pass reflects this same
// sky. A reflection of a sky that is not the sky on screen is obvious in a screenshot and invisible
// from any assertion, so the two are made incapable of disagreeing rather than kept in step by hand.
const SKY_ZENITH: vec3<f32> = vec3<f32>(0.025, 0.04, 0.065);
const SKY_HORIZON: vec3<f32> = vec3<f32>(0.12, 0.20, 0.30);

// The sky along a world direction, for a reflection. The screen gradient below is the same two colours
// mixed by height in the frame instead, which is all a background needs.
fn sky_colour(direction: vec3<f32>) -> vec3<f32> {
    return mix(SKY_HORIZON, SKY_ZENITH, clamp(direction.z, 0.0, 1.0));
}

@fragment
fn lighting_fragment(input: FullscreenOutput) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(input.position.xy);
    let coverage = textureLoad(g_coverage, pixel, 0).r;
    if (coverage < 0.5) {
        // Pixel y grows downward, so this runs from the zenith at the top of the frame to the horizon
        // at the bottom.
        let horizon = clamp(input.position.y / camera.viewport.y, 0.0, 1.0);
        return vec4<f32>(mix(SKY_ZENITH, SKY_HORIZON, horizon), 1.0);
    }
    let world = world_at(pixel);
    let albedo = textureLoad(g_albedo, pixel, 0).rgb;
    let normal_roughness = textureLoad(g_normal, pixel, 0);
    let normal = normalize(normal_roughness.xyz);
    let view_direction = normalize(camera.camera_position.xyz - world);
    var primary_visibility = shadow_visibility(world, normal);
    // Fade toward fully lit at grazing incidence to the primary light. See SHADOW_INCIDENCE_FADE.
    let primary = camera.lights[0].source_direction.xyz;
    let primary_length = length(primary);
    if (primary_length > 0.00001) {
        let incidence = dot(normal, -primary / primary_length);
        let trust = smoothstep(0.0, SHADOW_INCIDENCE_FADE, incidence);
        primary_visibility = mix(1.0, primary_visibility, trust);
    }
    let occlusion = mix(
        AO_AMBIENT_FLOOR,
        1.0,
        textureLoad(ambient_occlusion, pixel, 0).r
    );
    var color = vec3<f32>(0.0);
    for (var index = 0; index < LIGHT_COUNT; index += 1) {
        let light = camera.lights[index];
        // Slot 0 is the primary and is the only shadowed light, in both its ambient and direct
        // shares. The rest are fills: shadowing them would darken the scene twice for one
        // occluder and defeat the purpose of having them.
        let shadowed = index == 0;
        // Only the primary light is both shadowed and occluded, so it is the only one where the two
        // could compound; the accent fills take occlusion alone and are unaffected by this.
        let ambient_scale = select(
            occlusion,
            primary_ambient_scale(primary_visibility, occlusion),
            shadowed
        );
        color += albedo * light.ambient.rgb * ambient_scale;
        let direction_length = length(light.source_direction.xyz);
        if (direction_length > 0.00001) {
            let visibility = select(
                1.0,
                mix(SHADOW_DIRECT_FLOOR, 1.0, primary_visibility),
                shadowed
            );
            let light_direction = -light.source_direction.xyz / direction_length;
            let diffuse_factor = max(dot(normal, light_direction), 0.0);
            color += albedo * light.diffuse.rgb * diffuse_factor * visibility;
            // Highlight strength falls off with roughness, not just its width, so a fully rough
            // material has no highlight at all. A fixed strength instead gives every surface a
            // sheen regardless of what its material declared, once per light.
            let half_vector = normalize(light_direction + view_direction);
            let specular = pow(
                max(dot(normal, half_vector), 0.0),
                mix(64.0, 8.0, normal_roughness.w)
            );
            let specular_strength = SPECULAR_STRENGTH * (1.0 - normal_roughness.w);
            color += light.diffuse.rgb * specular * specular_strength * visibility;
        }
    }
    // Self-illumination, decoded from the G-buffer coverage channel. Added after the light loop so
    // it survives full shade, which is the whole point of a lamp: the emitted term takes its hue
    // from the material's own albedo, and the intensity is the material's emissive strength.
    color += albedo * max(coverage - 1.0, 0.0);
    return vec4<f32>(color, 1.0);
}

@group(1) @binding(0) var scene_color: texture_2d<f32>;
@group(1) @binding(1) var scene_sampler: sampler;

// Scene exposure, applied before the tone curve.
//
// Reinhard maps 1.0 to 0.5, so an unexposed scene whose brightest surfaces sit near unity lands
// entirely in the lower half of the range and reads as flat and grey -- the same mistake as tone
// mapping the forward pass at all. Exposing first puts fully lit ground near the top of the range and
// leaves the curve doing what it is for: rolling off the values that genuinely exceed one.
const EXPOSURE: f32 = 1.6;

fn reinhard(hdr: vec3<f32>) -> vec3<f32> {
    let exposed = hdr * EXPOSURE;
    return exposed / (vec3<f32>(1.0) + exposed);
}

// A contrast-adaptive sharpen in the spirit of AMD FidelityFX CAS: it boosts an unsharp-mask
// style detail term by an amount that scales down toward zero both near luminance extremes
// (avoids blooming/crushing) and at genuinely hard edges (avoids ringing on silhouette
// edges), so it only restores softer mid-contrast detail lost to texture filtering. It is not an
// antialiasing pass and does not pretend to be one.
const SHARPEN_STRENGTH: f32 = 0.6;

@fragment
fn composite_fragment(input: FullscreenOutput) -> @location(0) vec4<f32> {
    let inverse_viewport = camera.viewport.zw;
    let uv = (input.position.xy + vec2<f32>(0.5)) * inverse_viewport;
    let center = reinhard(textureSampleLevel(scene_color, scene_sampler, uv, 0.0).rgb);
    let north = reinhard(textureSampleLevel(
        scene_color,
        scene_sampler,
        uv + vec2<f32>(0.0, -inverse_viewport.y),
        0.0
    ).rgb);
    let south = reinhard(textureSampleLevel(
        scene_color,
        scene_sampler,
        uv + vec2<f32>(0.0, inverse_viewport.y),
        0.0
    ).rgb);
    let west = reinhard(textureSampleLevel(
        scene_color,
        scene_sampler,
        uv + vec2<f32>(-inverse_viewport.x, 0.0),
        0.0
    ).rgb);
    let east = reinhard(textureSampleLevel(
        scene_color,
        scene_sampler,
        uv + vec2<f32>(inverse_viewport.x, 0.0),
        0.0
    ).rgb);
    let minimum = min(center, min(min(north, south), min(west, east)));
    let maximum = max(center, max(max(north, south), max(west, east)));
    let peak = min(minimum, vec3<f32>(1.0) - maximum) / max(maximum, vec3<f32>(0.001));
    let amplitude = sqrt(clamp(peak, vec3<f32>(0.0), vec3<f32>(1.0))) * SHARPEN_STRENGTH;
    let neighbor_average = (north + south + west + east) * 0.25;
    let sharpened = center + (center - neighbor_average) * amplitude;
    return vec4<f32>(clamp(sharpened, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}

// -------------------------------------------------------------------------------------------------
// Water
//
// A bounded plane with procedural waves, drawn between lighting and the composite and blended into the
// HDR target -- so it tone maps with everything else instead of being composited over an
// already-curved image, which is what would make a bright highlight on it clip rather than roll off.
//
// See `water.rs` for where the surface data comes from, why it is renderer state rather than terrain
// data, and the provenance note that applies to every constant here.
// -------------------------------------------------------------------------------------------------

struct Water {
    // xy the minimum corner, zw the maximum, in world units.
    bounds: vec4<f32>,
    // x mean surface elevation, y time in seconds, z and w grid cells along each axis.
    surface: vec4<f32>,
    // rgb the shallow tint, w the depth over which it reaches the deep tint.
    shallow: vec4<f32>,
    // rgb the deep tint, w the depth over which the shoreline reaches full opacity.
    deep: vec4<f32>,
    // x wave amplitude, y dominant wavelength, z travel speed, w surface roughness.
    waves: vec4<f32>,
}

// Group 2, not 1. Group 1 in this module is the composite's scene colour, and one module cannot bind
// two different resources to the same slot; the water pipeline leaves group 1 empty.
@group(2) @binding(0) var<uniform> water: Water;

const WATER_WAVE_COUNT: i32 = 5;
const TAU: f32 = 6.2831853;

// Successive wave directions are advanced by the golden angle.
//
// Any rational fraction of a turn eventually puts two waves on a shared axis, and a shared axis is
// what builds a lattice; this angle never repeats, so no two of the five ever line up. Four directions
// at 90-degree steps is the worst case and reads as a tiled texture rather than as a surface.
const WATER_GOLDEN_ANGLE: f32 = 2.399963;

// How much of the primary light's *ambient* share survives where the surface is shadowed.
//
// Higher than the terrain's SHADOW_AMBIENT_FLOOR, and not because water is brighter. That constant is
// tuned against an occlusion term that darkens terrain as well, and water is not in the G-buffer so it
// receives no occlusion at all. Reusing the terrain figure would sink a shadowed lake below the
// shadowed ground beside it.
const WATER_AMBIENT_FLOOR: f32 = 0.45;

// The direct and specular floor, which is far lower. A shadow genuinely does extinguish sun glitter,
// and that is the most visible thing a shadow does to water.
const WATER_DIRECT_FLOOR: f32 = 0.05;

// Water's reflectance at normal incidence. Everything about how water reads follows from this being
// small: from overhead it is almost entirely transmissive, and at a grazing angle almost a mirror.
const WATER_F0: f32 = 0.02;

// Peak glitter strength. Deliberately above one: the sun's reflection is the brightest thing in a
// daylit scene, and the HDR target and the composite's tone curve exist so a value like this rolls off
// instead of clipping to white.
const WATER_SPECULAR_STRENGTH: f32 = 1.4;

struct WaveSample {
    height: f32,
    // Partial derivatives of height with respect to world x and y.
    gradient: vec2<f32>,
}

// Height and slope of the summed wave train at a world position.
//
// Both come out of one call because the gradient is the analytic derivative of the same sum -- the
// cosines cost nothing extra once the sines are being taken, and an analytic normal is exact at any
// tessellation, which matters because the grid density is chosen from the wavelength rather than fixed.
fn wave_sample(xy: vec2<f32>, time: f32) -> WaveSample {
    let amplitude = water.waves.x;
    let base_length = max(water.waves.y, 0.001);
    let speed = water.waves.z;

    // Each wave's share of the dominant wavelength, in steps of about 0.61 rather than 1/2, 1/3, 1/4.
    // Near-harmonic ratios reinforce at regular intervals, and that interference *is* the visible
    // lattice that makes a summed-sine surface look tiled; an irrational-ish ratio never closes.
    var lengths = array<f32, 5>(1.0, 0.61, 0.372, 0.227, 0.138);
    // Amplitudes proportional to wavelength, which holds steepness constant across the five so the
    // short chop is not near-vertical, and normalised to total one — so `wave_height` really is the
    // crest height it claims to be rather than a figure the sum then overshoots by 2.3 times.
    var weights = array<f32, 5>(0.426, 0.260, 0.158, 0.097, 0.059);

    var result: WaveSample;
    result.height = 0.0;
    result.gradient = vec2<f32>(0.0);
    for (var index = 0; index < WATER_WAVE_COUNT; index += 1) {
        let angle = f32(index) * WATER_GOLDEN_ANGLE;
        let direction = vec2<f32>(cos(angle), sin(angle));
        let wavenumber = TAU / (base_length * lengths[index]);
        let wave_amplitude = amplitude * weights[index];
        // Deep-water gravity waves travel at a speed proportional to the square root of their
        // wavelength, so the long swell outruns the short chop. Giving every wave one speed instead
        // slides the whole sum rigidly across the map, which reads as a scrolling texture.
        let phase_speed = speed * sqrt(lengths[index]);
        let phase = wavenumber * (dot(direction, xy) - phase_speed * time);
        result.height += wave_amplitude * sin(phase);
        result.gradient += direction * (wave_amplitude * wavenumber * cos(phase));
    }
    return result;
}

fn wave_normal(gradient: vec2<f32>) -> vec3<f32> {
    return normalize(vec3<f32>(-gradient.x, -gradient.y, 1.0));
}

struct WaterVertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
}

// The grid, procedural exactly as the terrain's is, in the same six-corner ring order.
//
// The order is load-bearing: walking the quad in Z order instead swaps the last two corners, which
// shears anything mapped across the surface along a diagonal. That bug sat in two terrain fixtures
// unnoticed for as long as nothing sampled through them.
@vertex
fn water_vertex(@builtin(vertex_index) vertex_index: u32) -> WaterVertexOutput {
    let cells = max(vec2<u32>(u32(water.surface.z), u32(water.surface.w)), vec2<u32>(1u));
    let quad = vertex_index / 6u;
    let corner = vertex_index % 6u;
    let cell = vec2<u32>(quad % cells.x, quad / cells.x);
    var offsets = array<vec2<u32>, 6>(
        vec2<u32>(0u, 0u),
        vec2<u32>(1u, 0u),
        vec2<u32>(0u, 1u),
        vec2<u32>(1u, 0u),
        vec2<u32>(1u, 1u),
        vec2<u32>(0u, 1u),
    );
    let grid = vec2<f32>(cell + offsets[corner]);
    let span = water.bounds.zw - water.bounds.xy;
    let xy = water.bounds.xy + grid / vec2<f32>(cells) * span;
    let waves = wave_sample(xy, water.surface.y);

    var output: WaterVertexOutput;
    output.world_position = vec3<f32>(xy, water.surface.x + waves.height);
    output.clip_position = camera.view_projection * vec4<f32>(output.world_position, 1.0);
    return output;
}

@fragment
fn water_fragment(input: WaterVertexOutput) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(input.clip_position.xy);
    let scene = textureLoad(scene_depth, pixel, 0);

    // The depth test, done here rather than by the rasterizer. This pass cannot *attach* the scene
    // depth buffer: the lighting bind group it shares already binds that same texture for sampling,
    // and a texture cannot be both an attachment and a bound resource in one pass. Comparing against
    // the stored value is the same `Less` test the hardware would have run.
    //
    // `textureSampleCompareLevel` inside `shadow_visibility` takes an explicit mip level, so
    // discarding here does not put a derivative-dependent sample in non-uniform control flow.
    if (scene < input.clip_position.z) {
        discard;
    }

    // No bed means no map. Coverage is zero only *outside* the terrain's footprint, so a body whose
    // rectangle overhangs the heightfield would otherwise draw as a slab hanging past the map edge --
    // plainly visible under the terrain's own boundary, because terrain is an open sheet rather than a
    // solid. Water is therefore clipped to where terrain exists. A coastal map wanting open sea past
    // the shore extends its heightfield under that sea, which is what the shoreline clip below needs
    // from it in any case.
    if (textureLoad(g_coverage, pixel, 0).r < 0.5) {
        discard;
    }

    let bed = world_from_depth(pixel, scene);
    // Measured from the *displaced* surface rather than the mean elevation, so the shoreline advances
    // and retreats with the swell instead of sitting on a fixed contour.
    let depth = input.world_position.z - bed.z;
    if (depth <= 0.0) {
        discard;
    }

    let waves = wave_sample(input.world_position.xy, water.surface.y);
    let normal = wave_normal(waves.gradient);
    let view_direction = normalize(camera.camera_position.xyz - input.world_position);
    let visibility = shadow_visibility(input.world_position, normal);

    // The transmitted term, tinted by how much water the view looks through.
    let tint = mix(
        water.shallow.rgb,
        water.deep.rgb,
        clamp(depth / max(water.shallow.w, 0.001), 0.0, 1.0)
    );
    let light = camera.lights[0];
    var body = tint * light.ambient.rgb * mix(WATER_AMBIENT_FLOOR, 1.0, visibility);
    var glitter = vec3<f32>(0.0);
    let primary_length = length(light.source_direction.xyz);
    if (primary_length > 0.00001) {
        let light_direction = -light.source_direction.xyz / primary_length;
        let shaded = mix(WATER_DIRECT_FLOOR, 1.0, visibility);
        body += tint * light.diffuse.rgb * max(dot(normal, light_direction), 0.0) * shaded;
        // A tight highlight, driven by the material's roughness, so a choppy lake and a still one
        // differ in the size of the glitter and not only in how high the waves stand.
        //
        // The upper bound is 720 and not something mirror-like in the thousands. An exponent of 2,000
        // is a lobe under two degrees wide, and against wave normals that only stray about fifteen
        // degrees it lands on almost no pixel at all — the surface then renders matte, and the glitter
        // that makes water read as water is *present in the shader and invisible in the frame*.
        let half_vector = normalize(light_direction + view_direction);
        let gloss = mix(720.0, 48.0, clamp(water.waves.w, 0.0, 1.0));
        glitter = light.diffuse.rgb
            * pow(max(dot(normal, half_vector), 0.0), gloss)
            * WATER_SPECULAR_STRENGTH
            * shaded;
    }

    // The reflected term, deliberately *not* shadow-attenuated. What a cast shadow occludes is the
    // sun, not the sky, so shadowed water keeps reflecting and still reads as water rather than as a
    // dark hole in the terrain.
    let incidence = clamp(dot(normal, view_direction), 0.0, 1.0);
    let fresnel = WATER_F0 + (1.0 - WATER_F0) * pow(1.0 - incidence, 5.0);
    let reflected = sky_colour(reflect(-view_direction, normal));

    let colour = mix(body, reflected, fresnel) + glitter;
    // Opacity is the greater of what depth and what reflectance imply. A shallow edge seen from
    // overhead is nearly clear, but the same edge seen at a grazing angle is a mirror, and an alpha
    // taken from depth alone would fade out the far shore of every lake.
    let alpha = max(clamp(depth / max(water.deep.w, 0.001), 0.0, 1.0), fresnel);
    return vec4<f32>(colour, alpha);
}
