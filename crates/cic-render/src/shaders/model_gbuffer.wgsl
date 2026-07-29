// Instanced model G-buffer and shadow-depth passes.
//
// Requires `scenery.wgsl`, whose `sway_offset` every entry point here calls.
//
// Writes the same G-buffer targets as `terrain_gbuffer.wgsl` and shares its depth attachment, so
// models and terrain occlude each other correctly and the deferred lighting pass cannot tell them
// apart. That is the point of a G-buffer: one lighting pass, however many kinds of geometry wrote into
// it.
//
// The material index arrives per *vertex* rather than as bound state. Every vertex of a primitive
// carries the same index, so a model's primitives concatenate into one buffer pair and the whole model
// draws in a single instanced call. See `model.rs` for what that avoids.
//
// # Four entry points, one geometry path
//
// Opaque and masked geometry differ only in whether a fragment can discard, and the shadow passes differ
// from the G-buffer only in what they write. So all four share `place_vertex`, and in particular all four
// apply the sway identically — a cascade that swayed differently would throw a shadow detached from its
// caster, and no still capture of the lit frame would show it, because the caster and the shadow are in
// different parts of the image.

struct Uniforms {
    view_projection: mat4x4<f32>,
    camera_position: vec4<f32>,
    light_direction: vec4<f32>,
    light_ambient: vec4<f32>,
    light_diffuse: vec4<f32>,
    terrain: vec4<f32>,
    layers: vec4<u32>,
    palette: array<vec4<f32>, 8>,
    detail: array<vec4<f32>, 8>,
    // xy the world wind vector in units per second, z scene time, w the previous frame's scene time.
    animation: vec4<f32>,
    // xy this frame's sub-pixel jitter as a clip-space offset, zw unused.
    jitter: vec4<f32>,
    // The previous frame's unjittered view-projection, for the motion target.
    //
    // These three end the block, and they must stay at the end: `terrain_forward.wgsl` binds this same
    // buffer through a struct declaring only a prefix of it. See `UNIFORM_BYTES` in `terrain.rs`.
    previous_view_projection: mat4x4<f32>,
}

/// Base colour in `base_color`.
///
/// `factors` holds the metallic factor, the roughness factor, the base-colour array slice, and whether
/// that slice holds anything. `maps` holds the normal slice and its presence, then the
/// metallic-roughness slice and its presence. `surface` holds the normal-map scale, the alpha cutoff
/// (zero meaning never discard), the emissive strength, and one reserved slot.
struct Material {
    base_color: vec4<f32>,
    factors: vec4<f32>,
    maps: vec4<f32>,
    surface: vec4<f32>,
}

// Group 0 is the terrain group, bound already for its view-projection. Models read the transform and the
// wind from it, and sharing the group means one uniform buffer drives every pass in the frame -- so the
// camera cannot disagree between them, and neither can the wind.
@group(0) @binding(0) var<uniform> uniforms: Uniforms;

// Group 1 is the shadow cascade and group 2 the materials, kept apart because one module declares
// both and a group/binding pair must be unique within it. The G-buffer pipelines leave group 1 empty
// and the opaque shadow pipeline leaves group 2 empty -- which the graphics API allows explicitly, its
// pipeline layout taking an optional layout per slot for exactly this. The *masked* shadow pipeline
// binds both, because deciding whether to discard needs the material.
struct ShadowCascadeView {
    view_projection: mat4x4<f32>,
}

@group(1) @binding(0) var<uniform> cascade: ShadowCascadeView;

