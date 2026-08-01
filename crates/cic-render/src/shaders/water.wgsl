// The water surface: a bounded plane with procedural waves.
//
// A composition chunk. Requires `scene.wgsl`, `scene_colour.wgsl`, `shadow.wgsl`, `sky.wgsl`,
// `atmosphere.wgsl` and one `reflection_*.wgsl` provider --
// which is the whole reason composition exists. Before it, water had to share one file with the lighting
// and composite passes to reach `shadow_visibility` and `world_from_depth`, because WGSL has no include
// mechanism.
//
// Drawn between lighting and the composite and blended into the HDR target, so it tone maps with
// everything else rather than being composited over an already-curved image -- which is what would make
// a bright highlight on it clip instead of rolling off.
//
// See `water.rs` for where the surface data comes from, why it is renderer state rather than terrain
// data, the three kinds of body the presets describe, and the provenance note that applies to every
// constant here.

struct Water {
    // xy the minimum corner, zw the maximum, in world units.
    bounds: vec4<f32>,
    // x mean surface elevation, y time in seconds, z and w grid cells along each axis.
    surface: vec4<f32>,
    // rgb the shallow tint, w the depth over which it reaches the deep tint.
    shallow: vec4<f32>,
    // rgb the deep tint, w unused.
    //
    // `w` carried a shoreline feather until absorption took over deciding opacity. It is left in place
    // rather than repacked because every field after it would move, and a silent misalignment of this
    // block is the failure the size assertions exist for.
    deep: vec4<f32>,
    // x wave amplitude, y dominant wavelength, z travel speed, w surface roughness.
    waves: vec4<f32>,
    // xy the unit heading the train runs along, z the half-angle its directions spread over in
    // radians, w how far the crests are peaked, in 0..1.
    //
    // The spread is what separates the three kinds more than any other figure: a river's train is
    // nearly collimated down the channel, an ocean swell fans out about the wind, and a lake is
    // isotropic because nothing over it has a preferred direction for long enough to matter.
    train: vec4<f32>,
    // xy the current the whole surface is carried along by, in world units per second, z the depth
    // over which shore foam fades out, w how much foam there is at all.
    current: vec4<f32>,
    // x how much whitecap breaks on the tallest crests in open water, y how far the bed is displaced
    // by refraction in world units, z how far the reflectance is lifted above the physical, w reserved.
    //
    // The whitecap share is separate from the shore's and not a fraction of it: a lake laps at its
    // edge and never breaks in the middle, and an ocean does both, so one number cannot carry them.
    foam: vec4<f32>,
}

// Group 1. It was group 2 before composition existed, because water then shared a module with the
// composite pass, which owns group 1 there -- and one module cannot bind two different resources to the
// same slot. Its own module means its own slots, and the pipeline layout no longer needs an empty gap.
@group(1) @binding(0) var<uniform> water: Water;

// `TAU` comes from `sky.wgsl`, which every program composing this one also composes.

// Components in the summed train.
//
// Twelve rather than five. Five is enough to make a surface that *moves* and visibly too few to make
// one that does not read as a pattern: with only five the sum has a short beat, and the capture of a
// still lake showed unmistakable diagonal banding across the whole body. Nine fixed the banding and
// left the surface reading as *too even* — real water is not a tidy superposition, and the three
// components past nine are what carry the fine ripple a near camera resolves. They are also the
// answer to wanting a detail normal map: the same detail, from the same generator, with no texture to
// author, no tiling to hide, and the level-of-detail below already knowing how to remove it with
// distance. The cost is twelve sine-cosine pairs per vertex and per shaded pixel, against two texture
// loads and a shadow lookup the same fragment already pays for.
const WATER_WAVE_COUNT: i32 = 12;

// Reciprocal of the twelve wavelength shares summed, which is each component's steepness: crest height
// over wavelength.
//
// Amplitudes are `steepness * wavelength` rather than a second table. Holding the ratio constant is
// what keeps the short chop from standing near-vertical, and deriving one table from the other makes
// that structural instead of a claim two tables have to keep agreeing about. Normalising by the sum is
// what makes `wave_height` the crest height it says it is rather than a figure the sum overshoots.
const WATER_STEEPNESS: f32 = 0.226474;

