// Ground-truth ambient occlusion over the resolved G-buffer, plus its bilateral cross blur.
//
// This is the cosine-weighted arc integral from Jimenez et al., "Practical Realtime Strategies for
// Accurate Indirect Occlusion" (2016), evaluated in world space because the G-buffer already stores
// a depth per pixel that unprojects to one. Occlusion drives the ambient share of
// deferred lighting, which is the dominant term in a daylit outdoor scene: every fill light
// contributes its own ambient, so an unoccluded sum leaves creases, hollows, and the ground under
// scenery visually ungrounded.
//
// Sampling is purely spatial. This renderer has no temporal antialiasing, so a temporally
// accumulated estimate would shimmer under free-flight camera motion; the noise is a per-pixel
// interleaved gradient offset resolved by the blur below instead.

// Must match `SceneCamera` in `terrain_deferred.wgsl` byte for byte: both passes bind the same
// uniform buffer.
struct DirectionalLight {
    ambient: vec4<f32>,
    diffuse: vec4<f32>,
    source_direction: vec4<f32>,
}

struct Camera {
    view_projection: mat4x4<f32>,
    // Inverse of `view_projection`, for reconstructing world position from scene depth.
    inverse_view_projection: mat4x4<f32>,
    camera_position: vec4<f32>,
    viewport: vec4<f32>,
    lights: array<DirectionalLight, 3>,
}

struct FullscreenOutput {
    @builtin(position) position: vec4<f32>,
}

@vertex
fn fullscreen_vertex(@builtin(vertex_index) vertex_index: u32) -> FullscreenOutput {
    let x = f32(i32(vertex_index) - 1) * 3.0;
    let y = f32(i32(vertex_index & 1u) * 2 - 1) * 3.0;
    var output: FullscreenOutput;
    output.position = vec4<f32>(x, y, 0.0, 1.0);
    return output;
}

@group(0) @binding(0) var g_normal: texture_2d<f32>;
// Geometry coverage in `r`; below 0.5 no geometry was drawn. See `g_coverage` in
// `terrain_deferred.wgsl`.
@group(0) @binding(1) var g_coverage: texture_2d<f32>;
@group(0) @binding(2) var<uniform> camera: Camera;
// Read directly as a depth texture; nothing here is multisampled, so there is no resolve step.
@group(0) @binding(3) var scene_depth: texture_depth_2d;

// World position of a G-buffer pixel, reconstructed from its depth.
//
// Occlusion and its bilateral blur both measure distances between neighbouring positions, so they
// inherited the old world target's whole-unit quantization directly: a tolerance of six world units
// cannot separate a crease from a flat surface when the positions themselves snap to two-unit steps.
// See `world_from_depth` in `terrain_deferred.wgsl` for why that target is gone.
fn world_from_depth(pixel: vec2<i32>, depth: f32) -> vec3<f32> {
    let uv = (vec2<f32>(pixel) + vec2<f32>(0.5)) * camera.viewport.zw;
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let homogeneous = camera.inverse_view_projection * vec4<f32>(ndc, depth, 1.0);
    return homogeneous.xyz / homogeneous.w;
}

// Position in `xyz` and geometry coverage in `w`, so a tap that landed on sky can be skipped.
fn load_geometry(pixel: vec2<i32>) -> vec4<f32> {
    let depth = textureLoad(scene_depth, pixel, 0);
    let coverage = textureLoad(g_coverage, pixel, 0).r;
    return vec4<f32>(world_from_depth(pixel, depth), coverage);
}

const PI: f32 = 3.14159265358979;
const HALF_PI: f32 = 1.57079632679490;

// Two slices at eight steps is the standard low GTAO preset: 32 position fetches per pixel once
// both sides of each slice are counted.
const AO_SLICES: i32 = 2;
const AO_STEPS: i32 = 8;
// World units. At the project's default 8-unit terrain spacing this reaches a few cells and the
// height of a typical structure -- far enough to ground scenery without darkening whole valleys.
const AO_RADIUS_WORLD: f32 = 28.0;
// Screen-space clamp, so a surface directly under the camera cannot turn the search into a
// full-screen gather. Kept low relative to the step count on purpose: the clamp is only reached
// when the camera is close enough that the world radius covers most of the screen, and stretching
// a fixed eight steps across a wide kernel spaces the taps far enough apart that individual
// occluder hits show up as diagonal banding. Trading reach for tap density at that range costs
// nothing visually, because occlusion barely reads when the ground fills the view.
const AO_MAX_SCREEN_RADIUS: f32 = 48.0;
// Above 1.0 deepens contact darkening without pushing open ground off white.
const AO_POWER: f32 = 1.6;

