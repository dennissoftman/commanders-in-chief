# Current objective

## Where the project is

M0 through M2 are complete: the workspace and its invariants, the resource layer, and the native asset
formats. **M3's charter is complete** — the renderer draws a lit, shadowed, occluded, textured scene with
water and weather, both headlessly and in a window, and a visual regression harness compares captures
against committed references.

What works:

- A `cic-assets` terrain uploads to the GPU and renders through a seven-pass deferred chain: four shadow
  cascades, a G-buffer, ambient occlusion with a bilateral blur, deferred lighting that reconstructs
  world position from depth, a blended water pass, and a tone-mapping composite.
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
  it — which is how a single shader reached 620 lines. Twelve programs are assembled from fifteen chunks;
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
- **Time of day drives the light by default.** `DeferredFrame::new` derives its sun from the environment,
  and `in_environment` re-derives it, so changing the hour moves the sun rather than leaving a light that
  silently disagrees with it. The derivation is calibrated against the hand-tuned preset it replaced and a
  test pins it there.
- **Scene time is a frame parameter** — `DeferredFrame::time` — and nothing in the renderer reads a
  clock. That is what makes a capture of moving water or drifting cloud reproducible.
- Windowed presentation, driven by the reusable camera:

```bash
cargo run -p cic-render --example terrain_viewer --release
```

Pass a `.cicmap` path to view a real map; with no argument it generates terrain, buildings, a water
table derived from the heightfield's own low point, and their surfaces, so the viewer runs before any
content exists.

## Next verified step

**A GPU-capable CI runner.** This is the last piece of M3's exit condition and the only one outstanding.
CI is `ubuntu-latest` with no adapter, so `GpuContext::new` finds none and every rendering test skips
there — true since the first render test landed, not something recent changed. Two steps, in order:

1. Install Mesa's `lavapipe` on the runner so an adapter exists.
2. On that runner, generate the reference set with `CIC_UPDATE_REFERENCES=1`, review the images, and
   commit them under their own adapter directory.

References cannot be copied from a developer machine: a software rasteriser and an NVIDIA card differ far
beyond the tolerance, which is why the sets are keyed by adapter in the first place.

After that, in rough order: antialiasing per
[ADR 0005](docs/adr/0005-antialiasing-strategy.md) — a resolution scale, FXAA, then TAA, and explicitly
not MSAA — then normal and roughness maps to go with the base-colour textures, then M4's interface layer.

## Gate status

Formatting, strict lints (`clippy::all` and `clippy::pedantic` as errors, plus `-D warnings` as CI runs
it), and the full test suite all pass on the pinned toolchain. **227 tests across five crates**, 31 of
which render on a real device (verified on an NVIDIA RTX 4080 SUPER) and write their captures to
`target/tmp/`. Nine committed references cover terrain layers, instanced models, the deferred chain, water,
water under a glancing sun, cloud shadows, fog, wet ground, and snow.

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
