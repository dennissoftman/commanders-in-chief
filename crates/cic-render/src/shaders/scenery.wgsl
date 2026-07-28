// Wind-driven vertex displacement for scenery.
//
// A composition chunk. Declares only functions and constants, so it composes ahead of any pass that
// wants to sway geometry -- currently the model G-buffer, its four shadow cascades, and the motion
// vector all three of them write.
//
// Written from scratch; see `scenery.rs` for the derivation of every constant here and for why the
// predecessor's table was not consulted. The constants are duplicated across the language boundary
// because a shader needs them as literals, and a test in `scenery.rs` pins the pair rather than trusting
// this comment.
//
// # Why one function, called from four places
//
// The displacement has to be *identical* in the G-buffer pass, in each shadow cascade, and in the motion
// vector. A cascade that swayed differently would throw a shadow that detached from its caster, which is
// the single most visible way to get this wrong -- and it is a failure that no still capture of the lit
// frame would show, because the caster and the shadow are in different parts of the image. So the whole
// model is this one function and every entry point calls it.
//
// # Why the previous frame is a parameter rather than a stored buffer
//
// A temporal resolve needs to know where each vertex *was*. For sway that is free: the displacement is a
// pure function of scene time, so passing the previous frame's time returns the previous frame's
// position exactly. No per-vertex history, no second vertex buffer, and no drift between what the motion
// vector claims and what the geometry did.

// See `SWAY_REFERENCE_SPEED` in `scenery.rs`.
const SWAY_REFERENCE_SPEED: f32 = 8.0;
// See `SWAY_STEADY` and `SWAY_OSCILLATION`. They sum to one, so the crest reaches the saturated
// amplitude exactly and the trough stays positive -- a plant must not lean into the wind.
const SWAY_STEADY: f32 = 0.55;
const SWAY_OSCILLATION: f32 = 0.45;
// See `SWAY_SHAPE_EXPONENT`. The square is a cantilever's mode shape near its base, and it is what keeps
// the foot of a trunk still.
const SWAY_SHAPE_EXPONENT: f32 = 2.0;
// See `SWAY_GUST_WAVELENGTH`. World units between crests of the travelling gust front.
const SWAY_GUST_WAVELENGTH: f32 = 90.0;
// See `SWAY_FLUTTER_RATIO`. Deliberately not an integer, so the flutter and the sway never close.
const SWAY_FLUTTER_RATIO: f32 = 5.37;

const TAU: f32 = 6.2831853;

/// Displaces a vertex offset from its anchor, given the sway parameters its instance carries.
///
/// `offset` is the vertex's position relative to the instance's anchor, already in world space -- so the
/// amplitude comes out in world units and an instance's own scale carries through without this needing
/// to know about it. `weight` is the mesh's per-vertex share of the sway, `sway` is
/// `(tip_fraction, phase, frequency, flutter)` as `SwayProfile::packed` writes it, `wind` is the world
/// wind vector in units per second, and `time` is scene time.
///
/// Returns the *displaced* offset, not the delta, because the last step needs the whole vector.
fn sway_offset(
    offset: vec3<f32>,
    weight: f32,
    sway: vec4<f32>,
    anchor: vec3<f32>,
    wind: vec2<f32>,
    time: f32,
) -> vec3<f32> {
    let tip_fraction = sway.x;
    let frequency = sway.z;
    let speed = length(wind);
    // Three ways for this to be a no-op, and all three are ordinary rather than exceptional: rigid
    // scenery, still air, and the base of every plant that does move. Returning early keeps rigid
    // scenery costing nothing instead of costing a multiply by zero.
    if (tip_fraction <= 0.0 || frequency <= 0.0 || speed <= 0.0 || weight <= 0.0) {
        return offset;
    }
    let direction = wind / speed;
    // Bounded rather than proportional. Drag rises with the square of speed but a leaning element sheds
    // load, and more to the point nothing stops a scenario authoring an absurd wind -- see
    // SWAY_REFERENCE_SPEED.
    let pressure = speed / (speed + SWAY_REFERENCE_SPEED);
    let shape = pow(clamp(weight, 0.0, 1.0), SWAY_SHAPE_EXPONENT);

    // The gust front. Subtracting the along-wind distance means a plant downwind lags one upwind, so the
    // front travels *with* the wind rather than against it. Getting this sign wrong is invisible in a
    // still and unmistakable in motion.
    let travel = dot(anchor.xy, direction) / SWAY_GUST_WAVELENGTH * TAU;
    let phase = sway.y - travel;
    let slow = frequency * time * TAU + phase;
    // Never negative: see SWAY_STEADY.
    let along = pressure * (SWAY_STEADY + SWAY_OSCILLATION * sin(slow));
    // A quarter turn out of step with the bend, so the sideways motion peaks as the plant passes through
    // its neutral position -- which is when a real stem is moving fastest and least constrained.
    let fast = slow * SWAY_FLUTTER_RATIO + TAU * 0.25;
    let across = pressure * sway.w * sin(fast);
    let sideways = vec2<f32>(-direction.y, direction.x);
    let amplitude = tip_fraction * abs(offset.z) * shape;
    let displaced = offset + vec3<f32>(
        (direction * along + sideways * across) * amplitude,
        0.0,
    );

    // Bending a vertex sideways while leaving its height alone *lengthens* the branch it sits on, and
    // under a strong wind that makes a tree visibly taller. Re-projecting onto the sphere of its original
    // radius is one normalize and it turns the stretch into the shortening a real bend produces.
    let radius = length(offset);
    let bent = length(displaced);
    if (radius <= 0.0001 || bent <= 0.0001) {
        return displaced;
    }
    return displaced * (radius / bent);
}
