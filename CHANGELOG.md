# Changelog

All notable user-visible changes are recorded here.

## Unreleased

### Fixed

- Linux and macOS builds no longer retain the Windows-only Steam registry command import.
- Angled terrain views now select virtual-texture detail in camera-space depth and rank projected
  page bounds instead of filling a world-axis square around the viewport footprint. Coarse visible
  coverage is retained before fine upgrades, removing the misplaced rectangular LOD island.

### Added

- Bounded declarative mount profiles and repeatable ordered `--mod` layers for custom bases and
  total conversions, plus lazy directory/BIG providers that index on mount and read only requested
  resources under caller-selected limits.
- Bounded water-only MAP decoding/reporting, stable lake/river staging, a modern hybrid-deferred
  terrain viewer with thickness-aware forward water, and deterministic Modern-profile de-tiling.
- Horizon-safe terrain detail streaming with a persistent 128-page GPU-composed virtual-texture
  cache over the stable 8-pixel background. Bordered 16/32-pixel pages preserve authored layers,
  cliff UVs, custom edges, and Modern macro variation; stable page tables, LRU reuse,
  GPU-generated linear mipmaps, and anisotropic sampling remove runtime CPU terrain rebakes.
  Water now uses bounded
  source-resolved caustic animation, source transparency depth,
  a more opaque body, and restored shallow shoreline haze and crest effects.
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
- Added stable rendering for every decoded W3D pass and texture stage. Later stages use an explicit
  multiplicative preview while each pass retains its decoded opaque, alpha, or additive blend.
- Added explicit-time CPU sampling for temporal UV mapper arguments, including scrolling, atlas,
  rotation, sine, step, zigzag, deterministic-random, edge, and bump-linear inputs.
- Extended `cic-inspect w3d-render` to resolve deduplicated textures and capture a selected animation
  frame, mapper time, and rotation without reading a clock. The synthetic two-pass/two-stage
  textured animation capture has a checked RGBA SHA-256 completion hash.
- Synthetic unit and integration tests plus CI quality gates.
- Added bounded bare and `EAR\0` RefPack-wrapped `CkMp` MAP symbol-table and top-level chunk
  inventories with opaque unknown payload preservation, deterministic last-symbol-wins name
  resolution, and stable VFS-backed reports.
- Added `HeightMapData` versions 1 through 4 with explicit dimension, border, boundary, payload,
  allocation, and sample-cardinality checks plus stable row-major `cic-inspect map-height` output.
- Added deterministic 8-bit grayscale PNG export to `cic-inspect map-height --png` with exact
  stored sample order and no color-space transform.
- Added bounded immutable `BlendTileData` version-6/7 tile planes, version-6 source-equivalent
  height-derived cliff flags, version-7 legacy cliff-bitmap normalization, terrain and edge texture
  classes, blend records, and cliff UV records, plus a stable VFS-backed `cic-inspect map-blend`
  report.
- Added an original versioned MAP fixture, negative parser tests, a synthetic BIG-backed completion
  artifact, and a bounded MAP fuzz target.
- `cic-inspect map-height` now writes a basename-derived grayscale PNG by default; `--report`
  selects the stable text report and `--png` supplies an explicit output path.
- Added a bounded Terrain INI declaration decoder and deterministic `DefaultTerrain` inheritance so
  semantic MAP texture classes resolve through mounted `Terrain.big`/`INI.big` resources.
- Added source-scaled terrain geometry and deterministic base/primary/extra texture staging with
  packed tile quadrants, source-rounded mip reduction, procedural blend masks, and source-selected
  triangle diagonals.
- Added `cic-inspect map-render`, which produces a depth-tested isometric sRGB PNG and stable
  geometry/layer/hash diagnostics through the headless GPU renderer. An original layered-terrain
  fixture carries a checked RGBA SHA-256 completion hash.
- Added `cic-inspect map-view`, a perspective terrain flyover sharing the map-render resource and
  staging path, with WASD/vertical flight, speed boost, right-mouse look, wheel dolly, and camera
  reset controls.
- Added explicit `legacy` and `modern` terrain policies. Both apply same-class stored cliff UVs;
  the default legacy policy also reproduces bounded steep-slope UV retile and height-selected
  triangle diagonals.
- Added separately indexed custom-edge geometry and deterministic quarter-atlas texturing for
  white material coverage, black gaps, and colored decorative edge pixels in both headless and
  interactive terrain rendering.
- Added bounded renderer detail streaming: quantized, depth-capped screen-space footprints rebake
  authored terrain as independent 16- and 32-pixel tiers over the unchanged deterministic 8-pixel
  background. Generation checks immediately cancel obsolete work and suppress stale uploads;
  explicit-time overlap transitions retain the previous resident patch during replacement.
- Added a bounded `WaterTransparency` INI decoder and renderer-neutral `WaterAppearance` input.
  Installed profiles may resolve complete `caust00`-`caust31` image sequences into a mipmapped GPU
  texture array; synthetic mounts remain valid without retail resources.
- Added terrain-surface directional shading to `map-view`. This explicit presentation preview
  improves slope readability without changing staged values or deterministic headless hashes;
  source-authored MAP lighting remains a later semantic decoder.
- Enabled back-face culling for terrain, custom edges, and streamed detail after verifying the
  stable height-field winding; deterministic terrain capture hashes remain unchanged.
