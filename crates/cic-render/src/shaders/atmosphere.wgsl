// The sky, and later the fog and cloud terms that share its reasoning.
//
// A composition chunk. Requires nothing but is required by anything that shades an outdoor surface.

// The sky, as the two colours everything that needs one fades between.
//
// Named constants rather than literals at the point of use, because the water pass reflects this same
// sky. A reflection of a sky that is not the sky on screen is obvious in a screenshot and invisible
// from any assertion, so the two are made incapable of disagreeing rather than kept in step by hand.
const SKY_ZENITH: vec3<f32> = vec3<f32>(0.025, 0.04, 0.065);
const SKY_HORIZON: vec3<f32> = vec3<f32>(0.12, 0.20, 0.30);

// Declared here rather than in `water.wgsl`, which used to own it, because the cloud gradients need it
// too and every program that has one has the other. A shared constant living in the chunk both callers
// already depend on is the composition step paying for itself.
const TAU: f32 = 6.2831853;

// The sky along a world direction, for a reflection. The screen gradient below is the same two colours
// mixed by height in the frame instead, which is all a background needs.
fn sky_colour(direction: vec3<f32>) -> vec3<f32> {
    return mix(SKY_HORIZON, SKY_ZENITH, clamp(direction.z, 0.0, 1.0));
}

// -------------------------------------------------------------------------------------------------
// Cloud shadows
//
// Procedural rather than sampled from a coverage texture. A pattern this large needs no authored detail,
// costs no upload, and -- the reason that settled it -- carries no provenance question, which a borrowed
// cloud texture would. See `environment.rs` for what drives these figures.
// -------------------------------------------------------------------------------------------------

