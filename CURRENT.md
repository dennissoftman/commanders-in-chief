# Current objective

## Where the project is

M0 through M2 are complete: the workspace and its invariants, the resource layer, and the native asset
formats. **M3's charter is complete bar one item** — the renderer draws a lit, shadowed, occluded, textured
scene with water and weather, both headlessly and in a window, and a visual regression harness compares
captures against committed references. The exception is **terrain level of detail**: chunks are now
frustum-culled per pass, but a chunk that is drawn is drawn at full density whatever its size on screen.
This document and the milestone both claimed the whole item was done until it was checked.

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
per-pass breakdown once a second, which is where the figures above came from.

## Next verified step

**A GPU-capable CI runner.** This is the last piece of M3's exit condition and the only one outstanding.
CI is `ubuntu-latest` with no adapter, so `GpuContext::new` finds none and every rendering test skips
there — true since the first render test landed, not something recent changed. Two steps, in order:

1. Install Mesa's `lavapipe` on the runner so an adapter exists.
2. On that runner, generate the reference set with `CIC_UPDATE_REFERENCES=1`, review the images, and
   commit them under their own adapter directory.

References cannot be copied from a developer machine: a software rasteriser and an NVIDIA card differ far
beyond the tolerance, which is why the sets are keyed by adapter in the first place.

After that, in rough order:

1. **Terrain level of detail**, the remaining half of the charter item now that culling has landed. A chunk
   that is visible still draws every cell it has, whether it covers forty pixels or four; the chunk
   decomposition culling introduced is what a per-chunk stride would hang off, plus skirts or stitching so
   neighbouring densities do not crack.
2. **TAA**, the quality tier ADR 0005 plans and the last antialiasing item: a jittered projection, a
   motion-vector target, a history buffer, and neighbourhood clamping. It needs the regression harness
   accounted for, since a temporal accumulator makes one captured frame depend on the frames before it.
3. **Normal and roughness maps** to go with the base-colour textures.
4. **M4's interface layer**, whose settings screen has real content waiting for it now that a display
   setting exists with more than one option.

## Gate status

Formatting, strict lints (`clippy::all` and `clippy::pedantic` as errors, plus `-D warnings` as CI runs
it), and the full test suite all pass on the pinned toolchain. **271 tests across five crates**, 36 of
which render on a real device (verified on an NVIDIA RTX 4080 SUPER) and write their captures to
`target/tmp/`. Eleven committed references cover terrain layers, instanced models, the deferred chain, water,
water under a glancing sun, cloud shadows, fog, wet ground, snow, an antialiased frame, and a supersampled
one. The nine that predate the antialiasing work are **byte-identical** across it, which is what shows a
growing uniform block, a rerouted composite, and a removed frame field changed no image on the default path.

The render tests skip rather than fail when no adapter is available, so a machine or CI runner without a
GPU or software rasteriser reports honestly instead of red. The regression comparison itself is a pure
function over bytes with its own unit tests, so that half is verified even with no GPU present.

## Standing constraints

- Nothing in this tree derives from another game's code or reads another game's data. See
  [LICENSING.md](LICENSING.md). Water was written from scratch and the removed shader was not consulted;
  **scenery sway is now the only outstanding provenance case.**
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
  that matched its preset's colour exactly while sitting 27 degrees away in azimuth. The regression
  harness now catches this class automatically — but only for the nine scenes it has references for, and
  only once someone has looked at those references and confirmed they are right.
- **A fixture can be the bug.** Twice now a correct implementation was tuned against a fixture that could
  not show what was being measured: a shadow fixture whose ridge was wider than its own shadow, and a fog
  fixture so flat and so distant that an integral along the ray smoothed away everything the density did.
  Four rounds of tuning went into the second before the fixture was suspected.
- **Presentation needs running, not just testing.** The one bug the headless suite structurally could not
  catch — surface capabilities queried through an adapter from the wrong instance — appeared the first
  time the window opened.
