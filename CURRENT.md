# Current objective

## Where the project is

M0 through M2 are complete: the workspace and its invariants, the resource layer, and the native asset
formats. M3 is active and now draws a lit, shadowed, occluded, *textured* scene both headlessly and in a
window.

What works:

- A `cic-assets` terrain uploads to the GPU and renders through a six-pass deferred chain: four shadow
  cascades, a G-buffer, ambient occlusion with a bilateral blur, deferred lighting that reconstructs
  world position from depth, and a tone-mapping composite.
- Heights and layer weights live in *writable* textures with displacement and normals computed in the
  vertex shader, so terrain deformation and route grading are texture writes rather than a remesh.
- Instanced models share the terrain G-buffer and every shadow cascade, one draw call per model
  whatever its material count, with a per-instance transform and colour tint.
- Both surfaces are textured through one mechanism: a colour texture array per drawing unit, indexed by
  a slice number the material carries. Terrain layers tile their albedo in *world* space at a per-layer
  detail scale and blend a per-layer roughness by the same weights; model materials take their base
  colour from the images their glTF carried. Mip chains are generated on the CPU in linear light.
- Windowed presentation, driven by the reusable camera:

```bash
cargo run -p cic-render --example terrain_viewer --release
```

Pass a `.cicmap` path to view a real map; with no argument it generates terrain, buildings, and their
surfaces, so the viewer runs before any content exists.

## Next verified step

Water surfaces. This is the one remaining piece of M3 with a provenance constraint on it: the previous
water shader's texture scale, tint, and depth-feather policy were derived, so it was held back and has
to be written from scratch without consulting the original. See [LICENSING.md](LICENSING.md).

After that, in rough order: multisampling, normal and roughness maps to go with the base-colour
textures, and the committed reference captures that close the milestone.

## Gate status

Formatting, strict lints (`clippy::all` and `clippy::pedantic` as errors), and the full test suite all
pass on the pinned toolchain. **181 tests across five crates**, 23 of which render on a real device
(verified on an NVIDIA RTX 4080 SUPER) and write their captures to `target/tmp/`.

The render tests skip rather than fail when no adapter is available, so a machine or CI runner without a
GPU or software rasteriser reports honestly instead of red.

## Standing constraints

- Nothing in this tree derives from another game's code or reads another game's data. See
  [LICENSING.md](LICENSING.md).
- Every decoder is bounded and total — see [binary parsing](docs/invariants/binary-parsing.md).
- Anything that will reach simulation state follows [determinism](docs/invariants/determinism.md) from
  the start, because it cannot be retrofitted.
- **A rendering change is not verified by a green test suite. Look at the capture.** Every rendering bug
  so far passed its own assertions and was caught by opening the PNG: reversed layer ramps, two separate
  tone-mapping mistakes, a shadow camera on the wrong side of the scene, an occlusion blur whose
  tolerance rejected every neighbour at distance, twice a test fixture measuring itself rather than
  the renderer, and a quad UV mapping that walked the unit square in the wrong order — sheared every
  textured face along a diagonal, and had been sitting in two fixtures unnoticed for as long as nothing
  sampled through it.
- **Presentation needs running, not just testing.** The one bug the headless suite structurally could not
  catch — surface capabilities queried through an adapter from the wrong instance — appeared the first
  time the window opened.
