// What a surface sees looking out along its mirror direction: the sky, and only the sky.
//
// A composition chunk, and **one of several that answer the same question**. Exactly one
// `reflection_*` chunk is composed into a program, they all export `reflection_colour` with this
// signature, and which one a pipeline gets is chosen on the Rust side by `ReflectionProvider`. This is
// the cheapest of them and the one every program falls back to.
//
// Requires `sky.wgsl` for `sky_reflection`, and must follow it.
//
// # Why this is one function and not three lines at each site
//
// The reflected term was computed inline where it was needed, which was fine while there was exactly
// one answer to the question: the sky, evaluated analytically from a direction. Nothing in the scene
// reflects anything else -- no probe, no cubemap, no screen-space trace -- so `sky_colour(reflect(...))`
// *was* the whole implementation, and naming it would have been ceremony.
//
// It stops being ceremony as soon as there is a second possible answer. A reflection that traced the
// scene would need every caller to change rather than one, and the callers are in different files with
// different reasons for asking, so the drift would be silent -- water reflecting the traced scene while
// a metal hull still mirrored a flat sky, which reads as the hull being unpainted rather than as a bug.
// One function is the seam: a provider substitutes this chunk, keeps the signature, and both callers
// follow.
//
// **That seam has now been used twice, and it held both times.** A captured environment is exactly the second
// answer it was written for, and wiring it took editing this function and nothing above it — the water
// pass reflects an HDRI without knowing one exists. What the change did add is a `cone` parameter,
// because the analytic sky had no detail for a spread lobe to average and a captured one has nothing
// but.
//
// # The two call sites, one of which is not here yet
//
// `water.wgsl` calls this for its Fresnel-weighted reflected share, which is the one place a reflection
// is currently visible.
//
// The lighting pass is the other, and it does *not* call this yet, because it has nothing to call it
// with: a metal there takes `albedo * ambient`, and that expression is the flat-sky answer already --
// see the reflectance note in `lighting.wgsl`. Wiring it through this function would change the frame
// rather than move it, so it is left alone. A reflection provider replaces that ambient term for
// metals, and that is the second edit it makes.
fn reflection_colour(
    world_position: vec3<f32>,
    normal: vec3<f32>,
    view_direction: vec3<f32>,
    cone: f32
) -> vec3<f32> {
    // `world_position` is unused by either sky, both of which depend only on direction. It is in the
    // signature because every provider that is not the sky needs it -- a trace needs an origin, and a
    // probe needs to know which one it is nearest -- and adding a parameter later means editing the
    // callers this function exists to keep from being edited.
    _ = world_position;
    return sky_reflection(reflect(-view_direction, normal), cone);
}