// How deep the wave groups cut, and how long they are relative to the component they modulate.
//
// This is the answer to a summed sine train reading as corduroy however many components it has. Real
// water arrives in *groups*: a set of crests builds, runs, and fades, and a crest has a finite length
// along its own ridge rather than running unbroken to the horizon. A pure sum has neither property —
// every component is an infinite plane wave of constant amplitude, so the sum is exactly as strong
// everywhere and its crests never end. That is what "too regular" is, and adding components does not
// touch it, because the sum of any number of infinite plane waves is still stationary.
//
// So each component carries a slow envelope, and the envelope varies mostly *across* the direction of
// travel — which is what breaks a ridge into segments rather than merely making the whole train pulse.
// A depth of 0.4 takes a component down to a fifth of its amplitude at the quietest and never above
// its nominal one, so the envelope only ever removes energy. The scale is the envelope's wavenumber as
// a fraction of the component's own: at 0.19 a group runs about five crests, which is roughly what a
// real sea does.
const WATER_GROUP_DEPTH: f32 = 0.40;
const WATER_GROUP_SCALE: f32 = 0.19;
// A little of the envelope runs *along* travel too, so groups advance instead of standing as fixed
// lanes on the map.
const WATER_GROUP_ALONG: f32 = 0.37;
// How fast the envelope drifts, against the wave's own phase speed. Well under one: a group travels at
// the group velocity, which for deep-water gravity waves is half the phase velocity, and slower still
// reads better because a group that keeps pace with its own crests is not a group.
const WATER_GROUP_SPEED: f32 = 0.35;

// The fraction the component directions are spread by, and the one their phases are offset by.
//
// Two different irrationals, so the two sequences are not each other. The golden ratio's conjugate is
// the standard low-discrepancy generator -- it fills an interval about as evenly as anything can
// without ever repeating -- and `fract(index * it + 0.5)` puts the longest component exactly on the
// heading, which is what makes a river's dominant wave run down the channel rather than across it.
// The plastic number's conjugate is the second generator of the same family and is used for the
// phases, so the components are not all crossing zero together at the world origin.
const WATER_DIRECTION_FRACTION: f32 = 0.618034;
const WATER_PHASE_FRACTION: f32 = 0.754878;

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

// The reflectance a fully stylised surface is given instead, and the honest name for it is a lie.
//
// The physical figure is correct and it is why a reflection is nearly invisible from a playing camera.
// The reflected share goes as the fifth power of the incidence: about seventy percent at eight degrees
// above the surface, five at thirty, three at forty. An RTS camera lives at the last of those, so a
// *perfect* mirror would contribute three percent of each water pixel. That was measured rather than
// reasoned -- switching the whole scene between the sky and screen-space providers moved 1.8% of pixels
// by a mean of 0.17 of 255, and the two frames are indistinguishable side by side.
//
// Lifting F0 is the right shape for that exaggeration, and better than scaling the result. The Fresnel
// curve is already near one at a grazing angle, so raising its *base* adds reflection where there is
// almost none and leaves the case that already worked alone; multiplying the share instead would
// saturate a grazing view into a flat mirror while barely touching the overhead one. This is the
// standard artistic F0 knob, and it is exposed per material because how far to depart from physics is
// a decision about a scene rather than a property of water.
const WATER_F0_STYLISED: f32 = 0.35;

// RMS slope of the wave train, in units of crest height over dominant wavelength.
//
// A derived figure rather than a tuned one, and it has moved twice for reasons worth keeping. Every
// component contributes the same slope, because they share `WATER_STEEPNESS`:
// `steepness * TAU * height / length`. Twelve independent sinusoids sum in quadrature to
// `sqrt(12 / 2)` of one, giving `0.226474 * 6.283185 * 2.449490 = 3.485`.
//
// The group envelope then takes energy out and never adds it, so the figure has to come down by the
// envelope's own RMS: a term running `1 - d + d * cos` has mean square `(1 - d)^2 + d^2 / 2`, which at
// `d = 0.40` is `0.36 + 0.08 = 0.44` and an RMS of 0.663. So `3.485 * 0.663 = 2.31`.
//
// What that neglects is the slope of the envelope itself, which is real and is second order: the
// envelope's wavenumber is `WATER_GROUP_SCALE` of the component's, so its contribution is under a
// tenth of the component's own and lands inside the quadrature sum rather than beside it.
const WATER_SLOPE_RMS: f32 = 2.31;

// The reflection half-angle a fully rough surface spans, in radians. About twenty degrees, which is the
// lobe the gloss exponents below describe at the rough end.
const WATER_ROUGHNESS_CONE: f32 = 0.35;

