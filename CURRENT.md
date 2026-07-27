# Current objective

## Where the project is

M0 through M2 are complete: the workspace and its invariants, the resource layer, and the native asset
formats. M3 is active and now draws a lit, shadowed, occluded terrain both headlessly and in a window.

What works:

- A `cic-assets` terrain uploads to the GPU and renders through a six-pass deferred chain: four shadow
  cascades, a G-buffer, ambient occlusion with a bilateral blur, deferred lighting that reconstructs
  world position from depth, and a tone-mapping composite.
- Heights and layer weights live in *writable* textures with displacement and normals computed in the
  vertex shader, so terrain deformation and route grading are texture writes rather than a remesh.
- Windowed presentation, driven by the reusable camera:

```bash
cargo run -p cic-render --example terrain_viewer --release
```

Pass a `.cicmap` path to view a real map; with no argument it generates terrain, so the viewer runs
before any content exists.

## Next verified step

The model pipeline: render imported glTF geometry with its PBR materials through the existing G-buffer,
instanced, with the shadow pass extended to cover models as well as terrain. That is what turns a
landscape into a scene, and it is the last piece M4's interface work needs underneath it.

After that, in rough order: albedo textures per terrain layer (currently flat palette colours), water,
multisampling, and the committed reference captures that close the milestone.

## Gate status

Formatting, strict lints (`clippy::all` and `clippy::pedantic` as errors), and the full test suite all
pass on the pinned toolchain. **154 tests across five crates**, twelve of which render on a real device.

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
  tolerance rejected every neighbour at distance, and twice a test fixture measuring itself rather than
  the renderer.
- **Presentation needs running, not just testing.** The one bug the headless suite structurally could not
  catch — surface capabilities queried through an adapter from the wrong instance — appeared the first
  time the window opened.