// A hash in `0..1` from a lattice cell. The `fract(sin(...))` form is the standard one; it is not
// distribution-quality, and does not need to be, because the eye is being shown soft blobs.
fn cloud_hash(cell: vec2<f32>) -> f32 {
    return fract(sin(dot(cell, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

// A pseudo-random unit vector for a lattice point.
fn cloud_gradient(cell: vec2<f32>) -> vec2<f32> {
    let angle = cloud_hash(cell) * TAU;
    return vec2<f32>(cos(angle), sin(angle));
}

// Gradient noise, quintic-interpolated, remapped to `0..1`.
//
// *Gradient* noise rather than value noise, and that choice was forced by looking at the capture. Value
// noise stores a scalar per lattice point and interpolates it, so inside each cell the field is a bilinear
// patch — and coverage works by thresholding the field, which turns those patches' contours into visibly
// straight edges meeting at cell corners. Two rounds of trying to smooth it away failed: rotating the
// octaves removed the axis-aligned steps but left angular facets, and a quintic interpolant fixed the
// creases between cells without touching the shape of the field inside them.
//
// Gradient noise is zero *at* every lattice point and carries a random direction there instead, so the
// lattice has no value to leak into the contours. Quintic interpolation still matters: `6t^5-15t^4+10t^3`
// is flat to second order at both ends, where the usual cubic smoothstep leaves a second-derivative
// discontinuity along every cell boundary.
fn cloud_noise(position: vec2<f32>) -> f32 {
    let cell = floor(position);
    let offset = position - cell;
    let weight = offset * offset * offset * (offset * (offset * 6.0 - 15.0) + 10.0);
    let corner_a = dot(cloud_gradient(cell), offset);
    let corner_b = dot(
        cloud_gradient(cell + vec2<f32>(1.0, 0.0)),
        offset - vec2<f32>(1.0, 0.0)
    );
    let corner_c = dot(
        cloud_gradient(cell + vec2<f32>(0.0, 1.0)),
        offset - vec2<f32>(0.0, 1.0)
    );
    let corner_d = dot(
        cloud_gradient(cell + vec2<f32>(1.0, 1.0)),
        offset - vec2<f32>(1.0, 1.0)
    );
    let blended = mix(
        mix(corner_a, corner_b, weight.x),
        mix(corner_c, corner_d, weight.x),
        weight.y
    );
    // Signed and roughly within -0.7..0.7, so this centres it on a half and fills the unit range.
    return clamp(blended * 0.7 + 0.5, 0.0, 1.0);
}

// How much of the sun reaches a point on the ground, in `0..=1`.
//
// Sampled in *world* space, which is the same lesson the terrain detail texture had to learn: a screen-
// or uv-space pattern slides as the camera moves and stretches with the map, and either one reads
// immediately as a texture stuck to the lens rather than as weather over the ground.
//
// This attenuates the sun's *direct* term only. A cloud occludes the sun's disc, not the sky, so the
// ambient share must survive it -- taking ambient down too is what makes cloud shade read as a hole in
// the world instead of as an overcast patch.
fn cloud_shadow(world_xy: vec2<f32>) -> f32 {
    let coverage = camera.clouds.x;
    if (coverage <= 0.0) {
        return 1.0;
    }
    let scale = max(camera.clouds.y, 1.0);

    // Three octaves, each *rotated* as well as scaled.
    //
    // The rotation is not decoration. Value noise is bilinear over an axis-aligned lattice, so its
    // contours follow that lattice — and coverage works by thresholding the density, which turns those
    // gentle diagonal gradients into hard contours. Stacking octaves that share the lattice's axes made
    // every shadow edge visibly rectangular, in straight horizontal and vertical steps. Rotating each
    // octave by an angle sharing no axis with the last removes it, which is the same fix and the same
    // reason as the golden-angle wave directions in `water.wgsl`.
    //
    // The lacunarity is 2.13 rather than 2 for the same reason: doubling keeps every octave's lattice
    // aligned with the first one's.
    let turn = mat2x2<f32>(0.8, -0.6, 0.6, 0.8);
    var position = (world_xy + camera.cloud_drift.xy) / scale;
    // Two octaves, not three. Each one added pulls the sum further toward its mean, and a narrow
    // distribution is worse here than a plain one: coverage works by thresholding the density, so once the
    // spread is narrower than the soft edge band every patch is *partially* shaded and the deck reads as
    // a uniform haze. Two keeps the range wide enough for distinct patches with distinct edges.
    let density = cloud_noise(position) * 0.65 + cloud_noise(turn * position * 2.13) * 0.35;
    // Coverage moves the *threshold* rather than scaling the density, so raising it grows the shaded
    // patches outward from where they already were. Scaling instead dims the whole map uniformly, which
    // is a brightness change wearing a cloud's name.
    // Scaled well down from the authored figure. `softness` reads as "0 is a hard edge and 1 is a very
    // soft one", but the band it controls is measured against the *noise* range, which spans well under
    // the full unit interval — so taking it at face value made the band wider than the whole distribution
    // and turned every setting into the same grey wash.
    let softness = max(camera.clouds.w, 0.001) * 0.18;
    let threshold = 1.0 - coverage;
    let shaded = smoothstep(threshold - softness, threshold + softness, density);
    return 1.0 - shaded * clamp(camera.clouds.z, 0.0, 1.0);
}

// -------------------------------------------------------------------------------------------------
// Fog
//
// Analytic height and distance fog. Not volumetric: a ray march through the shadow map buys light
// shafts and costs an order of magnitude more, while this buys what fog is actually for -- depth cues
// and weather -- for a handful of instructions.
// -------------------------------------------------------------------------------------------------

// Fog opacity between the camera and a world position, in `0..=1`.
//
// The density is *integrated along the view ray* rather than sampled at the fragment. Sampling at the
// surface gets the common case wrong in the most visible way: a valley floor seen from a hilltop would
// be as clear as the hilltop itself, because nothing would account for the dense air the ray crossed on
// the way down. The closed form below is the integral of `exp(-h)` over the segment, which is available
// precisely because the falloff is exponential.
fn fog_factor(world_position: vec3<f32>) -> f32 {
    let density = camera.fog.w;
    if (density <= 0.0) {
        return 0.0;
    }
    let distance = length(camera.camera_position.xyz - world_position);
    let falloff = max(camera.fog_params.x, 0.001);
    let base = camera.fog_params.y;
    let start = (camera.camera_position.z - base) / falloff;
    let end = (world_position.z - base) / falloff;
    let difference = end - start;
    // A level ray makes the integral degenerate, so it falls back to the plain exponential rather than
    // dividing by a difference of zero.
    var mean = exp(-start);
    if (abs(difference) > 0.0001) {
        mean = (exp(-start) - exp(-end)) / difference;
    }
    return 1.0 - exp(-max(density * distance * mean, 0.0));
}

fn apply_fog(colour: vec3<f32>, world_position: vec3<f32>) -> vec3<f32> {
    return mix(colour, camera.fog.rgb, fog_factor(world_position));
}