// Peak glitter strength. Deliberately above one: the sun's reflection is the brightest thing in a
// daylit scene, and the HDR target and the composite's tone curve exist so a value like this rolls off
// instead of clipping to white.
const WATER_SPECULAR_STRENGTH: f32 = 1.4;

// Where a component stops contributing slope, as a fraction of its own wavelength covered by one pixel.
//
// A component is at the Nyquist limit when a pixel spans half its wavelength, and it is already
// unreliable well before that, so the fade runs from a quarter to a half. Below a quarter -- four
// pixels or more across a wave -- the analytic normal is honest and is kept at full strength.
const WATER_DETAIL_FADE_START: f32 = 0.25;
const WATER_DETAIL_FADE_END: f32 = 0.5;

// How deep the refraction offset keeps growing with, in world units.
//
// The lateral shift of a refracted ray goes as the distance it travels through the water, so it grows
// with depth -- but only while the bed is still visible. Past this the tint has closed over it and a
// larger offset would move a bed nobody can see, so the figure is where the effect stops mattering
// rather than where the physics stops holding.
const WATER_REFRACTION_DEPTH: f32 = 8.0;

// Foam is not quite white. Sea foam is a froth of water and air and takes a little of the sky, and a
// pure white band along a shore reads as a clipped highlight rather than as surf.
const WATER_FOAM_COLOUR: vec3<f32> = vec3<f32>(0.92, 0.95, 0.96);

// Where along the wave the foam appears, as a fraction of crest-to-trough with the mean surface at a
// half. Both windows are placed against the *distribution* of that figure rather than against its
// range, and getting that wrong is why the first attempt at whitecaps put one speck on a whole sea.
//
// The sum of twelve enveloped components is very far from uniform over `0..=1`: it is a sum of
// independent terms, so it piles up around the mean, and its standard deviation works out at about
// 0.086 of the range. A threshold at 0.78 is therefore past three sigma — a figure the surface
// reaches on a handful of pixels in a frame.
//
// Below the first figure the water is in a trough and drawing back, above the second it is under a
// crest and breaking. The band between them is what makes the foam pulse along the shoreline instead
// of ringing it evenly.
const WATER_FOAM_CREST_START: f32 = 0.35;
const WATER_FOAM_CREST_END: f32 = 0.62;

// Where a whitecap breaks, on the same scale. Deliberately above the shore band: at the shore any
// crest arriving is a crest breaking, and in open water only the ones standing well above the rest
// are. Two sigma to three, so a few percent of the surface is breaking at any moment.
const WATER_WHITECAP_START: f32 = 0.62;
const WATER_WHITECAP_END: f32 = 0.74;

// And how steep that crest has to be, as a multiple of the train's own RMS slope.
//
// Height alone is not enough and the capture said so: keyed on height only, an ocean's whitecaps come
// out the size of the swell that carries them — hundred-unit blobs that read as ice floes rather than
// as broken water. The reason is that height is dominated by the longest components, so a "high"
// region is a whole crest wide.
//
// Slope is the physical criterion anyway. A wave breaks when its face is too steep to stand, not when
// it is tall, and the gradient carries every component — so gating on it fragments the patch down to
// the scale of the chop riding on the swell, which is the scale whitecaps actually have.
const WATER_WHITECAP_SLOPE_START: f32 = 1.15;
const WATER_WHITECAP_SLOPE_END: f32 = 1.85;

struct WaveSample {
    height: f32,
    // Partial derivatives of height with respect to world x and y, damped for the shading footprint
    // this sample was taken for. Equal to the true analytic derivative when the footprint is zero.
    gradient: vec2<f32>,
}

// How much of a component of this wavelength a pixel spanning `footprint` world units can still resolve.
//
// This is the whole of the normal level-of-detail. Without it the shading normal carries every
// component at full strength however little of the train a pixel covers, and the Fresnel term is
// violently sensitive to that normal at a grazing view: near the horizon a six-degree slope change
// moves the reflected share from about 0.19 to 0.72, so neighbouring pixels alternate between the
// bright sky and the dark body and the surface reads as scattered dark speckle rather than as water.
// Damping per component rather than damping the whole gradient by the shortest one is what keeps a
// distant swell shaped while the chop riding on it goes flat: the two differ by seven times in
// wavelength, so they stop being resolvable seven times apart.
//
// What is lost here is not lost from the frame. The reflection cone below is built from the material's
// *whole* slope RMS and does not consult the normal, so the variance this removes from the geometry is
// exactly the variance that term already spends widening the lobe -- a distant surface comes out
// flat-but-rough, which is what water at that distance is.
fn wave_detail(footprint: f32, wave_length: f32) -> f32 {
    let covered = footprint / max(wave_length, 0.0001);
    return 1.0 - smoothstep(WATER_DETAIL_FADE_START, WATER_DETAIL_FADE_END, covered);
}