fn saturate_scalar(value: f32) -> f32 {
    return clamp(value, 0.0, 1.0);
}

fn project_to_pixel(world: vec3<f32>) -> vec2<f32> {
    let clip = camera.view_projection * vec4<f32>(world, 1.0);
    let ndc = clip.xy / max(clip.w, 0.0001);
    return (ndc * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5)) * camera.viewport.xy;
}

fn load_world(pixel: vec2<f32>) -> vec4<f32> {
    let limit = vec2<i32>(camera.viewport.xy) - vec2<i32>(1);
    let clamped = clamp(vec2<i32>(pixel), vec2<i32>(0), limit);
    return load_geometry(clamped);
}

// Interleaved gradient noise; stable per pixel and free of any frame-varying term. Drives the
// slice rotation, which it distributes well under a small blur.
fn spatial_noise(pixel: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(pixel, vec2<f32>(0.06711056, 0.00583715))));
}

// A second, decorrelated value for the step phase. Reusing the rotation noise here would make a
// pixel's slice orientation and its sample distances vary together, so neighbouring pixels sharing
// a rotation also sample identical distances and the tap pattern survives the blur as banding.
fn step_jitter(pixel: vec2<i32>) -> f32 {
    var hash = (u32(pixel.x) * 73856093u) ^ (u32(pixel.y) * 19349663u);
    hash ^= hash >> 13u;
    hash *= 1274126177u;
    return f32(hash & 0xFFFFu) / 65536.0;
}

// Largest horizon cosine along one side of a slice, with occluders faded out toward the search
// radius so the estimate stays local instead of banding at the radius edge.
fn horizon_cosine(
    center_pixel: vec2<f32>,
    position: vec3<f32>,
    view_direction: vec3<f32>,
    step_vector: vec2<f32>,
    jitter: f32,
) -> f32 {
    var best = -1.0;
    for (var step = 1; step <= AO_STEPS; step += 1) {
        let travel = (f32(step) - 0.5 + jitter) / f32(AO_STEPS);
        let sample = load_world(center_pixel + step_vector * travel);
        if sample.w < 0.5 {
            continue;
        }
        let delta = sample.xyz - position;
        let distance = length(delta);
        if distance < 0.0001 {
            continue;
        }
        let falloff = saturate_scalar(1.0 - distance / AO_RADIUS_WORLD);
        let cosine = dot(delta / distance, view_direction);
        best = max(best, mix(-1.0, cosine, falloff));
    }
    return best;
}

