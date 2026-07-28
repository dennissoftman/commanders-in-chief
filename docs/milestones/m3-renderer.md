# M3: Renderer

Draw a map: terrain, water, models, lighting, and shadows, in a window and headlessly.

**Status:** Charter complete, plus the atmosphere and weather that were not in it. Every item below is landed and tested; the one qualification is that CI
has no GPU, so the regression harness runs on a developer machine and not yet on a runner. See
[Exit condition](#exit-condition).

## Charter

- Terrain rendering from a heightfield, with level of detail that holds up at both a tactical zoom and
  a strategic one.
- Static and instanced model rendering from imported geometry.
- Directional lighting with cascaded shadows, and ambient occlusion.
- Water surfaces.
- Windowed presentation driven by the reusable camera, plus a headless capture path.
- Visual regression tests: a capture compared against a committed reference, so a rendering change
  either is intended or fails CI. (Built and verified; "fails CI" awaits a runner with an adapter.)

## Landed

- **Headless device and capture.** A device with a software-adapter fallback, bounded capture sizing,
  padded readback, and PNG encoding. Headless before windowed, because a capture is the only rendering
  verification that runs in CI.
- **A forward terrain pass.** A `cic-assets` terrain renders with directional lighting and blended
  texture layers. Heights go into an `R16Uint` texture and layer weights into an `R8Unorm` array, both
  writable; the grid is procedural in the vertex shader and normals come from central differences on
  the height texture. Terrain deformation and route grading are therefore texture writes, not a remesh
  — verified by tests that draw, write a region, and draw again.
- **View and projection**, with a test that pins the near and far planes to depth 0 and 1. That
  assertion exists because an OpenGL-convention matrix here would clip half the scene and look like a
  culling bug.
- **The WGSL shader set**, validated at test time by the same front end the GPU backend uses.
- **Virtual-page residency bookkeeping**, decided without touching a GPU so it is testable in isolation.
- **Texture resources**, deduplicated by content hash under explicit byte budgets.
- **The camera**, as a standalone model with no window, input, or GPU dependency.

- **The deferred chain**, now in seven passes: four depth-only shadow cascades, a G-buffer, ground-truth
  ambient occlusion with a bilateral blur, deferred lighting that reconstructs world position from
  depth, a blended water pass, and a tone-mapping composite. The lighting and occlusion shaders were adapted rather than
  rewritten, keeping their technique and their reasoning; only their legacy camera layout and
  three-light model were replaced.
- **Cascade fitting**, as pure arithmetic with no GPU involvement: bounding-sphere fits for rotation
  invariance, texel-grid snapping against shimmer, and a light-axis reach sized from the scene's
  height and the sun's elevation.

- **Windowed presentation**: a surface-aware device, sRGB format selection, resize that rebuilds
  everything sized to the surface, and an input-to-intent mapping that is testable without a window.
  The `terrain_viewer` example ties it together.

- **Instanced models**, sharing the terrain G-buffer and every shadow cascade, with per-instance
  transform and colour tint. Material index is a per-vertex attribute rather than bound state, so a
  model's primitives concatenate and the whole model draws in one call.

- **Texturing**, for both surfaces, through one shared mechanism: a colour texture array per drawing
  unit, indexed by a slice number the material carries. Terrain layers get an albedo tiled in *world*
  space at a per-layer detail scale, plus a per-layer roughness blended by the same weights as the
  colour; model materials get a base colour from the images their glTF carried. Mip chains are generated
  on the CPU in linear light. See [ADR 0004](../adr/0004-texture-arrays-and-world-space-tiling.md).

- **Water surfaces**, written from scratch: a bounded plane with five summed directional waves, drawn
  between lighting and the composite and blended *inside* the HDR target so its glitter tone maps with
  the scene rather than clipping over it. The shoreline is not authored — the pass discards wherever the
  reconstructed bed rises through the displaced surface, so a rectangle plus a heightfield already give
  an irregular shore that moves with the swell. Depth drives a shallow-to-deep tint and the edge
  opacity, Schlick Fresnel mixes in a reflection of the same sky the lighting pass paints, and the
  primary shadow term applies with water's own shade floors. Every constant was authored here; the
  predecessor's water shader was removed rather than carried across and was not consulted. See
  [LICENSING.md](../../LICENSING.md).

- **Scene time as a frame parameter.** `DeferredFrame::time` drives every animated surface, and nothing
  in the renderer reads a clock. That is what makes a capture of moving water reproducible, so it is a
  precondition for the harness below rather than a convenience.

- **The visual regression harness.** A capture is compared against a committed reference and fails on a
  regression, writing the capture and an amplified difference image beside the other test output.
  References are committed per adapter, because two GPUs do not agree to the byte and a tolerance loose
  enough to span them is loose enough to accept a real fault. Nine references cover terrain layers,
  instanced models, the whole deferred chain, water, water under a glancing sun, cloud shadows, fog, wet
  ground, and snow. The comparison
  itself is a pure function over bytes with its own unit tests, so it is verified even on a machine with
  no GPU.

- **Composed shaders.** WGSL has no include mechanism, and that one gap had shaped the set badly: any pass
  needing `shadow_visibility`, `world_from_depth`, or the sky had to live in the *same file* as them, so
  deferred lighting, the composite, and the whole water surface had accumulated into one 620-line file with
  nothing else in common. Named chunks are now concatenated in Rust — no preprocessor, no dependency, no
  directives inside the WGSL — and every composed program is validated by `naga` at test time. Five of the
  sixteen shaders present before this were bound to a pipeline; six of the remainder were superseded dead
  code and were deleted, two of them carrying comments describing a uniform layout that no longer existed.
  The five still unwired are marked `staged`, and a test fails if any chunk is named by no program.

- **An atmosphere**, derived from an hour of the day and a weather state rather than configured field by
  field. Sun direction and colour, ambient, sky, fog, and cloud coverage all follow from those two, because
  they are not independent: an overcast sky is dimmer in the beam *and* brighter in the ambient *and* greyer
  *and* foggier, all being one cloud deck seen from different angles. Weather is blendable scalars rather
  than an enum, because weather transitions. See [ADR 0006](../adr/0006-atmosphere.md).

- **Cloud shadows**, as procedural gradient noise sampled in *world* space, domain-warped so its contours are
  wisps rather than blobs, attenuating the sun's direct term only — a cloud occludes the sun's disc, not the
  sky — with a depth that rises with cloud thickness rather than saturating into one uniform shade.

- **Height and distance fog**, marched along the view ray in six taps with an exponential height falloff per
  tap, so a valley pools while a ridge stands clear of it. Applied inside the lighting and water shaders
  rather than as a depth-based post pass, because water writes no depth and a post pass would fog it at the
  depth of the terrain behind it.

- **A weather surface response**: wetness darkening albedo and dropping roughness, and snow settling by
  *slope* so it lies on flats and slides off cliffs. Both act on the G-buffer in the lighting pass rather
  than in the shaders that wrote it — terrain and models arrive there as albedo, normal and roughness, which
  is exactly what the two modify, so one implementation covers both.

- **Time of day drives the light.** `DeferredFrame` derives its sun from its environment, so changing the
  hour moves the sun instead of leaving a light that silently disagrees with it. The derivation is calibrated
  against the hand-tuned preset it replaced, in direction as well as colour, and a test pins it there.

## Remaining

- Antialiasing. Terrain silhouettes stair-step, water glitter sparkles, and cloud-shadow edges alias, and
  [ADR 0005](../adr/0005-antialiasing-strategy.md) settles what to do about it: a resolution scale, FXAA,
  and TAA — and explicitly *not* MSAA.
- Normal, roughness, and metallic *maps*. Only base colour is textured; the other channels are still
  per-material or per-layer factors.
- Alpha-tested materials. Everything is opaque, so the shadow passes have no fragment stage — foliage
  will need one.
- Wiring the residency bookkeeping to a real virtual-texture cache, so terrain detail scales past what
  one texture can hold.
- **A GPU-capable CI runner, and a reference set generated on it.** This is the one part of the exit
  condition not met, and it is infrastructure rather than renderer work — see below.

## Exit condition

A map package loads and renders, in a window and headlessly, at a stable frame rate; the visual
regression harness runs in CI with committed references.

**Met, except in CI.** The renderer half is done and the harness works: it catches a change of four
8-bit steps in one channel — a difference invisible to the eye — across 90% of a frame, verified by
perturbing the exposure constant and watching all five references fail.

What is not met is "in CI". The runner is `ubuntu-latest` with no GPU and no software rasteriser, so
`GpuContext::new` finds no adapter and **every rendering test skips there** — which has been true since
the first render test landed and is not something water or the harness changed. Closing it needs two
steps, in order:

1. Install Mesa's `lavapipe` on the runner, so an adapter exists and the rendering tests execute.
2. Generate the reference set on that runner — `CIC_UPDATE_REFERENCES=1` — then review those images and
   commit them under their own adapter directory.

The second step has to happen on Linux, which is why it is not in the change that built the harness. Note
that the references cannot simply be copied from a developer machine: a software rasteriser and an NVIDIA
card differ far beyond the tolerance, which is the whole reason the sets are keyed by adapter.

## Design notes

**Heights and weights are textures, not a baked mesh.** This was decided against known future
requirements rather than discovered: a faction whose map presence is literally paved needs to grade
roads across terrain at runtime, and levelling a structure needs terrain to deform. Both are texture
writes in this design and a remesh-plus-upload in the obvious alternative. The cost of designing for it
now was nil; the cost of retrofitting it would not have been.

**The forward pass does not tone map.** Albedo and the light terms are both near unity, so a Reinhard
curve compressed contrast that was never out of range — flattening exactly the slope shading the pass
exists to show. Tone mapping belongs with the deferred path, where accumulated lights genuinely can
exceed one.

**Ambient is deliberately low.** A large constant ambient lifts shadowed slopes almost to lit ones,
which flattens a heightfield just as effectively as a vertical light does. Skylight belongs in an
ambient-occlusion pass, not a constant.

**Captures are asserted on luminance spread, not colour count.** A distinct-colour count mostly reports
how varied the fixture's palette was; luminance range and deviation report whether light actually
differentiates slopes from flats.

**A statistical assertion cannot replace a reference image, and the two fail differently.** Every
rendering fault in this project's history produced a frame with a perfectly healthy luminance spread —
that is *why* the spread assertions passed while the images were wrong. A committed reference catches the
class of fault the statistics structurally cannot. It also fails in the opposite way: a spread threshold
is too permissive, while a reference is so sensitive that a driver update will trip it, and the answer to
that is to review the difference images and regenerate deliberately, never to loosen the tolerance until
it stops complaining.

**A water fixture has to be shaped for water, exactly as a shadow fixture has to be shaped for its sun.**
The shadowing terrain was reused first and was useless for this: its spire puts the elevation range in the
hundreds, so any level high enough to fill its bowl also drowned the plain. The next attempt used a basin
too narrow, giving a lake covering 1.6% of the frame — which leaves no room between "drew nothing" and
"failed to clip" for a bound to sit in, and made the animation assertion vacuous rather than failing.

**The difference between a dry frame and a flooded one is a mask of the water.** That is what makes any
assertion about the *surface* possible: a measurement over the whole frame mostly reports the terrain's
range. Comparing two renders and measuring only where they disagree is how "the water is a flat sheet"
became a falsifiable claim.

**One GPU device per test binary, not per test.** Creating and destroying several devices concurrently
on one adapter crashed the driver outright — an access violation rather than a test failure.

**Shadow tests need a control that differs only in shadowing.** Comparing an oblique sun against an
overhead one is not one: moving the sun changes every surface's incidence, so the two frames differ even
with the shadow pass deleted. Collapsing `shadow_distance` instead holds the light, camera, geometry, and
occlusion identical and puts every receiver outside all four cascades.

**Shadow acne on a face nearly parallel to the light is an *ambient* artifact.** Front-face culling in
the shadow pass separates near from far depth along the light, but on a grazing face the far surface is
laterally rather than deeply offset, so the two land within a texel. The direct term hides it — incidence
is near zero there — while the ambient term is shadow-attenuated and does *not* depend on incidence, so
the flicker surfaces as diagonal striping. Fading the attenuation out as incidence approaches zero fixes
it at the cause and costs nothing visible, since that geometry receives no direct light anyway.

**A cascade's reach toward the light must be sized from the tallest *caster*, not the tallest terrain.**
A model standing on terrain reaches higher than the terrain does, so a cascade sized from the
heightfield alone fails to record it as an occluder at a low sun. That figure has to be *recomputed*
when instances change, not merged with the previous one: taking a maximum against the old value means
removing the tallest instance leaves every cascade reaching toward a caster that is no longer there.

**A terrain detail texture must be addressed in world units, not in the terrain's own `uv`.** Normalized
coordinates fit exactly one copy of an image across the whole map — about four metres per texel on a
two-kilometre map, which is uniform blur at every zoom a player uses. This is the single decision that
separates ground from a stretched photograph.

**Mip levels must be averaged in linear light.** The sRGB transfer curve is concave, so the mean of two
encoded values sits above the encoding of their mean. Averaging stored bytes makes a high-contrast
texture pale as it recedes — a gradient the eye reads as fog that nobody added.

**A texture sample cannot be branched around on a per-fragment condition.** `textureSample` takes its mip
level from screen-space derivatives, defined only in uniform control flow. Skipping the sample for
materials without a texture leaves the mip level undefined for the ones *with* one. Sample
unconditionally and discard with `select`; the shader validation test catches a regression, because
`naga` runs the same uniformity analysis the backend does.

**A wrong UV mapping is invisible until something is mapped through it.** Both box fixtures assigned
corner coordinates as `[corner & 1, corner >> 1]`, which walks the unit square in Z order while the
corners walk it in a ring — so the last two swapped and every face was sheared along a diagonal. It cost
nothing for as long as the fixtures were untextured, and showed up in the first capture that sampled
them.

**Surface capabilities must be queried through the adapter that owns the surface.** Reconstructing an
adapter from a second `wgpu::Instance` to ask about a surface belonging to the first is not a wrong
answer but a hard failure inside the graphics layer. `GpuContext` retains its adapter for this reason.
No headless test could have caught it, because none of them create a surface — it took running the app.

**A water surface cannot attach the depth buffer it needs to read.** The pass shares the lighting bind
group — deliberately, so it reuses the cascade selection rather than keeping a second copy of it — and
that group already binds the scene depth for sampling. One pass cannot both attach and sample the same
texture, so `water_fragment` performs the depth comparison itself against the value it loads. That is the
same `Less` test the rasterizer would have run, on a value the shader was reading anyway.

**Summed sine waves read as a tiled texture unless two things are deliberately irrational.** The first
attempt used wavelength ratios of 1, ½, ⅓, ⅙ and four directions a little over 90° apart, and produced an
unmistakable diamond lattice. Near-harmonic wavelengths reinforce at regular intervals, and any rational
fraction of a turn eventually puts two waves on a shared axis — a shared axis being what a lattice is
made of. Steps of about 0.61 in wavelength and the golden angle in direction both fixed it, and neither
was predicted; the grid was visible in the first capture and in none of the assertions.

**A specular term can be present in the shader and absent from the frame.** Water is smooth, so the
obvious gloss exponent is mirror-like — and at two thousand the lobe is under two degrees wide, against
wave normals that stray about fifteen. The highlight then lands on almost no pixel at all and the surface
renders matte. Lowering the exponent to 720 was not a tuning preference but the difference between
glitter and dead code, and there is now a test that puts the sun in the mirror direction and asserts the
peak brightens, because nothing else would have noticed.

**Water clips to the terrain's footprint, not to its own rectangle.** An earlier version treated "no bed
behind this fragment" as infinitely deep water, which sounds right and is wrong: coverage is zero only
*outside* the heightfield, so the rule never fired inside a map and did nothing but paint a slab hanging
past the map edge — clearly visible under the terrain boundary, because terrain is an open sheet rather
than a solid. It took 10% of the frame and was found by opening the capture for a test asserting that
water below all terrain draws nothing.

**A composition step was the prerequisite for the atmosphere, not a tidying exercise.** Cloud shadows, fog
and the weather response all need `shadow_visibility` or `world_from_depth`, and all three would have landed
in the same file as them — pushing one shader past 900 lines — for want of an `#include`. Concatenating named
chunks in Rust costs about eighty lines and removes the constraint entirely.

**A refactor is provable when captures are committed.** Splitting the deferred shader into six chunks, moving
fifteen files, deleting six, and rewiring three pipelines produced *byte-identical* output on every reference.
That is the difference between a restructure and a change that happens to still pass its assertions.

**A noise lattice is usually the hash, not the interpolation.** `fract(sin(dot(cell, k)) * c)` is the form
every snippet reaches for, and it is a function of a *linear combination* of the coordinates — so every cell
on a line perpendicular to `k` receives a correlated value, and the field is streaked before any interpolation
happens. Two fixes were shipped against the symptom first: rotating the octaves removed axis-aligned steps but
left angular facets, and moving from value noise to gradient noise softened those without removing them.
Integer bit mixing has no preferred direction and removed the lattice outright.

**Coverage must move a threshold, not scale a density.** Scaling darkens every pixel by the same factor, which
is a brightness slider wearing a cloud's name — and it satisfies any assertion that merely asks whether the
frame got darker. The test therefore measures the *variance* of the per-pixel drop, since dappling and dimming
differ in exactly that.

**A shadow term that saturates gives every patch one depth.** A `smoothstep` up to the coverage onset reads as
a stencil laid over the ground. Real cloud shade varies with how much cloud is overhead, so the depth has to
keep rising with density past the onset.

**Fog is an integral, which makes it structurally harder to vary than a shadow.** A cloud shadow is evaluated
at the surface, so spatial variation in the noise is spatial variation on screen. Fog accumulates along the
ray, and an integral smooths its own input: patchiness needs the density to differ *along* the ray, which one
sample at the midpoint cannot express however it is tuned. It also inverts the scale intuition — a patch scale
far smaller than the ray length averages several banks per ray and returns the uniform wash it was meant to
remove.

**A fog layer is thick or thin relative to the *camera*, not to the terrain.** With the camera 614 units up
and a 52-unit layer, `exp(-(614 - 30) / 52)` is about 1e-5, so the rays passed through effectively no fog at
all and the frame was unchanged. The figure that looked reasonable against the terrain's height was five
orders of magnitude off against the camera's.

**Calibrate a derived value against the tuned one it replaces.** The environment's sun was first derived from
physical reasoning alone and produced an ambient about three times darker than the preset it was to replace —
which would have dimmed every scene. Worse, a second version matched diffuse and ambient *exactly* while
sitting 27 degrees away in azimuth: that rotated every shadow and flattened a ridge fixture shaped to run
across the old light. Colour agreement is not direction agreement, and only the capture said so.

**Presentation is the same chain pointed at a swapchain.** The only differences are the output format,
which a surface commonly reports as BGRA rather than RGBA, and that a resize reallocates every
intermediate target — which invalidates every bind group holding a view of one, so the chain is rebuilt
rather than just the surface reconfigured.

**A shadow fixture has to be shaped for the sun it is lit by.** Two separate versions of the test
measured nothing: one placed the sun so every shadow fell behind its own caster and out of frame; the
other used a ridge wider than its own shadow was long, so the shadow landed entirely on the ridge's
unlit back slope. Neither was a renderer fault, and neither was visible from the assertions.

## Explicitly not done

- No post-processing chain beyond what the shader set already covers.
- No particle system; it belongs with the gameplay that spawns effects.
- No level-of-detail generation for models. Terrain has it because a heightfield's regularity makes it
  cheap; model LOD wants measurement first.
- **MSAA is declined outright**, rather than pending. Multisampling a deferred G-buffer means four times
  the memory on every target *and* per-sample lighting behind a stencil pass, because averaging normals
  or depths across a silhouette yields values describing no surface that exists — and having paid for all
  that it still fixes only geometric edges, not the texture and specular aliasing that are now the more
  visible cases. This entry previously appeared under both "remaining" and "not done", which read as an
  omission nobody had decided about. [ADR 0005](../adr/0005-antialiasing-strategy.md) decides it.
- No reflections of scene geometry in water. The surface reflects the sky gradient and nothing else, so a
  cliff at the water's edge does not appear in it. A planar reflection pass means drawing the scene twice
  and a screen-space one means a ray march over the depth buffer; both are worth more than they cost only
  once there is something on the shore worth seeing reflected.
- No refraction offset. What is beneath the surface is read at the fragment's own pixel rather than along
  a bent ray, so a submerged object does not displace as the waves pass over it. Doing it properly needs
  the lit scene as a *texture* the water pass can sample at an offset, which is a fourth read of the
  colour target for an effect visible only in the shallows.
- No flow, foam, or shoreline wave breaking. The shore is a depth-driven fade, not a simulation.
- No anisotropic filtering. It is an optional device capability, and a sampler that fails to create on a
  software adapter would take the headless suite with it. Trilinear until there is a measured reason and
  a capability check.
- No texture compression. Colour arrays upload as `Rgba8UnormSrgb`; the block-compressed formats differ
  per backend and want an offline pipeline, which belongs with M8's tooling.
