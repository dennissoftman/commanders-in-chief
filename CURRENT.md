# Current objective

## Where the project is

M0 through M2 are complete: the workspace and its invariants, the resource layer, and the native asset
formats. **M3's charter and its exit condition are both met** — the renderer draws a lit, shadowed,
occluded, textured scene with water and weather, both headlessly and in a window, and a visual regression
harness compares captures against committed references **on the CI runner**, so a rendering regression now
fails a build rather than only a developer's machine.

The last open line was **terrain level of detail**, and it is closed by amendment rather than by
implementation: frustum culling delivered what it was for, and the density half is deliberately not built.
The measurement is the reason and the decision is recorded with a date and an owner — see
[M3's charter](docs/milestones/m3-renderer.md#charter). What is *not* acceptable, and was true of this
document twice, is a line reading as though something were done.

**M4 is under way.** A `cic-ui` crate holds the [layout format](docs/formats/ui-layout.md), a two-pass
solver, a string table, the closed action set, and widget behaviour: retained state keyed by node id,
semantic input routing with focus and keyboard navigation, input-method composition, and now **tabs that
switch pages** rather than merely holding a number. Nothing draws it yet, and there is no screen stack.

Landing M3's last five renderer items turned up a defect every committed reference had been rendered
through, so **ten of the twenty-two references changed** — see the antialiasing entry below.

What works:

- A `cic-assets` terrain uploads to the GPU and renders through a seven-pass deferred chain: four shadow
  cascades, a G-buffer, a half-resolution ambient-occlusion estimate with a bilateral upsample, deferred
  lighting that reconstructs world position from depth, a blended water pass, and a composite that tone maps
  and resolves the render resolution to the caller's. An eighth pass antialiases, when the display settings
  ask for it. The whole chain sums to **0.68 ms** at 1920x1200 on an RTX 4080 SUPER.
- Heights and layer weights live in *writable* textures with displacement and normals computed in the
  vertex shader, so terrain deformation and route grading are texture writes rather than a remesh.
- Instanced models share the terrain G-buffer and every shadow cascade, one draw call per model
  whatever its material count, with a per-instance transform and colour tint.
- Both surfaces are textured through one mechanism: a colour texture array per drawing unit, indexed by
  a slice number the material carries. Terrain layers tile their albedo in *world* space at a per-layer
  detail scale; model materials take their base colour from the images their glTF carried. Mip chains
  are generated on the CPU in linear light.
- **Water**, as a bounded plane with five summed directional waves, blended inside the HDR target so its
  glitter tone maps with the scene. Its shoreline is not authored: the pass discards wherever the bed
  rises through the displaced surface, so a rectangle plus a heightfield give an irregular shore that
  moves with the swell.
- **Shaders compose from named chunks** ([`shader`](crates/cic-render/src/shader.rs)). WGSL has no include
  mechanism, and without composition every pass needing the cascade selection had to share one file with
  it — which is how a single shader reached 620 lines. Thirteen programs are assembled from sixteen chunks;
  a test fails if any chunk is named by no program, and five programs are marked `staged` so work held for
  a later milestone is distinguishable from dead code.
- **An atmosphere** ([`environment`](crates/cic-render/src/environment.rs)) derived from two authored
  numbers, an hour and a weather state: the sun's direction and colour, ambient, sky, fog, and cloud
  coverage all follow from them. Weather is blendable scalars rather than an enum of presets.
  - **Cloud shadows** — procedural gradient noise in world space, domain-warped into wisps, attenuating
    the sun's direct term only and with a depth that varies with cloud thickness.
  - **Height and distance fog** — marched along the view ray in six taps, so a valley pools while a ridge
    stands out of it.
  - **A weather surface response** — wetness darkening albedo and dropping roughness, and snow settling by
    *slope* so it lies on flats and not on cliffs. Applied to the G-buffer in the lighting pass, so terrain
    and models both get it from one implementation.
- **Antialiasing, in its two cheaper tiers** ([`display`](crates/cic-render/src/display.rs)), with MSAA
  declined outright per [ADR 0005](docs/adr/0005-antialiasing-strategy.md) rather than left open. A
  **resolution scale** from a half to two multiplies every screen-space target and the composite's filtered
  read of the HDR target is the downsample, so it costs no extra pass — and it is the only control that
  raises the actual sampling rate, so the only one that touches geometric, texture, specular *and*
  occlusion aliasing at once. Beneath it a **post pass** written from scratch: a luma gate, a Sobel
  orientation test, and a blend weight from the second difference across the edge, so smooth ramps are
  untouched, a hard step is halved, and an isolated sub-pixel highlight is hit hardest. Both are one
  `DisplaySettings` value, because a settings screen presents them as one choice.
  - Two findings came out of it. The composite's sharpen was *fighting* the scale — it amplifies soft edges
    hardest, and a supersampled silhouette is soft by construction — so it is now off above a scale of one.
    And no single statistic separates aliasing from detail: the obvious one reports supersampling as a
    regression, and what works is measuring *where* a setting acts rather than how much. Both are written up
    in the ADR.
- **A virtual-texture page cache** ([`terrain_page`](crates/cic-render/src/terrain_page.rs)), which is the
  consumer the residency bookkeeping never had: physical pages, a page table per level, and a compute pass
  that bakes the layer blend once per page instead of once per fragment per frame. The blend depends only on
  the terrain data, so recomputing it every frame for unchanged ground was the waste; and a page composed at
  a density chosen for how close it is, is what lets detail scale past one map-wide texture.
  - The compose shader was **rewritten, not wired up**: the staged one read a tile atlas with blend masks and
    edge tiles, and this terrain is a heightfield plus layer weights, so every input it declared was a
    resource the engine does not build.
  - A page carries a four-texel border of the neighbouring ground, because a filtered tap at a page edge
    reads across it — and a clamped border would put a seam on every boundary, crawling as the camera moves.
    Verified by reading a page back: a page straddling a colour boundary shows border red 144 against
    interior red 89, where a clamp would read 89.
  - The G-buffer samples pages once a cache is attached, and the two paths agree to a **mean of 0.004** and
    a worst case of **5** eight-bit steps. The direct blend stays as the fallback: a cache may run out of
    slots, and a frame must not depend on it having won — a one-slot cache draws 99.9% of the frame from the
    fallback.
  - **Page mip chains**, which took the path from correct to *better*. A page held one density, so ground
    under heavy minification aliased where the fallback — which samples an albedo array that has a chain —
    did not, making the cache worse than not using it on exactly the ground it exists for. Each update now
    reduces the pages it composed, one compute pass per level, and the G-buffer derives a level from
    screen-space derivatives because a residency branch is not uniform control flow.
    - **The border is the chain's budget**, which is why it went from four texels to eight: every reduction
      halves it, a filtered tap needs a whole texel of it, so `2^n` buys `n + 1` levels and four levels cover
      a minification of eight. Get it wrong and the seam the border prevents returns at every level *below*
      the base, on ground the base looks perfect on — so it is read back rather than rendered: border red
      184, 180, 170, 149 down the chain against an interior of 89, where a clamp would read 89.
    - **The reduction averages in linear light**, matching a prediction to **0.54** of an eight-bit step,
      where a byte-space average would be out by **26**. At a grazing angle the paged frame is now *smoother*
      than the blend it replaces — **238 against 386** — where sampling the base level only reads 2.00 times
      the blend.
  - Three fixtures in a row could not show what they measured before one did, which is the standing warning
    below earning its place a third time: `surface()` divides by the summed weight, so a single ramped layer
    normalizes to a constant. A fourth mistake of the same shape followed — the agreement test first drew
    through the *forward* pass, which has no page lookup, and reported the two frames as identical. The mip
    chain made it five and six: a flat-coloured page cannot tell a linear-light average from a byte-space
    one, and an aliasing metric measured across the axis the fixture's stripes run along reads 1.49 where the
    other axis reads 158.
- **A physically-based map set for models**: normal, roughness and metallic maps beside the base colour,
  with the tangent frame the first of them needs — read from glTF's `TANGENT` where a model supplies one and
  derived from the texture coordinates where it does not, which is the ordinary case rather than the
  exception. Three texture arrays per model rather than one, because base colour is sRGB-encoded and the
  other two are linear measurements and one array has one format: decoding a normal map as a colour turns a
  flat 128 into 0.216 instead of 0.502, which tilts the whole surface and reads as a lighting bug.
  - **Metallic cost no G-buffer bandwidth.** The albedo target's alpha was writing a constant 1.0 and nothing
    read it. Every expression the lighting pass gained reduces to its predecessor at zero metalness by
    construction, which is what kept the references byte-identical when the channel arrived.
  - Read from glTF and deliberately *not* applied: the occlusion map. Occlusion is an ambient-only
    multiplier and there is no channel left for one — folding it into albedo would darken the direct term,
    which is precisely what it must not do.
- **Alpha-tested materials**, which is how foliage is authored. A material that cuts its own silhouette gets
  its own index range and its own pipelines, so opaque geometry keeps its early depth rejection and its
  fragment-free shadow pass. The cut reaches **every shadow cascade**: a leaf card casting the rectangle its
  geometry occupies is worse than one casting nothing, because a hard quadrilateral on the ground reads as a
  solid object.
- **Scenery sway** ([`scenery`](crates/cic-render/src/scenery.rs)), written from scratch — three parts on
  three time scales, four profiles rather than a longer table, and every constant derived in the file from a
  stated physical argument. **This closes the last outstanding provenance case.**
  - The phase is derived from each instance's world position with integer arithmetic, so a stand of plants is
    never in unison and a capture is still reproducible. It also carries a term along the wind direction, so
    a gust visibly crosses the map — one dot product, and the single largest contributor to the effect
    reading as weather rather than as animation.
  - The flutter is at 5.37 times the sway rather than 5, because this renderer has already been caught by
    near-harmonic ratios once: five water waves at related wavelengths interfered into a visible lattice.
- **Temporal antialiasing** — the last tier of [ADR 0005](docs/adr/0005-antialiasing-strategy.md), and the
  last item on its list: a jittered projection on an eight-phase Halton sequence, a motion-vector target, a
  ping-ponged float history, and a neighbourhood clamp in YCoCg. **0.053 ms** at 1920x1200, twice the post
  pass and a nineteenth of the frame.
  - The jitter phase is a frame parameter like scene time, which is what makes a temporal capture
    reproducible: the harness renders a full cycle and compares the last frame. It also needed an API the
    design lacked — `reset_history`, for a frame that does not continue the last one, which is the real-game
    case of a jump cut.
  - Motion vectors are **exact for swaying geometry** rather than approximate, and it cost nothing: the
    displacement is a pure function of scene time, so the same vertex function at the previous time returns
    exactly where the vertex was.
  - **It found a defect the whole reference set had been rendered through.** Three resolve passes were adding
    half a pixel to a framebuffer coordinate that already carries it, so each sampled half a pixel away from
    the fragment it was shading — a translation of every frame, plus a two-texel average where the downsample
    should return one exact texel. Nothing caught it because every reference had been rendered through the
    same offset. What caught it was that accumulating a *static* frame never reached a fixed point: successive
    frames differed by 48, 33, 19, 9, 6 — convergent, and so passing any tolerance stated as "settles".
    Measured on the deferred fixture at 1.5% of pixels differing by more than two, peak 154. A textual test
    now pins the convention, because this is the one class of error a reference comparison structurally
    cannot catch.
- **Time of day drives the light by default.** `DeferredFrame::new` derives its sun from the environment,
  and `in_environment` re-derives it, so changing the hour moves the sun rather than leaving a light that
  silently disagrees with it. The derivation is calibrated against the hand-tuned preset it replaced and a
  test pins it there.
- **Scene time is a frame parameter** — `DeferredFrame::time` — and nothing in the renderer reads a
  clock. That is what makes a capture of moving water or drifting cloud reproducible.
- **Per-pass GPU timing** ([`timing`](crates/cic-render/src/timing.rs)), because every performance question
  here is workload-dependent: a total says something is slow, a breakdown says which pass. Each pass owns a
  fixed pair of timestamp queries, a skipped pass reads back as absent rather than as zero, and the
  tick-to-duration arithmetic is a pure function with its own tests. Optional, since `TIMESTAMP_QUERY` is —
  a device without it renders identically and reports nothing.
  - It refuted its own premise immediately. The terrain's two million unculled vertices a frame were the
    reason to build it, and at 1920x1200 the four cascades were 7% of the frame while ambient occlusion was
    58%. Measured at 720x480 the same code said the cascades were 36%, because the cascades cost the same at
    every window size and a small target leaves nothing to compare them against.
- **Terrain frustum culling** ([`culling`](crates/cic-render/src/culling.rs)), over a decomposition into
  32-cell chunks. The camera and each fitted cascade cull against their own frustum and the survivors draw
  as instanced runs — the instance index *is* the chunk index, so this needed no new binding, buffer or
  upload. Cascades use their own frusta rather than the camera's, because a cascade reaches behind the
  camera toward the light and a caster off screen still casts into view.
  - Verified by being invisible: every committed reference still matches byte for byte. The one case they
    could not cover is a chunk the terrain does not fill — 192 cells and 128 both divide evenly by 32 — so
    that has its own test, confirmed by breaking the shader on purpose and watching it fail at the
    predicted figure.
  - The win scales with the map and is nil at fixture size: 0.008 ms on a 257x257 terrain. On a
    **1025x1025** one the cascades go from **0.809 ms to 0.131 ms** and the G-buffer from **0.239 ms to
    0.071 ms**, taking the frame from **1.534 ms to 0.692 ms**. The figure that matters is that 0.692 ms is
    what the *small* terrain costs too: terrain no longer scales with map size.
- **A half-resolution occlusion estimate**, which is what that measurement led to. One estimate per 2x2
  block of render pixels, resolved back to full resolution by the bilateral pass that already existed — now
  the upsample as well as the blur, weighting each tap by the world distance to the render pixel its
  estimate was actually *taken* at. What halves is the number of estimates, not the resolution of anything
  they read.
  - At 1920x1200 the estimate went from **0.668 ms to 0.303 ms** and its resolve from **0.161 ms to
    0.075 ms**: the summed frame is **1.160 ms down to 0.677 ms, 42% off**. Occlusion is still the largest
    single cost, at 56% of the frame rather than 72%.
  - Its blur radius came *down* rather than up, against expectation — a wider kernel over coarser noise was
    the guess, and the captures said 3x3 half-resolution taps show no more noise than the old 5x5 while
    landing closer to the frame they replace.
- Windowed presentation, driven by the reusable camera:

```bash
cargo run -p cic-render --example terrain_viewer --release
```

Pass a `.cicmap` path to view a real map; with no argument it generates terrain, buildings, a water
table derived from the heightfield's own low point, and their surfaces, so the viewer runs before any
content exists. `T` toggles antialiasing and the bracket keys step the resolution scale, because what an
edge does *as the camera moves* is the whole subject and no still capture reports it; `P` prints the
per-pass breakdown once a second, which is where the figures above came from; and **`V` toggles the
virtual-texture cache**, for the same reason as `T` — a crawling page seam, a step between mip levels, and a
page arriving a frame late are all motion artefacts, and until this key existed the cache was reachable only
from a test. Verified by running it: 256 pages compose on the frame the key is pressed and the window is
indistinguishable from the direct blend, which is what it should be at a camera height where no page is
minified.

## Next verified step

**M4's interface layer, continued.** Its foundation has landed — see
[M4](docs/milestones/m4-interface.md) — as a `cic-ui` crate depending on nothing but `serde`: the
[layout format](docs/formats/ui-layout.md), a two-pass solver producing pixel-snapped rectangles, a
string table, the closed action set, widget behaviour, and tab pages. What is next, in order:

1. **The screen stack**, and the transactional settings apply the milestone's design notes require: a
   display change has to survive a revert timer rather than depend on the user being able to see well
   enough to click undo.
2. **Drawing it.** `ui.wgsl` is already in the shader set marked `staged` for this, and the capture
   harness that will cover the result now runs in CI — which is what makes it worth covering. Text
   rendering is the substantial part.

**Widget behaviour and input routing have landed**, including input-method composition — because a single
character per keystroke is the Latin case, and assuming it is the only one is how an engine ends up unable
to accept CJK text without being rebuilt. Retained state keys off node ids, which is why the format now
*requires* one on any widget holding state or taking focus rather than treating it as optional.

**Tabs switch pages**, which they did not before: `Widget::Tabs` tracked a number and nothing acted on it,
while the format's own comment claimed it switched between sibling pages. A strip's children are its headers
and a `pages` field names the stacked container holding the bodies, checked against each other at load —
three headers over two pages is a screen whose third tab shows nothing, and neither node is wrong on its own.
Visibility is decided in the *solver*, which is the one place state flows into layout: hit testing, keyboard
navigation and drawing all read the same solved sequence, so one of the three forgetting to filter is a
control the user cannot see taking a click. The consequence is that a tab change is a relayout, exactly as a
resize is.

The settings screen has real content waiting for it, since a display setting exists with more than one
option — and now a way to arrange it.

**The remaining renderer item is a view-driven detail request**, and the mip chain is what showed it
matters. A page's chain is four levels, covering a minification of eight; past that a page saturates while
the direct blend's albedo chain keeps going, which is why the grazing-angle capture still reads 1.93 in its
topmost thirty rows of ground against 0.62 everywhere nearer. Ground that far should have no page resident
at all, and *which ground has a page* is the residency decision nothing derives from a camera yet. The
residency map already ranks by projected size, so this is a small function over the frustum rather than a
design.

## Gate status

Formatting, strict lints (`clippy::all` and `clippy::pedantic` as errors, plus `-D warnings` as CI runs
it), and the full test suite all passes on the pinned toolchain: **418 tests across six crates**. The CI
runner runs the same suite against Mesa's lavapipe.

**No reference moved for the mip chain, which was not the expectation.** Every committed NVIDIA reference
still matches byte-for-byte within tolerance, including `terrain-from-pages.png` — the paged frame changed by
a mean of 0.004 and a worst case of 5, which is inside what the comparison allows in a small region while
still failing on four steps across most of a frame. The grazing-angle scene the chain is verified on
deliberately has **no committed reference**: its claim is a statistic about adjacent-pixel energy rather than
an image, and adding a reference scene would force a lavapipe capture from the runner for nothing the numbers
do not already say. Whether lavapipe's own filtering keeps `terrain-from-pages.png` inside tolerance is the
one thing this branch cannot check locally.

**The lavapipe reference set still has to be regenerated on the runner before CI can pass.** The half-pixel
fix above changed ten of the eleven scenes, and those images can only be rendered where lavapipe is — so a
branch carrying that fix fails CI once, with the captures and difference images uploaded as an artifact for
review, which is the flow the harness is built around for a deliberate rendering change. The NVIDIA set is
regenerated and each image was opened and checked. The rendering ones take about eleven
seconds there, which is what makes this affordable on every pull request. Captures go to `target/tmp/` and
upload as an artifact on every outcome, so a harness failure's capture and amplified difference image can
be looked at rather than being stranded on the runner.

Sixteen references per adapter cover terrain layers, instanced models, the deferred chain, water, water
under a glancing sun, cloud shadows, fog, wet ground, snow, an antialiased frame, a supersampled one, a
temporally accumulated one, a normal-mapped model, a metallic one, alpha-tested foliage, and a swaying
canopy — one set for an NVIDIA RTX 4080 SUPER and one for lavapipe, each generated on its own machine and
looked at before being committed.

The render tests still skip rather than fail when no adapter is available, so a developer machine with no
GPU reports honestly instead of red. **CI sets `CIC_REQUIRE_ADAPTER`, which makes the same situation a
failure there**, because a skipped rendering test and a passing one are the same colour and a runner that
silently lost its adapter would otherwise leave the harness protecting nothing. The regression comparison
itself is a pure function over bytes with its own unit tests, so that half is verified even with no GPU
present.

## Standing constraints

- Nothing in this tree derives from another game's code or reads another game's data. See
  [LICENSING.md](LICENSING.md). Water was written from scratch and the removed shader was not consulted;
  **scenery sway has since been written the same way, which closes the last outstanding case.** The rule
  that nothing may be copied backward across `5e824cf` still stands, and still has nothing left that wants
  to break it.
- Every decoder is bounded and total — see [binary parsing](docs/invariants/binary-parsing.md).
- Anything that will reach simulation state follows [determinism](docs/invariants/determinism.md) from
  the start, because it cannot be retrofitted.
- **A rendering change is not verified by a green test suite. Look at the capture.** Every rendering bug
  so far passed its own assertions and was caught by opening the PNG: reversed layer ramps, two separate
  tone-mapping mistakes, a shadow camera on the wrong side of the scene, an occlusion blur whose
  tolerance rejected every neighbour at distance, twice a test fixture measuring itself rather than the
  renderer, a quad UV mapping that walked the unit square in the wrong order, a wave sum that interfered
  into a diamond lattice, a specular exponent so tight the highlight reached no pixel at all, water
  painted as a slab past the edge of the map, a cloud hash correlated along one axis, and a derived sun
  that matched its preset's colour exactly while sitting 27 degrees away in azimuth.
  - **And one the harness could not have caught, which is worth its own line.** Three resolve passes offset
    their texture coordinate by half a pixel, and every reference was rendered through the offset — so the
    images agreed with each other and with the code. A reference comparison cannot catch an error applied
    uniformly to both sides of it. What caught it was a *property*: a temporal accumulation of a static
    frame must reach a fixed point, and it did not. The lesson is not that the harness is weak but that it
    has a blind spot with a shape, and the things that see into it are invariants rather than images. The regression
  harness now catches this class automatically, and in CI rather than only locally — but only for the
  eleven scenes it has references for, and only once someone has looked at those references and confirmed
  they are right.
- **A fixture can be the bug.** Repeatedly now a correct implementation has been measured against a fixture
  that could not show what was being measured: a shadow fixture whose ridge was wider than its own shadow, a
  fog fixture so flat and so distant that an integral along the ray smoothed away everything the density did
  — four rounds of tuning went into that one before the fixture was suspected — a page fixture whose single
  ramped layer normalized to a constant, a flat-coloured page that could not distinguish a linear-light
  average from a byte-space one, and an aliasing metric taken across the one axis its fixture's stripes did
  not vary along. **The pattern is worth naming: a fixture that cannot fail is indistinguishable from a
  fixture that passes.** The cheap defence is to make the *wrong* implementation's prediction part of the
  assertion, which is what the linear-light test does.
- **Presentation needs running, not just testing.** The one bug the headless suite structurally could not
  catch — surface capabilities queried through an adapter from the wrong instance — appeared the first
  time the window opened.