@group(2) @binding(0) var<storage, read> materials: array<Material>;
// One slice per image the model carried, in source order, bound once for the whole model -- which is
// what lets every material have its own picture without a bind group change between primitives.
//
// Three arrays rather than one because base colour is sRGB-encoded and the other two are linear
// measurements, and one array has one format. See `texture.rs`.
@group(2) @binding(1) var base_color_texture: texture_2d_array<f32>;
@group(2) @binding(2) var material_sampler: sampler;
@group(2) @binding(3) var normal_texture: texture_2d_array<f32>;
// glTF packs roughness in green and metallic in blue.
@group(2) @binding(4) var metallic_roughness_texture: texture_2d_array<f32>;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    // xyz tangent, w the bitangent's handedness.
    @location(2) tangent: vec4<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) material: u32,
    // This vertex's share of the sway, normalized over the model's own height.
    @location(5) sway_weight: f32,
    // A 4x4 instance transform, one column per attribute: there is no matrix vertex format.
    @location(6) transform_0: vec4<f32>,
    @location(7) transform_1: vec4<f32>,
    @location(8) transform_2: vec4<f32>,
    @location(9) transform_3: vec4<f32>,
    @location(10) tint: vec4<f32>,
    // Tip fraction, phase, frequency, flutter. See `SwayProfile::packed`.
    @location(11) sway: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) tint: vec4<f32>,
    // Flat, because a material index must not be interpolated between vertices — a fragment halfway
    // between materials 2 and 5 would read material 3.
    @location(2) @interpolate(flat) material: u32,
    @location(3) uv: vec2<f32>,
    @location(4) tangent: vec3<f32>,
    // The handedness, carried through interpolation rather than re-derived. It is constant across a
    // triangle in any sane mesh, and `sign` at the end recovers it exactly if a UV seam splits one.
    @location(5) handedness: f32,
    // The clip positions this frame and last, carried undivided. See `motion_vector`.
    @location(6) current_clip: vec4<f32>,
    @location(7) previous_clip: vec4<f32>,
}

struct GBufferOutput {
    @location(0) albedo: vec4<f32>,
    @location(1) normal_roughness: vec4<f32>,
    @location(2) coverage: f32,
    // Texture-coordinate motion since the previous frame.
    @location(3) motion: vec2<f32>,
}

fn instance_transform(input: VertexInput) -> mat4x4<f32> {
    return mat4x4<f32>(
        input.transform_0,
        input.transform_1,
        input.transform_2,
        input.transform_3,
    );
}

/// The transform's upper 3x3, which is the basis a direction takes.
///
/// Exact for rotation and uniform scale, which is all `ModelInstance` can express. A general transform
/// would need the inverse transpose; offering only the correct case is cheaper than computing an
/// inverse per vertex to support a case the API does not allow.
fn instance_basis(input: VertexInput) -> mat3x3<f32> {
    return mat3x3<f32>(
        input.transform_0.xyz,
        input.transform_1.xyz,
        input.transform_2.xyz,
    );
}

/// A vertex's world position at a given scene time, sway included.
///
/// Time is a parameter rather than read from the uniform so the motion vector can ask for the *previous*
/// frame's position from the same code. Sway is a pure function of time, so that returns exactly where
/// the vertex was — no per-vertex history buffer, and no way for the motion vector to disagree with what
/// the geometry did.
fn place_vertex(input: VertexInput, time: f32) -> vec3<f32> {
    let transform = instance_transform(input);
    let world = (transform * vec4<f32>(input.position, 1.0)).xyz;
    let anchor = input.transform_3.xyz;
    let swayed = sway_offset(
        world - anchor,
        input.sway_weight,
        input.sway,
        anchor,
        uniforms.animation.xy,
        time,
    );
    return anchor + swayed;
}

@vertex
fn gbuffer_vertex(input: VertexInput) -> VertexOutput {
    let basis = instance_basis(input);
    var output: VertexOutput;
    output.clip_position =
        uniforms.view_projection * vec4<f32>(place_vertex(input, uniforms.animation.z), 1.0);
    output.current_clip = output.clip_position;
    // Where this vertex was. Evaluated from the same function at the previous scene time, which for a
    // displacement that is a pure function of time returns the previous position *exactly* -- no per-vertex
    // history, and no way for the motion vector to disagree with what the geometry did.
    output.previous_clip = uniforms.previous_view_projection
        * vec4<f32>(place_vertex(input, uniforms.animation.w), 1.0);
    // The sway is not applied to the normal. It is a bend of a few degrees over a whole plant, so the
    // rotation of any one leaf is well under the variation the normal map already carries -- and
    // deriving the rotated normal correctly needs the displacement's gradient, which is several times
    // the cost of the displacement itself. Recorded here so the omission reads as a decision.
    output.normal = normalize(basis * input.normal);
    output.tangent = basis * input.tangent.xyz;
    output.handedness = input.tangent.w;
    output.tint = input.tint;
    output.material = input.material;
    output.uv = input.uv;
    return output;
}

