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

// A hash in `0..1` from a lattice cell, by integer bit mixing.
//
// **Not** `fract(sin(dot(cell, k)) * c)`, which is the form every noise snippet reaches for and which was
// the actual cause of the lattice this pattern showed for three separate attempts to fix it. That hash is a
// function of a *linear combination* of the coordinates, so every cell on a line perpendicular to `k`
// receives a correlated value — the pattern is streaked along that direction before any interpolation
// happens. Rotating the octaves and moving from value noise to gradient noise both attacked the
// interpolation and so could only ever soften the symptom.
//
// Bit mixing has no preferred direction: each coordinate is multiplied by its own large odd constant and
// the result is avalanched, so adjacent cells and distant ones are equally uncorrelated.
fn cloud_hash(cell: vec2<f32>) -> f32 {
    // `bitcast` rather than a conversion, so negative cell coordinates wrap into the integer range
    // predictably instead of relying on the conversion's behaviour at the boundary.
    var mixed = bitcast<u32>(i32(floor(cell.x))) * 0x27d4eb2du
        ^ bitcast<u32>(i32(floor(cell.y))) * 0x9e3779b9u;
    mixed ^= mixed >> 15u;
    mixed *= 0x85ebca6bu;
    mixed ^= mixed >> 13u;
    mixed *= 0xc2b2ae35u;
    mixed ^= mixed >> 16u;
    return f32(mixed) / 4294967295.0;
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

// Fractal cloud density in `0..1`, warped so its contours are wisps rather than round blobs.
//
// Two things make this read as smoke instead of as spots.
//
// **Octaves put roughness on the edges.** A two-octave field is smooth at every scale, so however soft its
// boundary it still reads as a shape with an outline. Five give the boundary detail of its own.
//
// **Domain warping tears the contours.** Summing octaves alone keeps every contour locally round, because
// each octave is isotropic and adding isotropic fields gives an isotropic field. Displacing the *sample
// position* by a lower-frequency copy of the noise stretches and hooks the contours instead, which is what
// distinguishes something moving in a fluid from a field of blobs.
fn cloud_density(sample_position: vec2<f32>) -> f32 {
    let turn = mat2x2<f32>(0.8, -0.6, 0.6, 0.8);

    // The warp is sampled well below the pattern scale, so it displaces whole regions rather than
    // roughening them. Roughness is what the octaves are for; doing both jobs with one frequency gives
    // neither.
    let warp = vec2<f32>(
        cloud_noise(sample_position * 0.5 + vec2<f32>(11.7, 3.1)),
        cloud_noise(sample_position * 0.5 + vec2<f32>(5.3, 19.4))
    ) - vec2<f32>(0.5);
    var position = sample_position + warp * 1.6;

    // Rotated as well as scaled between octaves, and a lacunarity of 2.13 rather than 2: doubling would
    // keep every octave's lattice aligned with the first one's.
    var amplitude = 1.0;
    var total = 0.0;
    var density = 0.0;
    for (var octave = 0; octave < 5; octave += 1) {
        density += cloud_noise(position) * amplitude;
        total += amplitude;
        position = turn * position * 2.13;
        amplitude *= 0.55;
    }
    density /= total;

    // Summing octaves pulls the distribution toward its mean, and a narrow distribution is fatal here
    // because coverage is a comparison against it: everything lands half-shaded and the deck reads as a
    // uniform haze. Stretching the contrast back out costs one multiply, where using fewer octaves would
    // cost exactly the edge detail they were added for.
    return clamp((density - 0.5) * 2.1 + 0.5, 0.0, 1.0);
}

// How much of the sun reaches a point on the ground, in `0..=1`.
//
// Sampled in *world* space, which is the lesson the terrain detail texture had to learn first: a screen- or
// uv-space pattern slides as the camera moves and stretches with the map, and either reads immediately as a
// texture stuck to the lens rather than as weather over the ground.
//
// Attenuates the sun's *direct* term only. A cloud occludes the sun's disc, not the sky, so the ambient
// share must survive it -- taking ambient down as well is what makes cloud shade read as a hole in the
// world instead of as an overcast patch.
fn cloud_shadow(world_xy: vec2<f32>) -> f32 {
    let coverage = camera.clouds.x;
    if (coverage <= 0.0) {
        return 1.0;
    }
    let scale = max(camera.clouds.y, 1.0);
    let density = cloud_density((world_xy + camera.cloud_drift.xy) / scale);

    // Coverage moves the *onset* rather than scaling the density, so raising it grows the shaded patches
    // outward from where they already were. Scaling instead dims the whole map uniformly, which is a
    // brightness change wearing a cloud's name.
    let onset = 1.0 - coverage;
    let softness = max(camera.clouds.w, 0.001) * 0.35;

    // Shade rises from the onset and *keeps rising* with density past it. A `smoothstep` that saturates at
    // the onset gives every shadowed patch one identical depth, which reads as a stencil laid over the
    // ground; real cloud shade varies with how much cloud is overhead, so a thick core has to shade harder
    // than a fringe. The eased term supplies the soft edge and the second factor the varying depth.
    let entered = clamp((density - onset + softness) / max(softness * 2.0, 0.02), 0.0, 1.0);
    let eased = entered * entered * (3.0 - 2.0 * entered);
    let depth = eased * (0.35 + 0.65 * entered);
    return 1.0 - depth * clamp(camera.clouds.z, 0.0, 1.0);
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
// Taps along the view ray. Six is coarse, and deliberately so -- see `fog_structure`.
const FOG_STEPS: i32 = 6;

// A cheap two-octave field for fog structure, rather than the five-octave warped one the clouds use.
//
// This is the trade that makes marching affordable. `cloud_density` costs seven noise evaluations, and
// walking the ray with it would cost forty-two per pixel. Fog is looked *through*, accumulating over a
// long path that blurs its own detail, so it does not need the edge roughness a shadow cast onto ground
// does -- where that detail is the whole point.
fn fog_structure(position: vec2<f32>) -> f32 {
    let turn = mat2x2<f32>(0.8, -0.6, 0.6, 0.8);
    let raw = cloud_noise(position) * 0.65 + cloud_noise(turn * position * 2.13) * 0.35;
    return clamp((raw - 0.5) * 1.9 + 0.5, 0.0, 1.0);
}

// Fog opacity between the camera and a world position, in `0..=1`.
//
// **Marched, not integrated in closed form**, and that is a correction to an earlier design rather than an
// escalation of it. The closed form is exact while the density varies only with *height*, and it was
// genuinely better than sampling at the fragment -- which would leave a valley floor as clear as the
// hilltop it is seen from, because nothing accounts for the dense air the ray crossed.
//
// But once the density also varies in xy there is no closed form, and the first attempt at patchiness --
// one noise tap at the ray's midpoint, scaling the analytic result -- could not work for a reason that had
// nothing to do with tuning: multiplying a smooth field by a mildly varying one leaves it smooth. Three
// rounds of raising patchiness, shrinking the scale, and lowering the density to escape the exponential's
// saturation all produced the same uniform wash. Fog stands in banks only if the density genuinely differs
// from one part of the ray to another, and that requires walking it.
//
// Height falloff is still exponential, evaluated per tap, so the valley-versus-hilltop behaviour the closed
// form bought is preserved rather than traded away.
fn fog_factor(world_position: vec3<f32>) -> f32 {
    let density = camera.fog.w;
    if (density <= 0.0) {
        return 0.0;
    }
    let falloff = max(camera.fog_params.x, 0.001);
    let base = camera.fog_params.y;
    let patchiness = clamp(camera.fog_params.z, 0.0, 1.0);
    let scale = max(camera.fog_params.w, 1.0);
    // A fog bank sits in its valley rather than racing the deck overhead, so it drifts at a fraction of
    // the cloud speed.
    let drift = camera.cloud_drift.xy * 0.35;

    let along = world_position - camera.camera_position.xyz;
    let step = length(along) / f32(FOG_STEPS);
    var optical = 0.0;
    for (var index = 0; index < FOG_STEPS; index += 1) {
        // The midpoint of each segment rather than an end, which is the difference between a Riemann sum
        // that converges at six taps and one that needs twenty.
        let fraction = (f32(index) + 0.5) / f32(FOG_STEPS);
        let point = camera.camera_position.xyz + along * fraction;
        var local = density * exp(-(point.z - base) / falloff);
        if (patchiness > 0.0) {
            let structure = fog_structure((point.xy + drift) / scale);
            local *= mix(1.0 - patchiness, 1.0 + patchiness, structure);
        }
        optical += local * step;
    }
    return 1.0 - exp(-max(optical, 0.0));
}


fn apply_fog(colour: vec3<f32>, world_position: vec3<f32>) -> vec3<f32> {
    return mix(colour, camera.fog.rgb, fog_factor(world_position));
}
