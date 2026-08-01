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
// Geometrically growing steps in world space, refined by bisection on the first step that ends up
// behind the depth buffer. The refinement is the standard fix for a coarse march: a step that lands
// past a surface reports a hit at the far end of it, which draws a reflection smeared along the ray by
// the step length.
//
// **The growth is not a refinement, it is the difference between working and not**, and the first
// version got it wrong. Fixed seven-unit steps over sixty units is a defensible march for a rough
// reflector filling the frame, and it is useless for water: a lake reflects its *far shore*, which at
// a grazing angle is hundreds of world units away, so every ray ran out of march long before reaching
// anything and the provider fell back to sky on every pixel. Measured against the sky provider it
// differed by 0.20 of 255 -- indistinguishable from doing nothing.
//
// Growing steps put fine samples where the ray leaves the surface, which is where a near hit needs
// precision, and coarse ones far away, where a whole ridge spans many steps anyway. Twelve steps at a
// ratio of 1.6 reach about nine hundred units from a two-unit first step.
//
// Marching in world space rather than screen space stays right, for the reason growth fixes: a
// screen-space DDA spends its samples evenly over pixels, and at a grazing view the pixels near the
// horizon each cover enormous distances, so the far end of the ray -- the part that matters here -- is
// sampled worst.
//
// This is the standard technique rather than anything novel; the usual references are McGuire and
// Mara's screen-space ray tracing note (2014) for the refinement, and Sousa's Crysis 3 presentation
// (2013) for the fade rules below.

// The first step in world units, how each one grows on the last, and how many there are.
//
// Two units, times 1.6, twelve times, reaches about nine hundred -- which on a map a couple of thousand
// units across is the far shore of anything a body is likely to be reflecting.
const REFLECTION_STEP_FIRST: f32 = 2.0;
const REFLECTION_STEP_GROWTH: f32 = 1.6;
const REFLECTION_MARCH_STEPS: i32 = 12;

// How many times the crossing step is halved. Four brings a seven-and-a-half-unit step down to under
// half a unit, which is finer than the depth reconstruction is trustworthy at these distances.
const REFLECTION_REFINE_STEPS: i32 = 4;

// How far behind the depth buffer a sample may be and still count as a hit, as a multiple of the step
// that got there.
//
// Without a thickness test the depth buffer reads as an infinitely deep occluder: every ray that ever
// passes behind anything registers a hit on it, and a reflection of the sky beyond a ridge comes back
// as a smear of the ridge. With one, a ray that passes *well* behind a surface is treated as having
// gone past it rather than into it.
//
// Relative to the step rather than absolute, because the steps grow: a fixed four units is generous
// against a two-unit step near the surface and impossibly tight against a hundred-unit one far away,
// so a fixed figure would make the march progressively blinder the further it went. What thickness a
// surface actually has is information a depth buffer does not carry, which is one of the reasons this
// technique is an approximation.
const REFLECTION_THICKNESS_STEPS: f32 = 1.5;

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

    var hit_uv = vec2<f32>(0.0);
    var found = false;
    var travelled = 0.0;
    var reach = 0.0;
    var step_length = REFLECTION_STEP_FIRST;
    var total = 0.0;
    for (var count = 0; count < REFLECTION_MARCH_STEPS; count += 1) {
        total += REFLECTION_STEP_FIRST * pow(REFLECTION_STEP_GROWTH, f32(count));
    }

    for (var step = 0; step < REFLECTION_MARCH_STEPS; step += 1) {
        let previous = reach;
        reach += step_length;
        let distance = reach;
        step_length *= REFLECTION_STEP_GROWTH;
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
        if (distance - length(surface - world_position)
            > (distance - previous) * REFLECTION_THICKNESS_STEPS) {
            break;
        }

        // Bisect the step that crossed. Halving from the near end each time, so `near` is always in
        // front of the surface and `far` always behind it.
        var near = previous;
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
    let distance_fade = 1.0 - smoothstep(0.7, 1.0, travelled / max(total, 0.001));
    return mix(sky, scene_colour_at(hit_uv), edge_fade * distance_fade);
}