@fragment
fn ao_fragment(input: FullscreenOutput) -> @location(0) vec4<f32> {
    let center_pixel = floor(input.position.xy) + vec2<f32>(0.5);
    let world = load_geometry(vec2<i32>(input.position.xy));
    if world.w < 0.5 {
        return vec4<f32>(1.0);
    }
    let position = world.xyz;
    let normal = normalize(textureLoad(g_normal, vec2<i32>(input.position.xy), 0).xyz);
    let view_direction = normalize(camera.camera_position.xyz - position);

    // A camera-facing basis for the slice directions. The reference axis swaps when the view is
    // near-vertical, which a top-down RTS camera reaches routinely.
    var reference = vec3<f32>(0.0, 0.0, 1.0);
    if abs(view_direction.z) > 0.99 {
        reference = vec3<f32>(0.0, 1.0, 0.0);
    }
    let axis_right = normalize(cross(view_direction, reference));
    let axis_up = cross(axis_right, view_direction);

    let radius_edge = project_to_pixel(position + axis_right * AO_RADIUS_WORLD);
    let screen_radius = clamp(length(radius_edge - center_pixel), 2.0, AO_MAX_SCREEN_RADIUS);

    let noise = spatial_noise(center_pixel);
    let jitter = step_jitter(vec2<i32>(input.position.xy));
    var visibility = 0.0;
    for (var slice = 0; slice < AO_SLICES; slice += 1) {
        let angle = (f32(slice) + noise) * PI / f32(AO_SLICES);
        let screen_direction = vec2<f32>(cos(angle), sin(angle));
        let slice_direction =
            axis_right * screen_direction.x + axis_up * screen_direction.y;

        let plane_normal = normalize(cross(slice_direction, view_direction));
        let projected_normal = normal - plane_normal * dot(normal, plane_normal);
        let projected_length = length(projected_normal);
        if projected_length < 0.0001 {
            continue;
        }
        let slice_normal = projected_normal / projected_length;

        // Signed tilt of the surface normal away from the view vector, within this slice.
        let cos_gamma = clamp(dot(slice_normal, view_direction), -1.0, 1.0);
        let gamma_sign = select(-1.0, 1.0, dot(slice_normal, slice_direction) >= 0.0);
        let gamma = gamma_sign * acos(cos_gamma);
        let sin_gamma = sin(gamma);

        let step_vector = screen_direction * screen_radius;
        let forward = horizon_cosine(
            center_pixel,
            position,
            view_direction,
            step_vector,
            jitter
        );
        let backward = horizon_cosine(
            center_pixel,
            position,
            view_direction,
            -step_vector,
            jitter
        );

        // Horizons are signed angles from the view vector, then clamped into the hemisphere
        // around the surface normal so back-facing geometry cannot occlude.
        let raw_forward = acos(clamp(forward, -1.0, 1.0));
        let raw_backward = -acos(clamp(backward, -1.0, 1.0));
        let horizon_forward = gamma + min(raw_forward - gamma, HALF_PI);
        let horizon_backward = gamma + max(raw_backward - gamma, -HALF_PI);

        let arc_forward =
            -cos(2.0 * horizon_forward - gamma) + cos_gamma + 2.0 * horizon_forward * sin_gamma;
        let arc_backward =
            -cos(2.0 * horizon_backward - gamma) + cos_gamma + 2.0 * horizon_backward * sin_gamma;
        visibility += projected_length * 0.25 * (arc_forward + arc_backward);
    }

    let normalized = saturate_scalar(visibility / f32(AO_SLICES));
    return vec4<f32>(saturate_scalar(pow(normalized, AO_POWER)));
}

@group(1) @binding(0) var ao_source: texture_2d<f32>;

// Bilateral cross blur. World-position distance is the range weight, so occlusion never bleeds
// across a silhouette or over a terrain crease, and the per-pixel slice rotation above averages
// out instead of appearing as directional streaking.
const AO_BLUR_RADIUS: i32 = 2;
// Base tolerance in world units, plus a share of the view distance.
//
// A fixed tolerance cannot work: one pixel covers a couple of world units near the camera and tens of
// units at a grazing angle in the middle distance, so a constant that smooths correctly up close
// rejects almost every neighbouring tap further out -- leaving the raw per-pixel noise unblurred
// exactly where the sampling is sparsest. Scaling with distance keeps the range weight comparable to
// the actual spacing between neighbouring samples.
const AO_BLUR_WORLD_TOLERANCE: f32 = 6.0;
const AO_BLUR_DISTANCE_SHARE: f32 = 0.05;

@fragment
fn ao_blur_fragment(input: FullscreenOutput) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(input.position.xy);
    let center = load_geometry(pixel);
    if center.w < 0.5 {
        return vec4<f32>(1.0);
    }
    let limit = vec2<i32>(camera.viewport.xy) - vec2<i32>(1);
    let view_distance = length(center.xyz - camera.camera_position.xyz);
    let tolerance = AO_BLUR_WORLD_TOLERANCE + view_distance * AO_BLUR_DISTANCE_SHARE;
    var total = 0.0;
    var weight_sum = 0.0;
    for (var y = -AO_BLUR_RADIUS; y <= AO_BLUR_RADIUS; y += 1) {
        for (var x = -AO_BLUR_RADIUS; x <= AO_BLUR_RADIUS; x += 1) {
            let tap = clamp(pixel + vec2<i32>(x, y), vec2<i32>(0), limit);
            let neighbor = load_geometry(tap);
            if neighbor.w < 0.5 {
                continue;
            }
            let separation = length(neighbor.xyz - center.xyz);
            let weight = saturate_scalar(1.0 - separation / tolerance);
            if weight <= 0.0 {
                continue;
            }
            total += textureLoad(ao_source, tap, 0).r * weight;
            weight_sum += weight;
        }
    }
    if weight_sum <= 0.0 {
        return vec4<f32>(textureLoad(ao_source, pixel, 0).r);
    }
    return vec4<f32>(total / weight_sum);
}