// Height and slope of the summed wave train at a world position, for a pixel covering `footprint`
// world units. Pass zero for the footprint where there is no pixel -- the vertex stage displaces
// geometry, which is not a shading question and must not be level-of-detailed.
//
// Both come out of one call because the gradient is the analytic derivative of the same sum -- the
// cosines cost nothing extra once the sines are being taken, and an analytic normal is exact at any
// tessellation, which matters because the grid density is chosen from the wavelength rather than fixed.
fn wave_sample(xy: vec2<f32>, time: f32, footprint: f32) -> WaveSample {
    let amplitude = water.waves.x;
    let base_length = max(water.waves.y, 0.001);
    let speed = water.waves.z;
    let heading = water.train.xy;
    let spread = water.train.z;
    // A quarter of the peaking, because 0.25 is the harmonic coefficient at which a second-order
    // Stokes wave's trough goes exactly flat -- so the exposed parameter reaches that limit at one.
    // Past it the trough grows a dimple in the middle, which no water does.
    let peak = clamp(water.train.w, 0.0, 1.0) * 0.25;

    // The whole surface is carried downstream before it is sampled. A river reads as flowing because
    // its water moves, not only because its waves do, and the two are different speeds: chop travels
    // over a current that is itself translating. A pure translation leaves the derivative alone, so
    // this costs the gradient nothing.
    let position = xy - water.current.xy * time;

    // Each component's share of the dominant wavelength, as `1 / golden^(index / 2)`.
    //
    // The ratio between *any* two of them is an irrational power of the golden ratio, so no two ever
    // line up again after the origin. That is the property that matters, and it is stronger than the
    // one the first version had: stepping by a fixed 0.61 makes neighbours irrational but leaves the
    // pair 1 and 0.372 close enough to 8:3 to beat visibly across a map. The band spans about 7:1, so
    // a 24-unit dominant wavelength carries chop down to three and a half units.
    var lengths = array<f32, 12>(
        1.0,
        0.786151,
        0.618034,
        0.485868,
        0.381966,
        0.300283,
        0.236068,
        0.185575,
        0.145898,
        0.114692,
        0.090170,
        0.070880,
    );

    var result: WaveSample;
    result.height = 0.0;
    result.gradient = vec2<f32>(0.0);
    for (var index = 0; index < WATER_WAVE_COUNT; index += 1) {
        let share = lengths[index];
        let wave_length = base_length * share;
        // Lean off the heading by a low-discrepancy fraction of the spread, rather than by a fixed
        // step around the whole circle. Any rational fraction of a turn eventually puts two components
        // on one axis, and a shared axis is what builds the lattice that makes a summed-sine surface
        // read as a tiled texture; this sequence never repeats and never lands symmetric.
        let lean = fract(f32(index) * WATER_DIRECTION_FRACTION + 0.5) * 2.0 - 1.0;
        let turn = spread * lean;
        let turn_cos = cos(turn);
        let turn_sin = sin(turn);
        let direction = vec2<f32>(
            heading.x * turn_cos - heading.y * turn_sin,
            heading.x * turn_sin + heading.y * turn_cos,
        );
        let wavenumber = TAU / wave_length;
        let wave_amplitude = amplitude * WATER_STEEPNESS * share;
        // Deep-water gravity waves travel at a speed proportional to the square root of their
        // wavelength, so the long swell outruns the short chop. Giving every wave one speed instead
        // slides the whole sum rigidly across the map, which reads as a scrolling texture.
        let phase_speed = speed * sqrt(share);
        let offset = TAU * fract(f32(index) * WATER_PHASE_FRACTION);
        let phase = wavenumber * (dot(direction, position) - phase_speed * time) + offset;
        let phase_sin = sin(phase);
        let phase_cos = cos(phase);
        // The harmonic by the double-angle identities rather than by two more transcendentals. The
        // arithmetic is exactly `sin(2p)` and `cos(2p)`, and it is four multiplies against a pair of
        // sines the shader takes twelve times over.
        let double_sin = 2.0 * phase_sin * phase_cos;
        let double_cos = 1.0 - 2.0 * phase_sin * phase_sin;

        // The group envelope. Its wavevector runs mostly *across* the component's travel, so what it
        // cuts is the length of a crest rather than the strength of the whole train -- a ridge that
        // fades out and picks up again further along, which is what stops a summed train reading as
        // corduroy. A little of it runs along travel as well, so the groups drift instead of standing
        // as fixed lanes over the map, and the second irrational offsets each component's envelope
        // from the others so they do not all fade together.
        let across = vec2<f32>(-direction.y, direction.x);
        let group_number = wavenumber * WATER_GROUP_SCALE;
        let group_vector = group_number * (across + WATER_GROUP_ALONG * direction);
        let group_phase = dot(group_vector, position)
            - group_number * phase_speed * WATER_GROUP_SPEED * time
            + offset * 2.0;
        // Between `1 - 2 * depth` and one, so the envelope only ever removes energy -- which is what
        // keeps `wave_height` an upper bound on the crest rather than a figure the groups overshoot.
        let envelope = 1.0 - WATER_GROUP_DEPTH + WATER_GROUP_DEPTH * cos(group_phase);
        let envelope_slope = -WATER_GROUP_DEPTH * sin(group_phase);

        // A second-order Stokes wave rather than a plain sine: the harmonic sharpens the crest and
        // flattens the trough, which is what a steep gravity wave actually does and what separates an
        // ocean swell from a pond ripple in silhouette. It shifts neither the mean surface nor the
        // crest-to-trough height, so `wave_height` still means what it says.
        let profile = phase_sin - peak * double_cos;
        result.height += wave_amplitude * envelope * profile;

        // The product rule, because the envelope is a function of position too. Dropping its term
        // would leave the normal describing a surface the height field no longer has -- subtly, and
        // exactly at the edges of a group where the envelope moves fastest.
        //
        // The harmonic has twice the wavenumber, so it stops being resolvable at twice the distance
        // and is damped against half the wavelength. The envelope has a fraction of it and outlives
        // both, which is right: a group is still visible long after the crests inside it are not.
        let detail = wave_detail(footprint, wave_length);
        let harmonic_detail = wave_detail(footprint, wave_length * 0.5);
        let group_detail = wave_detail(footprint, wave_length / WATER_GROUP_SCALE);
        let profile_slope = detail * phase_cos + harmonic_detail * 2.0 * peak * double_sin;
        result.gradient += wave_amplitude
            * (direction * (wavenumber * envelope * profile_slope)
                + group_vector * (group_detail * envelope_slope * profile));
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
    let waves = wave_sample(xy, water.surface.y, 0.0);

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

    // How much of the surface this pixel covers, in world units, taken across the quad. The wider of
    // the two screen axes rather than their average: at a grazing view one of them is many times the
    // other, and averaging would leave the long axis aliasing, which is the axis the sparkle was on.
    //
    // The derivatives sit here, after three `discard` statements, and that is sound. A `discard`
    // demotes the invocation to a helper rather than branching around the code below it, so this is
    // still uniform control flow and every lane in the quad still carries a world position -- which is
    // the same reason an alpha-tested pass may sample a mipped texture after its own cutout test.
    let world_dx = dpdx(input.world_position.xy);
    let world_dy = dpdy(input.world_position.xy);
    let footprint = max(length(world_dx), length(world_dy));

    let waves = wave_sample(input.world_position.xy, water.surface.y, footprint);
    let normal = wave_normal(waves.gradient);
    let view_direction = normalize(camera.camera_position.xyz - input.world_position);
    let visibility = shadow_visibility(input.world_position, normal);

    // What is under the surface, displaced by the wave normal.
    //
    // Water bends what it transmits, and a wave face is a lens that moves the bed sideways -- so a
    // stone under a ripple wobbles. Without this the transmitted term is a flat tint over a bed that
    // sits perfectly still under a moving surface, which is the most direct way to make water read as
    // coloured glass laid over the ground rather than as a liquid.
    //
    // Snell's law is not evaluated here. The offset is proportional to the normal's horizontal part,
    // which is its small-angle behaviour and is what the eye reads; a full refraction vector would
    // need the bed's *distance* along it, which is the thing a depth buffer cannot answer without
    // another march. The strength is in world units and is scaled down as the water shallows, because
    // a bed a hand's breadth under the surface cannot be displaced a metre without sliding visibly
    // out from under its own shoreline.
    //
    // The displacement is built in *world* space and then projected, rather than being added to a
    // texture coordinate. A first attempt added it straight to the UV scaled by the reciprocal
    // viewport, which silently treats a figure in world units as a figure in pixels -- the two differ
    // by the projection and by the distance to the water, so the offset came out around a hundredth of
    // a pixel and the whole term did nothing. Every reference image passed unchanged, which is exactly
    // how a dimensional error hides.
    //
    // How far the bed moves goes as the depth: a wave face bends the ray by an angle, and an angle
    // sweeps further the further it travels. Capped, because that relation holds for the shallows this
    // is visible in and would put the bed of a deep lake tens of units sideways.
    let bend = min(depth, WATER_REFRACTION_DEPTH) * water.foam.y;
    let displaced = project_to_screen(bed - vec3<f32>(waves.gradient * bend, 0.0));
    let own_uv = (vec2<f32>(pixel) + vec2<f32>(0.5)) * camera.viewport.zw;
    // A displaced sample that left the frame falls back to this pixel's own bed. Clamping the
    // coordinate instead would smear the border pixel along the edge of the water.
    let bed_uv = select(own_uv, displaced.uv, displaced.on_screen);
    let bed_colour = scene_colour_at(bed_uv);

    // How much of what is under the surface the water has absorbed, and the only depth ramp there is.
    //
    // One figure drives both the tint and the opacity, because they are the same physics: light
    // crossing water is absorbed along its path, which both shifts its colour and hides what it came
    // from. Two ramps is what this had, and the shorter of the two -- a shoreline feather of one to
    // three units -- was the one deciding opacity. That made every body fully opaque a metre or two
    // out, which is why refraction was invisible: there was no bed left to displace. It is also simply
    // wrong about water. You can see the bottom of a clear lake from a long way up.
    //
    // Exponential rather than linear, which is Beer-Lambert and is what absorption actually does: a
    // constant fraction removed per unit travelled. The practical difference from a clamped ramp is at
    // the two ends -- the shallows stay genuinely clear instead of starting to cloud immediately, and
    // deep water approaches opaque without ever snapping to it.
    let absorbed = 1.0 - exp(-depth / max(water.shallow.w, 0.001));

    // The transmitted term, tinted by how much water the view looks through.
    let tint = mix(water.shallow.rgb, water.deep.rgb, absorbed);
    let light = camera.lights[0];
    // Accumulated separately from `body` because the foam below is a white diffuse surface lit by the
    // same two terms, and lighting it a second way is how a foam band ends up brighter than the sunlit
    // water it sits on.
    var lit = light.ambient.rgb * mix(WATER_AMBIENT_FLOOR, 1.0, visibility);
    var glitter = vec3<f32>(0.0);
    let primary_length = length(light.source_direction.xyz);
    if (primary_length > 0.00001) {
        let light_direction = -light.source_direction.xyz / primary_length;
        // The same cloud term the ground gets, on the same direct share. Skipping it here would leave a
        // lake glittering under a deck that has visibly shaded every field around it.
        let shaded = mix(WATER_DIRECT_FLOOR, 1.0, visibility)
            * cloud_shadow(input.world_position.xy);
        lit += light.diffuse.rgb * max(dot(normal, light_direction), 0.0) * shaded;
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
    let body = tint * lit;

    // The reflected term, deliberately *not* shadow-attenuated. What a cast shadow occludes is the
    // sun, not the sky, so shadowed water keeps reflecting and still reads as water rather than as a
    // dark hole in the terrain.
    let incidence = clamp(dot(normal, view_direction), 0.0, 1.0);
    let f0 = mix(WATER_F0, WATER_F0_STYLISED, clamp(water.foam.z, 0.0, 1.0));
    let fresnel = f0 + (1.0 - f0) * pow(1.0 - incidence, 5.0);
    // How wide a cone this surface reflects into, which the sky then averages over. Two contributions,
    // and against the analytic sky neither mattered — a gradient in one variable averages to its own
    // centre — so both arrived with the captured one.
    //
    // The material's roughness is the smaller. It is deliberately low on water, which is why a lake
    // mirrors at all.
    //
    // The wave *slope* is the larger, and leaving it out is what renders a lake as coloured speckle:
    // neighbouring pixels then take single texels ten degrees apart, one off the horizon and the next
    // off the zenith. Every component is built at the same steepness, so each contributes the same
    // slope and the RMS of the sum is `WATER_SLOPE_RMS * height / wavelength`, which peaked crests
    // raise a little further; a mirror doubles any angle it reflects, hence the two. This is a
    // property of the *material* and not of the shading normal, which is why it is still right where
    // the normal has been damped flat: the slope has moved into the lobe rather than gone away.
    let peaking = clamp(water.train.w, 0.0, 1.0) * 0.25;
    let slope = WATER_SLOPE_RMS * water.waves.x / max(water.waves.y, 0.001)
        * sqrt(1.0 + 4.0 * peaking * peaking);
    let cone = max(clamp(water.waves.w, 0.0, 1.0) * WATER_ROUGHNESS_CONE, slope * 2.0);
    let reflected = reflection_colour(input.world_position, normal, view_direction, cone);

    // Shore foam. Keyed on depth so it bands the waterline the way surf does, and gated on the wave
    // train's own height so the band pulses along the shore instead of ringing it evenly -- which is
    // the same reason `depth` is measured from the displaced surface rather than from the mean.
    //
    // This is the largest single thing separating the three kinds at playing distance. An ocean
    // breaking on a beach is mostly read from its surf; a lake has a faint lap and a river has water
    // piling against its banks, and both want a fraction of the same term rather than a different one.
    let shore = 1.0 - smoothstep(0.0, max(water.current.z, 0.001), depth);
    let crest = clamp(waves.height / max(water.waves.x, 0.001) * 0.5 + 0.5, 0.0, 1.0);
    let shore_foam =
        water.current.w * shore * smoothstep(WATER_FOAM_CREST_START, WATER_FOAM_CREST_END, crest);

    // Whitecaps, which are the same froth in open water, on the crests that are both tall *and* steep.
    // The group envelope is what makes the first half read: crests without one all reach the same
    // height, so a threshold near the top either catches all of them or none. The second half is what
    // makes them the right size -- see the slope constants.
    //
    // The slope is measured against what this material's train would give on average, so the gate
    // means "steeper than this water usually is" rather than a figure in radians that would mean
    // something different for a pond and for a swell. The damped gradient is the honest one to use:
    // whitecaps a pixel cannot resolve should go the same way the normal did, or they come back as
    // exactly the sparkle this pass spent so much effort removing.
    let expected_slope = WATER_SLOPE_RMS * water.waves.x / max(water.waves.y, 0.001);
    let steepness = length(waves.gradient) / max(expected_slope, 0.0001);
    let whitecap = water.foam.x
        * smoothstep(WATER_WHITECAP_START, WATER_WHITECAP_END, crest)
        * smoothstep(WATER_WHITECAP_SLOPE_START, WATER_WHITECAP_SLOPE_END, steepness);
    let foam = clamp(max(shore_foam, whitecap), 0.0, 1.0);

    var surface = mix(body, reflected, fresnel) + glitter;
    surface = mix(surface, WATER_FOAM_COLOUR * lit, foam);
    // Fogged like any other surface, and before the bed is mixed in. Water that ignores fog while the
    // shore beside it fades out is the most conspicuous way to break a foggy scene -- and the bed is
    // deliberately not fogged again here, having been fogged by the pass that drew it.
    surface = apply_fog(surface, input.world_position);

    // Opacity is the greatest of what absorption, reflectance and foam imply. A shallow edge seen from
    // overhead is nearly clear, but the same edge seen at a grazing angle is a mirror, and a figure
    // taken from depth alone would fade out the far shore of every lake. Foam is froth rather than
    // water and hides the bed under it outright, so it forces its own share opaque.
    let opacity = max(max(absorbed, fresnel), foam);

    // Composited here rather than by the blender, and **that is what makes refraction possible at
    // all**. Fixed-function blending can only ever take the destination at *this* pixel, and a
    // displaced bed is by definition read from another one -- so the choice is between doing the
    // composite in the shader and not bending the transmitted view.
    //
    // With a zero offset this is exactly what `SrcAlpha, OneMinusSrcAlpha` computed, because
    // `scene_colour` is the same lit scene the destination held: the pass still blends, and the
    // arithmetic has moved rather than changed. The output alpha is one because the mix is already
    // done, and nothing downstream reads this target's alpha.
    let colour = mix(bed_colour, surface, opacity);
    return vec4<f32>(colour, 1.0);
}
