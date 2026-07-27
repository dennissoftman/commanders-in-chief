# Current objective

## Where the project is

M0 through M2 are complete: the workspace and its invariants, the resource layer, and the native asset
formats. M3 is active, and its first vertical slice has landed — a terrain package now renders.

What works: a `cic-assets` terrain uploads to the GPU and draws, headlessly, with directional lighting
and blended texture layers. Heights and layer weights live in *writable* textures with displacement and
normals computed in the vertex shader, so terrain deformation and route grading are texture writes
rather than a remesh. Captures resolve to PNG and are asserted on luminance spread and layer presence,
not just on being non-empty.

## Next verified step

Wire the deferred chain: G-buffer, cascaded shadows, and ambient occlusion. The shaders for all three
are already present and validated; what they need is the pipeline scaffolding and the bind group
layouts to match. Shadows and AO are also what the current forward pass most visibly lacks — the
terrain reads as correctly shaped but flatly lit, because nothing occludes anything yet.

After that: the model pipeline against imported glTF, then water, then windowed presentation.

## Gate status

Formatting, strict lints (`clippy::all` and `clippy::pedantic` as errors), and the full test suite all
pass on the pinned toolchain. **121 tests across five crates**, including five that render on a real
device.

The render tests skip rather than fail when no adapter is available, so a machine or CI runner without
a GPU or software rasteriser reports honestly instead of red.

## Standing constraints

- Nothing in this tree derives from another game's code or reads another game's data. See
  [LICENSING.md](LICENSING.md).
- Every decoder is bounded and total — see [binary parsing](docs/invariants/binary-parsing.md).
- Anything that will reach simulation state follows [determinism](docs/invariants/determinism.md) from
  the start, because it cannot be retrofitted.
- **A rendering change is not verified by a green test suite. Look at the capture.** The terrain work
  produced three bugs that all passed their assertions before the PNG was opened: reversed layer ramps
  that made one layer invisible, a tone-map curve that crushed all shading contrast, and a test terrain
  so flat that its "many distinct colours" assertion was measuring the fixture rather than the
  renderer.