/// The surface a fragment's material and maps describe, before the G-buffer packs it.
struct Surface {
    albedo: vec3<f32>,
    normal: vec3<f32>,
    roughness: f32,
    metallic: f32,
    alpha: f32,
    cutoff: f32,
    emissive: f32,
}

/// Reads every map and resolves the material into a surface.
///
/// Each map is sampled whether or not this material has one, then discarded by a `select` if it does
/// not. The branch that looks obviously cheaper is not available: `textureSample` derives its mip level
/// from screen-space derivatives, which exist only in uniform control flow, and the material index
/// varies per fragment. Guarding the calls would leave the mip level of every *textured* fragment
/// undefined -- the surfaces that matter, not the ones being skipped.
fn resolve_surface(input: VertexOutput) -> Surface {
    let material = materials[input.material];
    let base_slice = i32(material.factors.z);
    let sampled = textureSample(base_color_texture, material_sampler, input.uv, base_slice);
    let textured = material.factors.w > 0.5;
    let base = select(vec4<f32>(1.0), sampled, textured);

    let normal_sample = textureSample(
        normal_texture,
        material_sampler,
        input.uv,
        i32(material.maps.x),
    );
    let mr_sample = textureSample(
        metallic_roughness_texture,
        material_sampler,
        input.uv,
        i32(material.maps.z),
    );

    var output: Surface;
    output.albedo = material.base_color.rgb * base.rgb * input.tint.rgb;
    output.alpha = material.base_color.a * base.a * input.tint.a;
    output.cutoff = material.surface.y;
    output.emissive = material.surface.z;

    // Unlike terrain, a model's normal is not flipped toward the viewer: a model has a genuine inside
    // and outside, and forcing its normals to face the camera would light the interior of a hull as
    // though it were the exterior.
    let geometric = normalize(input.normal);
    output.normal = select(
        geometric,
        perturbed_normal(geometric, input.tangent, input.handedness, normal_sample.xy, material.surface.x),
        material.maps.y > 0.5,
    );

    // glTF multiplies the factor by the map's channel rather than replacing it, so a material with no
    // map reads the white fallback and comes out as its factor exactly.
    let mr = select(vec2<f32>(1.0), mr_sample.bg, material.maps.w > 0.5);
    output.metallic = clamp(material.factors.x * mr.x, 0.0, 1.0);
    output.roughness = clamp(material.factors.y * mr.y, 0.0, 1.0);
    return output;
}

/// Perturbs a geometric normal by a tangent-space normal map.
///
/// # Why `z` is reconstructed rather than read
///
/// The map stores all three components, and the third is redundant: a unit vector's `z` follows from its
/// `xy`. Reading it costs nothing, and rebuilding it is still the better answer, because averaging normal
/// maps down a mip chain does not preserve unit length -- so at any level but the first the *stored* `z`
/// describes a vector that is not normalized, and the further a surface recedes the more it lies.
/// Rebuilding from `xy` gives a unit normal at every level by construction.
///
/// # Why the basis is re-orthogonalized here
///
/// The tangent was orthogonalized against the *vertex* normal at import, and interpolation across a
/// curved triangle pulls the two apart again. A basis whose axes are not perpendicular skews the
/// perturbation, which tilts the flat regions of a normal map -- the regions that should be untouched.
fn perturbed_normal(
    normal: vec3<f32>,
    raw_tangent: vec3<f32>,
    handedness: f32,
    encoded: vec2<f32>,
    scale: f32,
) -> vec3<f32> {
    let along = dot(raw_tangent, normal);
    let orthogonal = raw_tangent - normal * along;
    let length_squared = dot(orthogonal, orthogonal);
    if (length_squared < 1.0e-12) {
        // A tangent parallel to the normal carries no direction on the surface. Better the unperturbed
        // normal than a basis built from a zero vector, which would produce a NaN and a black pixel.
        return normal;
    }
    let tangent = orthogonal * inverseSqrt(length_squared);
    // `sign` rather than the raw value: interpolation across a UV seam can land between +1 and -1, and a
    // bitangent scaled by 0.3 is not a basis vector.
    let bitangent = cross(normal, tangent) * select(-1.0, 1.0, handedness >= 0.0);
    // The scale acts on `xy` before `z` is rebuilt, so a scale of zero yields the geometric normal
    // *exactly* rather than a flattened approximation of it.
    let xy = (encoded * 2.0 - vec2<f32>(1.0)) * scale;
    let z = sqrt(max(1.0 - dot(xy, xy), 0.0));
    return normalize(tangent * xy.x + bitangent * xy.y + normal * z);
}

