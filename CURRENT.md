# Current objective

## Where the project is

M0 through M2 are complete: the workspace and its invariants, the resource layer, and the native asset
formats. **M3's charter is complete** — the renderer draws a lit, shadowed, occluded, textured scene with
water, both headlessly and in a window, and a visual regression harness compares captures against
committed references.

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
  detail scale and blend a per-layer roughness by the same weights; model materials take their base
  colour from the images their glTF carried. Mip chains are generated on the CPU in linear light.
- **Water**, as a bounded plane with five summed directional waves, blended inside the HDR target so its
  glitter tone maps with the scene. Its shoreline is not authored: the pass discards wherever the bed
  rises through the displaced surface, so a rectangle plus a heightfield give an irregular shore that
  moves with the swell. Depth drives the tint and the edge opacity; Fresnel mixes in a reflection of the
  same sky the lighting pass paints.
- **Scene time is a frame parameter** — `DeferredFrame::time` — and nothing in the renderer reads a
  clock. That is what makes a capture of moving water reproducible.
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
it), and the full test suite all pass on the pinned toolchain. **202 tests across five crates**, 29 of
which render on a real device (verified on an NVIDIA RTX 4080 SUPER) and write their captures to
`target/tmp/`.

The render tests skip rather than fail when no adapter is available, so a machine or CI runner without a
GPU or software rasteriser reports honestly instead of red. The regression comparison itself is a pure
function over bytes with its own unit tests, so that half is verified even with no GPU present.

## Standing constraints

- Nothing in this tree derives from another game's code or reads another game's data. See
  [LICENSING.md](LICENSING.md). Water was the last piece with a live provenance constraint on it and was
  written from scratch; the removed shader was not consulted.
- Every decoder is bounded and total — see [binary parsing](docs/invariants/binary-parsing.md).
- Anything that will reach simulation state follows [determinism](docs/invariants/determinism.md) from
  the start, because it cannot be retrofitted.
- **A rendering change is not verified by a green test suite. Look at the capture.** Every rendering bug
  so far passed its own assertions and was caught by opening the PNG: reversed layer ramps, two separate
  tone-mapping mistakes, a shadow camera on the wrong side of the scene, an occlusion blur whose
  tolerance rejected every neighbour at distance, twice a test fixture measuring itself rather than
  the renderer, a quad UV mapping that walked the unit square in the wrong order, a wave sum that
  interfered into a visible diamond lattice, a specular exponent so tight the highlight reached no pixel
  at all, and water painted as a slab hanging past the edge of the map. The regression harness now
  catches this class automatically — but only for the five scenes it has references for, and only once
  someone has looked at those references and confirmed they are right.
- **Presentation needs running, not just testing.** The one bug the headless suite structurally could not
  catch — surface capabilities queried through an adapter from the wrong instance — appeared the first
  time the window opened.
