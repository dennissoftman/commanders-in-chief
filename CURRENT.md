# Current objective

## Where the project is

M0 through M2 are complete: the workspace and its invariants, the resource layer, and the native asset
formats. M3 is active.

The seeded renderer holds the parts that carry no asset-format dependency — the validated WGSL shader
set, virtual-page residency bookkeeping, texture resources, and the camera. What remains in M3 is the
pipelines themselves: terrain meshing and the deferred pass, the model pipeline, shadow cascades and
ambient occlusion, water, and the frame loop with a headless capture path.

## Next verified step

Rebuild terrain rendering against `cic-assets`' terrain container: mesh a heightfield, upload
elevations as an `R16Unorm` texture, and drive the existing virtual-page residency logic from a real
camera view. The exit check is a headless capture of a map package's terrain, committed as the first
visual regression reference.

## Gate status

Formatting, strict lints (`clippy::all` and `clippy::pedantic` as errors), and the full test suite all
pass on the pinned toolchain. 108 tests across five crates.

## Standing constraints

- Nothing in this tree derives from another game's code or reads another game's data. See
  [LICENSING.md](LICENSING.md).
- Every decoder is bounded and total — see [binary parsing](docs/invariants/binary-parsing.md).
- Anything that will reach simulation state follows [determinism](docs/invariants/determinism.md) from
  the start, because it cannot be retrofitted.
- A rendering change is not verified by a green test suite. Look at the capture.
