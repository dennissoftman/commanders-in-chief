// What a surface sees looking out along its mirror direction: the scene itself, traced in screen space,
// falling back to the sky.
//
// A composition chunk, and the second answer to the question `reflection_sky.wgsl` asks. It exports
// the same `reflection_colour` and is substituted for that chunk rather than added beside it. Requires
// `scene.wgsl` for the depth buffer and the camera, `scene_colour.wgsl` for the lit scene and the
// projection helper, and `sky.wgsl` for the fallback -- and must follow all three.
//
// # What this buys and what it cannot
//
// The sky provider renders a lake beside a hill as *sky where the hill should be*, and trees standing
// in water reflecting nothing. That is the single largest reason water reads as flat, and it is what
// this fixes: the reflection is the scene.
//
// What screen space cannot do is reflect what is not on screen. Three cases, and all three are
// visible if you look for them:
//
// - **Off-screen geometry.** A hill just outside the frame does not reflect. The march reports a miss
//   and the sky answers, so the failure is a reflection that quietly reverts to sky rather than a hole.
// - **Behind other geometry.** The depth buffer holds one surface per pixel, so a ray passing behind
//   anything is lost. Ray tracing is the answer to this one, which is why the provider is swappable.
// - **Behind the camera.** `project_to_screen` reports these rather than dividing through, because a
//   position behind the viewer otherwise projects to a plausible point in front of it.
//
// The fallback is what makes all three acceptable: a miss is never a wrong colour, only a less
// informed one. That is also why the sky chunk stays composed in under this provider.
//
// # The march
//
// Fixed-step in world space, refined by bisection on the first step that ends up behind the depth
// buffer. Fixed steps are the cheap half and are wrong on their own -- a step that lands past the
// surface reports a hit at the far end of it, which draws a reflection smeared along the ray by the
// step length. The refinement costs a few more samples and puts the hit where the crossing is.
//
// Marching in *world* space rather than in screen space is the simpler of the two standard forms and
// the right one here. A screen-space DDA distributes its samples evenly over pixels, which is what you
// want when the reflector fills the frame; water is a roughly horizontal plane seen from above, so its
// reflected rays travel mostly *into* the screen, and even pixel steps put almost all the samples in
// the first few world units. Even world steps put them where the ray actually goes.
//
// This is the standard technique rather than anything novel here; the usual references are McGuire and
// Mara's screen-space ray tracing note (2014) for the refinement, and Sousa's Crysis 3 presentation
// (2013) for the fade rules below.

// How far a reflected ray is followed, in world units, and in how many steps.
//
// Sixty units at eight steps is seven and a half units a step, which for water on a map this scale
// covers the shore a body is likely to be reflecting and stops well short of the horizon. The bound is
// as much about what is *useful* as about cost: past a few tens of units the reflected image of a
// grazing ray is compressed into so few pixels that the sky fallback is indistinguishable from a hit.
const REFLECTION_MARCH_DISTANCE: f32 = 60.0;
const REFLECTION_MARCH_STEPS: i32 = 8;

// How many times the crossing step is halved. Four brings a seven-and-a-half-unit step down to under
// half a unit, which is finer than the depth reconstruction is trustworthy at these distances.
const REFLECTION_REFINE_STEPS: i32 = 4;

// How far behind the depth buffer a sample may be and still count as a hit, in world units.
//
// Without a thickness test the depth buffer reads as an infinitely deep occluder: every ray that ever
// passes behind anything registers a hit on it, and a reflection of the sky beyond a ridge comes back
// as a smear of the ridge. With one, a ray that passes *well* behind a surface is treated as having
// gone past it rather than into it. The figure is a guess about the scene rather than a measurement --
// which is exactly the information a depth buffer does not carry, and one of the reasons this
// technique is an approximation.
const REFLECTION_THICKNESS: f32 = 4.0;

// Where the reflection fades out, as a fraction of the way to the screen edge.
//
// A ray that leaves the frame stops reflecting *abruptly* without this, and the resulting hard line
// across a lake is far more noticeable than the missing reflection it replaces. Fading over the outer
// tenth trades a little of the effect for the absence of an edge.
const REFLECTION_EDGE_FADE: f32 = 0.10;

fn reflection_colour(
    world_position: vec3<f32>,
    normal: vec3<f32>,
    view_direction: vec3<f32>,
    cone: f32
) -> vec3<f32> {
    let direction = reflect(-view_direction, normal);
    let sky = sky_reflection(direction, cone);

    // A ray heading into the surface has nothing on screen to find. This is not merely an
    // optimisation: marching it walks backwards through the depth buffer and reports hits.
    if (direction.z < 0.0) {
        return sky;
    }

    let step_length = REFLECTION_MARCH_DISTANCE / f32(REFLECTION_MARCH_STEPS);
    var hit_uv = vec2<f32>(0.0);
    var found = false;
    var travelled = 0.0;

    for (var step = 1; step <= REFLECTION_MARCH_STEPS; step += 1) {
        let distance = f32(step) * step_length;
        let sample_position = world_position + direction * distance;
        let point = project_to_screen(sample_position);
        if (!point.on_screen) {
            break;
        }
        let pixel = vec2<i32>(point.uv * camera.viewport.xy);
        let scene_depth_value = textureLoad(scene_depth, pixel, 0);
        // Reversed sense against the water pass's own depth test, and deliberately so: there the
        // question is whether the surface is in front of the scene, here it is whether the ray has
        // gone behind it.
        if (point.depth < scene_depth_value) {
            continue;
        }
        // Behind something. How far behind decides whether the ray hit it or passed it.
        let surface = world_from_depth(pixel, scene_depth_value);
        if (distance - length(surface - world_position) > REFLECTION_THICKNESS) {
            break;
        }

        // Bisect the step that crossed. Halving from the near end each time, so `near` is always in
        // front of the surface and `far` always behind it.
        var near = distance - step_length;
        var far = distance;
        for (var refine = 0; refine < REFLECTION_REFINE_STEPS; refine += 1) {
            let middle = (near + far) * 0.5;
            let probe = project_to_screen(world_position + direction * middle);
            if (!probe.on_screen) {
                break;
            }
            let probe_pixel = vec2<i32>(probe.uv * camera.viewport.xy);
            if (probe.depth < textureLoad(scene_depth, probe_pixel, 0)) {
                near = middle;
            } else {
                far = middle;
            }
        }
        let settled = project_to_screen(world_position + direction * far);
        if (settled.on_screen) {
            hit_uv = settled.uv;
            travelled = far;
            found = true;
        }
        break;
    }

    if (!found) {
        return sky;
    }

    // Two fades, and both are about hiding the technique's own boundaries rather than about physics.
    //
    // The first is the screen edge, without which a lake carries a hard line where its reflected rays
    // start leaving the frame. The second is the march's own end: a ray that ran nearly the whole
    // distance is one whose hit is least trustworthy, and easing those back into the sky is cheaper
    // than making them right.
    let edge = min(min(hit_uv.x, 1.0 - hit_uv.x), min(hit_uv.y, 1.0 - hit_uv.y));
    let edge_fade = smoothstep(0.0, REFLECTION_EDGE_FADE, edge);
    let distance_fade = 1.0 - smoothstep(0.7, 1.0, travelled / REFLECTION_MARCH_DISTANCE);
    return mix(sky, scene_colour_at(hit_uv), edge_fade * distance_fade);
}
