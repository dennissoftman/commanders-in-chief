# M3: Renderer

Draw a map: terrain, water, models, lighting, and shadows, in a window and headlessly.

**Status:** Charter complete **except for terrain level of detail**, plus the atmosphere and weather that
were not in it. One qualification remains, and it is that a visible terrain chunk still draws at full
density — though it is now only drawn when visible at all. That charter line used to read "with level of
detail", which was simply untrue, and the frustum culling that has since landed is the first half of
closing it.

The other qualification is closed. CI had no GPU, so the regression harness ran on a developer machine
and not on a runner; it now runs on Mesa's lavapipe with its own committed reference set, and a rendering
regression fails the build. See [Exit condition](#exit-condition) and the remaining item below.

## Charter

- Terrain rendering from a heightfield, with level of detail that holds up at both a tactical zoom and
  a strategic one. **The heightfield renders and is frustum-culled; the level of detail does not exist
  yet** — see [Remaining](#remaining).
- Static and instanced model rendering from imported geometry.
- Directional lighting with cascaded shadows, and ambient occlusion.
- Water surfaces.
- Windowed presentation driven by the reusable camera, plus a headless capture path.
- Visual regression tests: a capture compared against a committed reference, so a rendering change
  either is intended or fails CI. **Done**, including "fails CI" — see
  [Exit condition](#exit-condition).

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

- **The deferred chain**, in seven passes, plus an eighth when antialiasing is on: four depth-only shadow cascades, a G-buffer, ground-truth
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

- **Antialiasing, in the two cheaper tiers [ADR 0005](../adr/0005-antialiasing-strategy.md) settles on** —
  and with MSAA declined outright rather than left open. Both halves are one value,
  `DisplaySettings`, because a settings screen will present them as one choice.
  - **A resolution scale**, from a half to two, as a multiplier on the render resolution. Every pass from
    the G-buffer to the HDR accumulation is allocated at that size and the composite's filtered read of the
    HDR target *is* the downsample, so this costs a size multiplier at allocation and no extra pass. It is
    the only control here that addresses every class of aliasing at once, because it is the only one that
    raises the actual sampling rate.
  - **A post pass**, written from scratch: a luma gate with an absolute and a relative term, a Sobel pair
    for edge orientation, and a blend weight built from the second difference across the edge — so it
    leaves smooth ramps alone, halves a hard step, and hits an isolated sub-pixel highlight hardest. FXAA
    as a category; Lottes' implementation was not consulted. See [LICENSING.md](../../LICENSING.md).
  - **The composite's sharpen now switches off above a scale of one**, which was a fault rather than a
    preference: it amplified soft edges hardest, and a supersampled silhouette is soft by construction. The
    numbers are in the ADR.
  - Both are togglable in the viewer, on `T` and the bracket keys, because what an edge does *as the camera
    moves* is the whole subject and no still capture reports it.

- **Per-pass GPU timing** ([`timing`](../../crates/cic-render/src/timing.rs)). Each pass owns a fixed pair
  of timestamp queries; the resolve buffer is cleared before only the passes that ran are resolved, so a
  skipped pass reads back *absent* rather than as a duration of zero. The tick arithmetic is a pure
  function with its own tests, and `TIMESTAMP_QUERY` is requested only where the adapter offers it, so a
  device without it renders identically and reports nothing. `P` in the viewer prints a breakdown once a
  second — reading blocks on the GPU, which is why it is not per frame.

- **Terrain frustum culling**, over a chunk decomposition ([`culling`](../../crates/cic-render/src/culling.rs)).
  The terrain divides into 32-cell chunks, each with a world-space box; the camera and each *fitted cascade*
  cull against their own frustum, and the surviving chunks draw as instanced runs. The instance index **is**
  the chunk index, which is what keeps this free of any new binding, buffer or upload: the vertex shader
  turns it into a grid origin from counts already in the terrain uniform, and adjacent chunks collapse into
  one draw.
  - Cascades cull against their own frusta and not the camera's, because a cascade reaches *behind* the
    camera toward the light — a caster off screen still throws a shadow into view, and culling it against
    the camera would make shadows wink out as their caster left the frame.
  - **Culling against the pass's own matrix cannot change the image, and that is the safety argument.**
    Each frustum is extracted from the very view-projection that pass renders with, so a chunk the test
    rejects is one the rasterizer would have clipped anyway. The only way to lose visible ground is to
    extract the planes wrongly — which is what the byte-identical references disprove, and what the unit
    tests attack directly, including the near-plane convention that would otherwise cull the whole world.
  - **It is invisible, and that is how it is verified.** Every committed reference still matches byte for
    byte with culling active. What the references could *not* cover is a partial chunk — 192 cells and 128
    both divide evenly by 32 — so there is a test for the ragged case, and it was confirmed by breaking the
    degenerate-vertex handling on purpose and watching it fail with the predicted number.
  - The win is a function of map size, and honestly nil at the size the fixtures use: about 0.008 ms on a
    257x257 terrain, where the camera sees most of the map anyway. On a **1025x1025** one, which is the size
    this project is actually aimed at, the four cascades go from **0.809 ms to 0.131 ms** and the G-buffer
    from **0.239 ms to 0.071 ms** — the frame from **1.534 ms to 0.692 ms, 55% off**. The number worth
    keeping is the other one: at 0.692 ms that sixteen-times-larger terrain costs the *same* as the small
    one, so terrain no longer scales with map size at all.

- **A half-resolution occlusion estimate**, which is what the timing was built to find and immediately
  paid for itself. One estimate per 2x2 block of render pixels, resolved back to full resolution by the
  bilateral pass that was already there — so that pass is now the upsample as well as the blur, weighting
  each tap by the world distance to the render pixel the estimate was actually *taken* at. What halves is
  the number of estimates, not the resolution of anything they read: each one still loads full-resolution
  depth and normals and still walks its slices at full-resolution spacing.
  - At 1920x1200 the estimate went from **0.668 ms to 0.303 ms** and its resolve from **0.161 ms to
    0.075 ms**, taking the summed frame from **1.160 ms to 0.677 ms — 42% off**. Occlusion is still the
    largest single cost at 56% of the frame, down from 72%.
  - The blur radius is 1 in half-resolution taps, not 2. Two was tried first on the reasoning that coarser
    noise needs a wider kernel; the captures said the opposite, and said it twice — 3x3 shows no more noise
    than the old 5x5 *and* lands closer to the full-resolution frame it replaces, because 5x5 in
    half-resolution space was simply over-blurring.
  - Every committed reference except the forward-pass one moved, by 0.1% to 0.6% of pixels at a peak
    channel difference of 6 — sub-perceptual, and confirmed by magnifying the spire's contact shadow, the
    concave bowl, and a building base against the pre-change captures before regenerating.

- **A GPU-capable CI runner**, which is what makes the harness able to fail a build. The runner installs
  Mesa's lavapipe, a software Vulkan implementation, so an adapter exists where there was none; before
  this `GpuContext::new` found nothing and every rendering test skipped, which had been true since the
  first render test landed.
  - **Installing the rasteriser was the easy half.** A skipped rendering test and a passing one are the
    same colour, so a runner whose adapter quietly stopped being usable would have left the job green
    while drawing nothing — indistinguishable from the state before the rasteriser existed.
    `CIC_REQUIRE_ADAPTER`, which CI sets, turns that into a failure, and it was verified by making it
    fire rather than by assuming it would.
  - `WGPU_BACKEND` turned out to do nothing: `wgpu::Instance::default()` reads none of the `WGPU_*`
    variables, so pinning the backend in CI would have been configuration that silently had no effect.
    `GpuContext` now builds its instance from the environment, which is also the only way to reach the
    no-adapter path on a machine that has a GPU.
  - The runner label is pinned to `ubuntu-24.04` rather than `ubuntu-latest`, because lavapipe's adapter
    name carries the LLVM version it was built against — the set is keyed
    `vulkan-llvmpipe-llvm-20-1-2-256-bits` — so an image rollout to a new Ubuntu release would orphan the
    committed set and fail every rendering test asking for a new one.
  - Captures and reference images upload as an artifact on every outcome, because the standing rule here
    is that a rendering change is verified by looking at the image, and a harness failure was otherwise
    leaving the capture and its amplified difference on a disk nobody can reach.

- **Physically-based model maps** ([`model`](../../crates/cic-render/src/model.rs)). Normal, roughness and
  metallic maps alongside the base colour, with the tangent frame the first of them needs — read from
  `TANGENT` where a model supplies one and derived from the texture coordinates where it does not, which is
  the ordinary case rather than the exception. Three texture arrays per model rather than one, because base
  colour is sRGB-encoded and the other two are linear measurements and one array has one format; see
  [`ColourSpace`](../../crates/cic-render/src/texture.rs) for what a normal map decoded as a colour does.
  - **Metallic cost no G-buffer bandwidth at all.** The albedo target's alpha channel was writing a constant
    1.0 and nothing read it, so a fourth material channel was already paid for. Eight bits is ample for a
    quantity that is 0 or 1 on almost every real material.
  - Every expression the lighting pass gained **reduces to its predecessor at zero metalness**, by
    construction rather than by tuning: a metal loses its diffuse term through a multiply by `1 - metallic`
    and takes its highlight colour through a `mix` from the dielectric constant. That is what let every
    committed reference stay byte-identical when the channel arrived.
  - What a metal *cannot* look like here is a mirror, and the reason is structural rather than a gap in
    this work: there is no environment probe, so a metal reflects the three light slots and the sky
    gradient and nothing else. It reads as darker with a coloured highlight, which is correct and
    incomplete. The light slots' own comment already anticipates the probe.
- **Alpha-tested materials** ([`model`](../../crates/cic-render/src/model.rs)), which is how foliage is
  authored. A material that cuts its own silhouette gets its own index range and its own pair of
  pipelines, so opaque geometry keeps its early depth rejection and its fragment-free shadow pass — a
  fragment stage that *can* discard forfeits early-Z on most hardware, and opaque geometry is the
  overwhelming majority.
  - The alpha test had to reach **every shadow cascade**, not only the lit frame. A leaf card casting the
    rectangle its geometry occupies is worse than one casting nothing, because the eye reads a hard
    quadrilateral on the ground as a solid object — so a canopy would darken the terrain in slabs.
  - Alpha-tested and two-sided are **one split rather than two**. They arrive together in practice and each
    is nearly free to grant to the other: foliage needs both, and an opaque material that merely asked to be
    double-sided reads a zero cutoff as "never discard". Splitting four ways would double the pipeline count
    to separate cases no content has asked for.
- **Scenery sway** ([`scenery`](../../crates/cic-render/src/scenery.rs)), written from scratch — the last
  outstanding provenance case in [LICENSING.md](../../LICENSING.md), now closed. Every constant is derived
  in that file from a stated physical argument, because a number nobody can justify is indistinguishable
  from a number that was copied.
  - The model is three parts on three time scales: a steady bend along the wind, a slow oscillation about it
    at the plant's own natural frequency, and a fast cross-wind flutter. Four profiles rather than a longer
    table, each a distinct physical regime, with anything between them reachable by constructor.
  - Two things would give the trick away at a glance and both are fixed by the *phase*. A stand moving in
    unison is the obvious one, so each instance derives its own phase from its world position — integer
    arithmetic, so it is identical every frame, every run, and in a capture. The subtler one is that wind
    arrives somewhere first: the phase also carries a term along the wind direction, and the visible result
    is a front crossing the map. That term costs one dot product and contributes more to the effect reading
    as weather than anything else in the file.
  - The flutter is at **5.37 times** the sway rather than 5, and the reason is this renderer's own history:
    five summed water waves at related wavelengths interfered into a visible diamond lattice, recorded in
    the design notes below. Two motions at an integer ratio repeat every slow cycle and the repeat is what
    the eye finds.
  - Sway is per-*instance* data rather than a bound uniform, and that is structural. The displacement has to
    be identical in the G-buffer, in all four cascades and in the motion vector, and the instance buffer is
    the only per-draw data every one of those passes already binds.
- **A virtual-texture page cache on the GPU**
  ([`terrain_page`](../../crates/cic-render/src/terrain_page.rs)), which is the consumer the residency
  bookkeeping never had. Physical pages as an array texture, a page table per level, and a compute pass that
  composes the layer blend once per page instead of once per fragment per frame — the blend depends only on
  the terrain data, so recomputing it every frame for ground that has not changed is the waste this removes.
  It is also what lets detail scale past one texture: a page is composed at a density chosen for how close it
  is, so the ground under the camera carries far more texels per metre than a map-wide texture could afford
  everywhere.
  - **The compose shader was rewritten rather than wired up.** The staged one composed pages from a tile
    atlas — per-cell material slots, blend masks with orientation codes, an edge-tile sheet, a macro lattice
    — and this terrain is a heightfield plus per-layer weights, so every input it declared was a resource
    this engine does not build. There was nothing to connect. See
    [LICENSING.md](../../LICENSING.md), which records both that the file carries no derivation and that it
    was written for a terrain this engine does not have.
  - **The compose pass binds the terrain's own uniform buffer**, so it cannot disagree with the G-buffer
    about the terrain it is composing. It also has to choose its own mip level, because a compute shader has
    no screen-space derivatives — and that turns out to be the better answer rather than a workaround: a
    page's texel density is a property of the page, not of whoever is looking at it.
  - **A page carries a four-texel border of the neighbouring ground.** Without it a filtered tap at a page
    edge clamps, which puts a seam along every page boundary — and boundaries are fixed to the ground rather
    than to the screen, so the seams would crawl as the camera moved. A test reads a page back and checks
    that a page straddling a colour boundary carries the far side in its border: interior red 89 against
    border red 144, where a clamped border would read 89.
  - **What remains is the fragment path.** The terrain G-buffer does not sample pages yet, and that is
    deliberately a separate step: switching it over changes every terrain frame, and the honest order was to
    prove the composition first. The properties that decide whether the cache is usable at all are verified
    by readback rather than by a rendered comparison, because a render cannot isolate them — see
    `tests/terrain_render.rs`.
- **Temporal antialiasing** ([`deferred`](../../crates/cic-render/src/deferred.rs), `taa.wgsl`), the last
  tier of [ADR 0005](../adr/0005-antialiasing-strategy.md) and the last item on its list: a jittered
  projection on an eight-phase Halton sequence, a motion-vector target, a ping-ponged float history, and a
  neighbourhood clamp in YCoCg. **0.053 ms** at 1920x1200, twice the post pass and a nineteenth of the
  frame.
  - The jitter phase is a frame *parameter*, like scene time and for the same reason, which is what makes a
    temporal capture reproducible: the harness renders a full cycle and compares the last frame, and two
    runs agree byte for byte.
  - It needed one API the design did not have. Two sequences in a row disagreed until `reset_history` was
    added, because the second started from what the first left behind — and that is the real-game case of a
    jump cut rather than a peculiarity of the test.
  - **It found a defect the whole reference set had been rendered through.** Three resolve passes were
    adding half a pixel to a framebuffer coordinate that already carries it, so each sampled half a pixel
    away from the fragment it was shading. Nothing caught it because every reference had been rendered
    through the same offset; what caught it was that accumulating a static frame never reached a fixed
    point. Ten of the twenty-two references changed. The measurement, the cost in each pass, and why a
    capture comparison structurally cannot catch this are in [ADR
    0005](../adr/0005-antialiasing-strategy.md#what-implementing-decision-4-established).
  - Motion vectors are **exact for swaying geometry**, not approximate, and it cost nothing: the
    displacement is a pure function of scene time, so the same vertex function evaluated at the previous
    time returns exactly where the vertex was.

## Remaining

- **Terrain level of detail.** Frustum culling has landed and is half the charter item — see Landed — but
  a chunk that *is* visible still draws every cell it has, whether it fills forty pixels or four. The chunk
  decomposition culling introduced is what LOD needs: a per-chunk stride chosen from distance, and either
  skirts or edge stitching so neighbouring densities do not crack.
  - Note what this is *not*. [`terrain_virtual`](../../crates/cic-render/src/terrain_virtual.rs) and
    [`detail`](../../crates/cic-render/src/detail.rs) are residency bookkeeping for terrain *texture*
    pages, decided in texels per cell. They are unwired, and wiring them would not remove a triangle.
- **Sampling composed pages in the terrain G-buffer.** The cache exists and composes correctly — see
  Landed — and the fragment path still does the eight-layer blend itself. What it needs: two more bindings on
  the terrain group, a page-table lookup to resolve a cell to a physical layer, and a fallback to the direct
  blend for a cell whose page is not resident. The fallback is not optional: a cache is allowed to run out of
  slots, and a frame must not depend on it having won.
  - Page mip chains go with it. A page has one level, so a page sampled at a shallow angle would alias where
    the direct blend does not — the terrain would look *worse* on exactly the ground a virtual texture is
    for. The downsample is a second compute pass over the resident pages.

## Exit condition

A map package loads and renders, in a window and headlessly, at a stable frame rate; the visual
regression harness runs in CI with committed references.

**Met.** The renderer half was already done and the harness works: it catches a change of four 8-bit
steps in one channel — a difference invisible to the eye — across 90% of a frame, verified by perturbing
the exposure constant and watching all five references fail.

"In CI" is now met too. The runner installs Mesa's lavapipe, the rendering tests execute there rather
than skipping, and eleven references are committed under
`references/vulkan-llvmpipe-llvm-20-1-2-256-bits/` alongside the NVIDIA set. The whole suite passes on the
runner exactly as on a developer machine — the count lives in
[CURRENT.md](../../CURRENT.md#gate-status), so it has one home — with the three rendering binaries taking
about eleven seconds between them. Software rasterisation turned out far cheaper here than expected, which
is what makes this viable on every pull request rather than nightly.

The reference set was generated on the runner and each image was opened and checked before being
committed, which is the step the mechanism is built around: the harness deliberately *fails* when a
reference is missing rather than accepting whatever it first rendered.

**References could not have been copied from a developer machine, and now there is a number for it.**
Comparing the two sets under this tolerance, the nine scenes that sample no texture agree — 0.0191% of
pixels at worst, peak channel difference 9, comfortably inside the allowance — while the two that sample
one are rejected, textured models by 0.3487% and the tiled terrain albedo by 11.4092%. See
[`regression`](../../crates/cic-render/src/regression.rs) for what that implies, because the ratio
between those groups is about six hundred and it locates the cause precisely.

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

**The first thing per-pass timing did was refute the reason it was built.** The case for it was that the
terrain submits its whole heightfield five times a frame with nothing culled, and that this was presumably
where the time went. It is not. At 1920x1200 the four shadow cascades come to **7%** of the summed passes
and the G-buffer to 6%, while ambient occlusion is **58%** and its blur another 14% — the frame is
overwhelmingly fragment-bound in one screen-space pass, and barely troubled by two million vertices. The
same numbers measured at 720x480 said the cascades were 36%, which is what a small render target does to a
ratio: the cascades draw into fixed-resolution shadow maps and so cost the same at every window size, and
at 720x480 there was nothing else large enough to compare against. **A profile taken at a resolution nobody
plays at ranks passes in an order nobody experiences.**

That does not retire terrain LOD — it is a charter item, it matters more on a larger map and a weaker GPU,
and two million unculled vertices is not a thing to leave standing. It does move it behind the occlusion
pass, and it settles the depth pre-pass question outright: a pre-pass buys back fragment work in the
G-buffer, which is 6% of the frame, by paying for a second full geometry submission. There is nothing there
to win.

**A frame cannot carry a size the targets already decide.** `DeferredFrame` used to hold a viewport, and
the reason it did was itself a fix — the figure had previously been passed twice and nothing checked the two
agreed. But the size the shaders reconstruct world positions against is a property of the *targets*, since
they are the textures being loaded from, so putting it on the frame left the same disagreement one step
further along: `SurfaceRenderer::render` had to overwrite whatever the caller supplied, because a frame one
resize behind moved every receiver and read as a shadowing fault rather than as a wrong number. A
resolution scale made this worse rather than better, splitting the one figure into two. The field is gone
and both sizes now come from `DeferredTargets`, which is the only thing that knows them. The same
reasoning removed `output_format` from `DeferredRenderer::new`: the LDR intermediate has to be *allocated*
in that format, so the two were bound to agree and nothing said so.

**A prefix is a contract, and nothing enforced it.** `terrain_ao.wgsl` binds the scene uniform through its
own struct declaring only the fields it reads, which is sound precisely while that declaration stays a
prefix of the full block. Adding the output size *after* the weather vector keeps it sound; adding it after
the viewport, where it reads better, would have misaligned every field past it in a shader that still
validates and still runs. There is now a test asserting the shared leading fields, since the failure mode
is silent in both directions.

**A shadow fixture has to be shaped for the sun it is lit by.** Two separate versions of the test
measured nothing: one placed the sun so every shadow fell behind its own caster and out of frame; the
other used a ridge wider than its own shadow was long, so the shadow landed entirely on the ridge's
unlit back slope. Neither was a renderer fault, and neither was visible from the assertions.

## Explicitly not done

**The ambient-occlusion map is read from glTF and not applied.** `ModelMaterial::occlusion_texture` and its
strength are imported, and the renderer ignores both. Occlusion is an *ambient-only* multiplier and there is
no G-buffer channel left for one: folding it into albedo would darken the direct term too, which is
precisely what an occlusion map must not do. Widening the coverage target to two channels is the change if
content ever ships one; until then this is a slot the importer fills and the renderer declines, which is
better than a slot nobody can find.

**Alpha *blending* is not supported, and cannot be without a second path.** A G-buffer pixel holds one
material, and blending needs two. `AlphaMode::Blended` is therefore drawn as a cut at half coverage —
`AlphaMode::cutoff` says so and says why — which is the closer of the two available wrong answers. Real
blending wants a forward pass after the composite, which is a larger decision than this milestone.

**The sway does not rotate normals.** The bend is a few degrees over a whole plant, well under the variation
a normal map already carries, and deriving the rotated normal correctly needs the displacement's gradient —
several times the cost of the displacement itself.


- No post-processing chain beyond what the shader set already covers.
- No particle system; it belongs with the gameplay that spawns effects.
- No level-of-detail generation for models, and none for terrain either. A heightfield's regularity makes
  terrain LOD *cheap to build*, which is why the format is shaped to permit it — see
  [the terrain format](../formats/terrain.md) — but nothing has been built. This entry used to read
  "Terrain has it", which was simply false, and it is the reason the charter above is not complete. Both
  now want the same thing first: measurement.
- **MSAA is declined outright**, rather than pending. Multisampling a deferred G-buffer means four times
  the memory on every target *and* per-sample lighting behind a stencil pass, because averaging normals
  or depths across a silhouette yields values describing no surface that exists — and having paid for all
  that it still fixes only geometric edges, not the texture and specular aliasing that are now the more
  visible cases. This entry previously appeared under both "remaining" and "not done", which read as an
  omission nobody had decided about. [ADR 0005](../adr/0005-antialiasing-strategy.md) decides it.
- No edge search in the post pass, so it softens each step of a long shallow staircase rather than
  reconstructing the line, and it cannot tell a one-pixel bright feature from a one-pixel artifact —
  because at that size the image holds no difference between them. Both are the cost class, and both are
  what TAA is for.
- No claim that a resolution scale *preserves fine detail* an upscale destroys. It should, and the fixture
  cannot show it: the shadowing terrain is a smooth heightfield in flat palette colours, so off the
  silhouette the three scales measure 0.00046, 0.00049 and 0.00045 — three numbers agreeing rather than an
  assertion. Making that claim needs a textured fixture, and the world-space tiled albedo in
  `terrain_render.rs` is the candidate.
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