/// Packs a resolved surface into the G-buffer.
fn write_surface(surface: Surface, input: VertexOutput) -> GBufferOutput {
    var output: GBufferOutput;
    output.motion = motion_vector(
        input.current_clip,
        input.previous_clip,
        uniforms.jitter.xy,
    );
    // Alpha carries the metallic factor. The channel was writing a constant 1.0 and nothing read it, so
    // this is the one place a fourth material channel was available at no bandwidth cost at all -- and
    // eight bits is ample for a quantity that is 0 or 1 on almost every real material.
    output.albedo = vec4<f32>(surface.albedo, surface.metallic);
    output.normal_roughness = vec4<f32>(surface.normal, surface.roughness);
    // Coverage is 1.0 for opaque geometry and carries emissive strength above that. See `lighting.wgsl`.
    output.coverage = 1.0 + surface.emissive;
    return output;
}

@fragment
fn gbuffer_fragment(input: VertexOutput) -> GBufferOutput {
    return write_surface(resolve_surface(input), input);
}

/// The G-buffer stage for materials that cut their own silhouette.
///
/// Identical to `gbuffer_fragment` but for the discard, and deliberately a separate entry point rather
/// than one stage with the test inside it: a fragment shader that *can* discard forfeits early depth
/// rejection on most hardware, and opaque geometry is the overwhelming majority. Paying for the
/// possibility everywhere to serve the foliage would be the wrong trade.
@fragment
fn gbuffer_masked_fragment(input: VertexOutput) -> GBufferOutput {
    let surface = resolve_surface(input);
    if (surface.alpha < surface.cutoff) {
        discard;
    }
    return write_surface(surface, input);
}

/// Depth-only pass, rendered once per shadow cascade.
///
/// No fragment stage: opaque models write depth and nothing else, so the rasterizer's output is the
/// whole result.
@vertex
fn shadow_vertex(input: VertexInput) -> @builtin(position) vec4<f32> {
    return cascade.view_projection
        * vec4<f32>(place_vertex(input, uniforms.animation.z), 1.0);
}

/// The shadow vertex stage for masked geometry, which needs its UVs carried through.
///
/// A leaf card that cast a rectangular shadow would be worse than one that cast none: the eye reads a
/// hard quadrilateral on the ground as a solid object, so a canopy would darken the terrain in slabs.
/// This is why the alpha test has to exist in the cascades and not only in the lit frame.
@vertex
fn shadow_masked_vertex(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.clip_position = cascade.view_projection
        * vec4<f32>(place_vertex(input, uniforms.animation.z), 1.0);
    output.normal = vec3<f32>(0.0, 0.0, 1.0);
    output.tangent = vec3<f32>(1.0, 0.0, 0.0);
    output.handedness = 1.0;
    // A shadow pass writes no motion target. Filled with the clip position it does have, so the struct is
    // wholly initialized rather than carrying whatever the interpolator makes of an unwritten location.
    output.current_clip = output.clip_position;
    output.previous_clip = output.clip_position;
    output.tint = input.tint;
    output.material = input.material;
    output.uv = input.uv;
    return output;
}

/// The shadow fragment stage for masked geometry: the discard is its entire purpose.
///
/// It samples only the base colour, because only alpha is being tested. The other two maps are bound but
/// not read, which costs nothing — an unread binding is not a fetch.
@fragment
fn shadow_masked_fragment(input: VertexOutput) {
    let material = materials[input.material];
    let sampled = textureSample(
        base_color_texture,
        material_sampler,
        input.uv,
        i32(material.factors.z),
    );
    let base = select(vec4<f32>(1.0), sampled, material.factors.w > 0.5);
    let alpha = material.base_color.a * base.a * input.tint.a;
    if (alpha < material.surface.y) {
        discard;
    }
}
