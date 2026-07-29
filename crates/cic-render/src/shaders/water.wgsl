// The water surface: a bounded plane with procedural waves.
//
// A composition chunk. Requires `scene.wgsl`, `shadow.wgsl`, `atmosphere.wgsl` and `reflection.wgsl` --
// which is the whole reason composition exists. Before it, water had to share one file with the lighting
// and composite passes to reach `shadow_visibility` and `world_from_depth`, because WGSL has no include
// mechanism.
//
// Drawn between lighting and the composite and blended into the HDR target, so it tone maps with
// everything else rather than being composited over an already-curved image -- which is what would make
// a bright highlight on it clip instead of rolling off.
//
// See `water.rs` for where the surface data comes from, why it is renderer state rather than terrain
// data, and the provenance note that applies to every constant here.

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

// Group 1. It was group 2 before composition existed, because water then shared a module with the
// composite pass, which owns group 1 there -- and one module cannot bind two different resources to the
// same slot. Its own module means its own slots, and the pipeline layout no longer needs an empty gap.
@group(1) @binding(0) var<uniform> water: Water;

const WATER_WAVE_COUNT: i32 = 5;
// `TAU` comes from `atmosphere.wgsl`, which every program composing this one also composes.

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
        // The same cloud term the ground gets, on the same direct share. Skipping it here would leave a
        // lake glittering under a deck that has visibly shaded every field around it.
        let shaded = mix(WATER_DIRECT_FLOOR, 1.0, visibility)
            * cloud_shadow(input.world_position.xy);
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
    let reflected = reflection_colour(input.world_position, normal, view_direction);

    // Fogged like any other surface, and before the alpha is decided. Water that ignores fog while the
    // shore beside it fades out is the most conspicuous way to break a foggy scene.
    let colour = apply_fog(mix(body, reflected, fresnel) + glitter, input.world_position);
    // Opacity is the greater of what depth and what reflectance imply. A shallow edge seen from
    // overhead is nearly clear, but the same edge seen at a grazing angle is a mirror, and an alpha
    // taken from depth alone would fade out the far shore of every lake.
    let alpha = max(clamp(depth / max(water.deep.w, 0.001), 0.0, 1.0), fresnel);
    return vec4<f32>(colour, alpha);
}
