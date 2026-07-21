# Changelog

All notable user-visible changes are recorded here.

## Unreleased

### Added

- Initial GPL-3.0-only repository charter and provenance policy.
- Rust workspace with bounded binary input and deterministic virtual filesystem crates.
- `cic-inspect manifest` command for deterministic loose-directory inventories.
- Bounded `BIGF`/`BIG4` archive indexing and mounting with stable entry provenance.
- Directory and BIG overlays in `cic-inspect manifest`.
- Bounded CSF localization decoding with complemented UTF-16, optional wave names,
  zero-string labels, and lossless raw names.
- `cic-inspect csf` deterministic localization reports through mounted directories and
  BIG archives.
- Bounded, unknown-preserving W3D chunk inventories with stable nested paths and known
  identifier names.
- `cic-inspect w3d` reports W3D chunk trees through mounted directories and BIG archives.
- Immutable W3D Header3 static geometry decoding with bounded vertex/triangle counts,
  exact record-size checks, static-channel validation, and range-checked triangle indices.
- `cic-inspect w3d-mesh` exact-bit geometry reports through mounted directories and BIG
  archives.
- Bounded W3D material inventories, vertex-material colors, first-pass material IDs, and
  explicit per-vertex diffuse color arrays.
- Bounded W3D fixed-function shader records, texture names/info, per-triangle shader and
  texture assignments, and texture-coordinate arrays.
- Bounded W3D hierarchy, highest-detail HLOD, rigid/skinned mesh composition, and classic
  raw-animation channel decoding, including split skeleton/skin/animation resources.
- `cic-inspect w3d-export` glTF 2.0 export with hierarchy transforms, skins, animation
  clips, first-pass PBR preview materials, UV conversion, and TGA/DDS-to-PNG image
  conversion. It emits one self-contained GLB by default and external glTF with `--gltf`,
  inferring the output name from the W3D resource unless an override is supplied.
- Base-color PNG output preserves decoded RGBA texels and declares the sRGB transfer
  function without applying an additional gamma transform or premultiplying alpha.
- Generals and Zero Hour resource profiles with `--zh`, one-off `--game-dir`, persisted
  installation roots, Steam library discovery, and deterministic base-then-expansion VFS
  layering.
- Missing referenced retail textures produce warned magenta placeholders so geometry and
  animation remain inspectable.
- glTF animation preview maps legacy offscreen attachment-bone hiding to bounded nonsingular
  near-zero-scale states, preventing carried props from expanding animated scene bounds by orders
  of magnitude or producing invalid joint rotations in glTF viewers.
- glTF skinned meshes are scene roots, and alpha cutoff is limited to masked materials, eliminating
  the corresponding Khronos validator findings.
- W3D bone-local skin vertices now use identity glTF inverse binds, fixing separated body parts and
  exploded animated infantry poses.
- Time-coded and adaptive-delta W3D animations now decode under explicit expansion limits and
  export through the same glTF animation path as classic raw clips.
- Vertex-material mapper modes and bounded argument strings, per-pass diffuse illumination and
  specular colors, and validated animated-texture metadata are retained as immutable values.
- GLB/glTF mesh extras preserve every fixed-function pass, texture stage, assignment, shader byte,
  mapper, animated-texture descriptor, and exact UV/scalar bits. All referenced base textures are
  embedded, while the visible metallic-roughness preview remains explicitly pass 0/stage 0.
- W3D `ONE + ONE` additive materials use separate alpha-coverage PNGs in the core-glTF preview,
  eliminating black sprite rectangles while retaining untouched decoded source RGBA images for
  fixed-function metadata consumers.
- Added the `cic-render` boundary with stable W3D geometry staging, a `wgpu` 30 Vulkan/Metal/DX12
  backend, explicit pose inputs, and bounded surface-free RGBA8 capture/readback.
- Added a synthetic translated-triangle capture example and checked-in SHA-256 completion hash.
- Added `cic-inspect w3d-render` for installed profiles or explicit BIG overlays. It composes the
  selected HLOD and hierarchy, stages rigid/one-bone bind geometry, and writes a depth-tested PPM
  plus adapter, geometry-count, and RGBA-hash diagnostics.
- Added `cic-inspect w3d-view` with a 960x720 presentation surface, automatic orthographic fit,
  45-degree elevated camera, continuous Z-up rotation, explicit-frame hierarchy/one-bone animation
  sampling, and Left/Right clip selection. The viewer applies the established bounded hidden-helper
  policy so legacy offscreen attachment sentinels cannot collapse animated model framing.
- Viewer framing is now computed once when a clip is selected; individual animation ticks preserve
  that fixed center and scale, removing per-frame alignment bobbing while Z-up rotation remains
  continuous.
- Added pass-zero/stage-zero W3D material rendering with expanded per-face UVs, source-alpha
  sampling, alpha testing, and opaque, source-alpha, or `ONE + ONE` additive GPU pipelines.
- Added a bounded texture resource manager with stable aliases, SHA-256 RGBA-content deduplication,
  resolved-VFS decode reuse, and effective GPU-material reuse across mesh draw ranges.
- Synthetic unit and integration tests plus CI quality gates.
