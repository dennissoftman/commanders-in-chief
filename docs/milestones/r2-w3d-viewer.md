# R2: W3D inspection and viewer

**Status:** Complete.

**Scope:** Bounded recursive chunk inventory followed by separately gated static geometry,
materials, hierarchies, animation, and an animated viewer.

**Exclusions:** MAP terrain, gameplay simulation, general asset editing, and retail asset
distribution.

**Inputs:** Original synthetic W3D streams and user-owned W3D resources through the VFS.

**Outputs:** Stable unknown-preserving chunk reports, immutable decoded asset values, portable glTF
sanity-check artifacts, deterministic renderer captures, and an interactive animated viewer.

**Owner:** `cic-formats` for decoding, `cic-render` for staging/presentation/resources, and
`cic-tools` for VFS-backed inspection and launch commands.

**Acceptance tests:** Exact nested boundary closure, truncation and depth/count/size limits,
unknown payload preservation, semantic count/index checks, split-resource BIG-backed CLI
integration, retail smoke verification, and external importer validation.

**Determinism:** File-order chunk trees, slash-separated numeric paths, stable identifier
names, and no renderer or host-order dependency in reports.

**Documentation:** `docs/formats/w3d.md`, provenance, compatibility matrix, and renderer-boundary
ADR 0004.

**Completion artifact:** Original nested and composed textured/animated fixtures, stable chunk and
exact-bit geometry reports, a Blender-importable synthetic GLB, headless renderer hashes/captures,
and installed-resource window smokes that retain no retail content.

**Progress:** The recursive inventory, 73-name identifier table, original nested fixture,
and `cic-inspect w3d` report are complete. A 113,980-byte installed W3D closes exactly into
525 records. Header3 versions 3.0 through 4.2, vertices, normals, and triangles now decode
into immutable renderer-neutral values with exact count/size and vertex-index validation.
The BIG-backed `cic-inspect w3d-mesh` report is deterministic. Materials, shaders, textures,
UVs, hierarchy/HLOD composition, rigid and skinned models, and classic raw-animation clips
decode into immutable bounded values. `cic-inspect w3d-export` composes split retail W3Ds,
resolves Generals or layered Zero Hour resources, converts TGA/DDS images to sRGB PNG, and
emits a Blender-importable self-contained GLB by default or external glTF with `--gltf`.
The format surface is complete for renderer ingestion: time-coded and adaptive-delta compressed
animation decode under bounded expansion and use the existing glTF clip path. All fixed-function
passes, stages, mapper data, animated-texture descriptors, and shader bytes are retained in
versioned GLB/glTF metadata; the visible core-glTF approximation remains explicitly pass
zero/stage zero. Installed compressed infantry and two-pass building exports passed local
verification without retaining retail data. Additive `ONE + ONE` light materials keep their
unchanged source RGBA images and use separate alpha-coverage preview images; installed airstrip
lights verified that black sprite backgrounds no longer become opaque rectangles.

### R2 renderer ingestion and animated viewer gate (complete)

**Scope:** Introduce a renderer crate that consumes immutable `cic-formats` model values, renders
the selected HLOD with hierarchy/skinning/animation, and begins fixed-function pass/stage
equivalence behind an explicit preview policy.

**Exclusions:** MAP terrain, gameplay/simulation ownership, asset editing, retail fixtures, and
claims of complete fixed-function equivalence before image comparisons exist.

**Inputs:** Existing original composed W3D fixtures and user-owned installed resources through the
VFS; no renderer-side parsing.

**Outputs:** An interactive animated viewer plus deterministic diagnostic captures from synthetic
fixtures.

**Owner:** A new renderer crate depending on `cic-formats`/`cic-vfs`; simulation and core remain
renderer-independent.

**Acceptance tests:** Headless synthetic-frame checks, hierarchy/skin pose comparisons, stable
material-pass command ordering, malformed-resource rejection before renderer ingestion, and a local
installed-resource smoke capture.

**Determinism:** Stable mesh/pass/stage submission order, explicit animation time input, no host
filesystem order, and no wall-clock values in diagnostic artifacts.

**Documentation:** A renderer-boundary ADR, compatibility updates, and capture instructions that do
not distribute retail content.

**Completion artifact:** Checked-in synthetic screenshot/capture hashes plus a locally verified
animated installed-model capture report.

**Progress:** ADR 0004 selects `wgpu` 30 with native Vulkan, Metal, and Direct3D 12. The new
`cic-render` crate stages validated W3D geometry in stable file order and completed a surface-free
64x64 RGBA8 triangle capture at an explicit pose. The checked-in SHA-256 matched a local RTX 4080
SUPER run. `cic-inspect w3d-render` now composes models from synthetic or installed BIG overlays,
applies rigid/one-bone bind transforms, and emits a depth-tested geometry capture. An installed
building smoke capture succeeded. `cic-inspect w3d-view` now presents a 960x720 auto-fitted,
45-degree elevated, Z-up rotating model, samples raw or compressed hierarchy animation at explicit
integer frames, and switches clips with Left/Right. Installed building and 39-clip infantry window
smokes passed, including bounded legacy helper-bone hiding. Clip framing is now fixed at selection
time rather than recomputed per animation tick. Pass-zero/stage-zero materials resolve textures
through the VFS, expand per-face UV seams, preserve source alpha, select opaque/alpha/additive GPU
pipelines, and reuse content-deduplicated texture images and material bind groups. An installed
airstrip initially used 15 effective materials and 13 unique textures without black sprite
backgrounds; the 39-clip infantry used four materials and four textures. The completed renderer
expands all passes/stages in stable order, uses a documented multiply policy for later stages,
samples temporal mapper arguments from explicit seconds, and exposes the same path to headless
capture. A synthetic two-pass/two-stage capture at animation frame 1 and mapper time 0.5 seconds
matches checked RGBA SHA-256
`b1f43b981348e99b89c5dcd15b64279cb1b9990df3996ae4b35e4939d8301672`. Final installed captures
rendered the airstrip as 27 draws/17 materials/14 textures and infantry frame 1 as four
draws/materials/textures without retaining retail content. Exact legacy fixed-function equivalence
remains explicitly excluded until broader image comparisons exist.

## Completion evidence

- Bounded W3D chunk-tree inventory with opaque unknown payload preservation, stable
  slash-separated tree paths, and 73 known chunk names.
- Deterministic `cic-inspect w3d` reports through loose-directory or BIG mounts.
- Original nested W3D fixture and synthetic BIG-to-W3D CLI completion artifact.
- A 113,980-byte installed Steam Generals W3D parses exactly into 525 stable inventory
  records; 12 sampled W3Ds use the documented container flag.
- Header3 versions 3.0 through 4.2, vertices, normals, and triangles decode into immutable,
  renderer-neutral values with explicit 4,000,000-record limits.
- Static meshes require exact count-sized payloads, mandatory static channels, unique data
  chunks, and in-range triangle indices.
- The original three-vertex/one-triangle fixture and BIG-backed `cic-inspect w3d-mesh`
  completion artifact pass; reports preserve floating-point values as exact bits.
- One installed version 4.2 mesh verified at 24 vertices, 24 normals, and 12 triangles.
- Material inventories, 32-byte vertex materials, singleton/per-vertex first-pass IDs, and
  explicit DCG arrays decode into immutable values with count, size, name, scalar, and index
  validation.
- First-pass DCG colors override vertex-material diffuse colors and are emitted as normalized
  glTF vertex colors.
- Original colored-triangle and synthetic BIG-backed completion artifacts pass. Two
  installed static meshes decode their material inventories and assignments directly from
  the user-owned W3D archive.
- Fixed 16-byte shader records, bounded texture names and 12-byte texture info records,
  singleton/per-triangle shader and texture IDs, finite UV arrays, and optional checked
  per-face UV indices decode into immutable renderer-neutral values.
- Bounded hierarchy, pivot, last/highest-detail HLOD, one-bone skin influence, and classic
  raw translation/quaternion animation decoding produce immutable model values.
- Model composition spans sibling skin, hierarchy, and animation W3Ds through the VFS;
  collision boxes referenced by HLOD are recognized and excluded from render meshes.
- `cic-inspect w3d-export` emits a self-contained GLB by default or, with `--gltf`, glTF
  2.0 JSON, an external binary buffer, and PNG images. Both forms include hierarchy nodes,
  rigid and skinned meshes, animation clips, and first-pass materials. The resource basename
  determines the default output name, with an optional explicit output-path override.
- Source PNGs preserve decoded RGBA texels, carry explicit sRGB metadata, and remain
  straight-alpha images; the GLB form embeds them as image buffer views. W3D `ONE + ONE`
  materials additionally receive a separate derived alpha-coverage image for the visible
  core-glTF preview because core glTF has no additive framebuffer blend equation.
- Generals is the default installed-resource profile. `--zh` deterministically enumerates and
  mounts Generals providers first, Zero Hour providers second, and explicit mods last. Opaque
  replacement resources use the last-mounted winner; cumulative definition formats parse full VFS
  history earliest-to-latest so partial Zero Hour files retain inherited Generals definitions.
  `--game-dir`, saved configuration, environment roots, and validated Steam discovery avoid
  repeated archive arguments.
- The synthetic completion artifact splits model, hierarchy, animation, and texture data
  across W3Ds and two BIGs. Retail Generals and Zero Hour exports succeeded; Blender 3.3
  imported a self-contained GLB with a 32-joint Zero Hour skin and 23 animation actions.
- Installed split-infantry animation exposed legacy offscreen helper-bone visibility values;
  glTF preview now maps those model-scale outliers to bounded nonsingular attachment states while
  preserving ordinary motion and every decoded clip.
- W3D skin vertices are exported in their decoded bone-local space with identity glTF inverse binds;
  installed infantry bind poses and animation no longer separate into local-origin body parts.
- Time-coded and adaptive-delta compressed animations decode to immutable per-frame channels under
  explicit frame, channel, time-code, packet, and 64,000,000-value expansion limits.
- Synthetic GLB integration exports raw and compressed sibling clips together. An installed
  infantry export produced 20 actions, including one verified time-coded compressed clip.
- Vertex-material mapper modes and bounded argument strings, `DIG`/`SCG` pass colors, and validated
  animated-texture descriptors decode into renderer-neutral values.
- `fixed-function-metadata-v1` GLB/glTF mesh extras retain every pass, stage, assignment, shader
  byte, mapper, texture descriptor, color array, and exact float bit pattern while the visible core
  glTF approximation remains explicitly pass zero/stage zero.
- An installed building export verified two-pass metadata, two textures, and a non-UV environment
  mapper on two meshes; every table texture was packaged without retaining retail data.
- The installed `abarfrccmd.w3d` airstrip lights verified opaque source DDS alpha plus `ONE + ONE`
  shader blending. Their preserved source PNGs remain byte-equivalent after decode, while derived
  preview images make black texels transparent and route only the visible glTF materials to them.
- `cic-render` now depends downward on immutable `cic-formats` values and stages validated W3D
  positions, normals, and triangle indices in stable file order without parser, VFS, filesystem,
  clock, window, or simulation ownership.
- The selected `wgpu` 30 backend enables native Vulkan, Metal, and Direct3D 12 plus WGSL. A
  surface-free RGBA8 path renders an explicitly posed synthetic triangle, strips copy-row padding,
  and returns bounded readback bytes with a SHA-256 diagnostic.
- A local RTX 4080 SUPER headless capture matched the checked-in 64x64 translated-triangle hash
  `7e1894e3ad60f3236f628efdef3e61f3d724e351a37bab9612273190fa8c1ee0`.
- `cic-inspect w3d-render` now uses the same installed profile or explicit BIG overlay path as W3D
  inspection, resolves textures, and accepts explicit animation index/frame, mapper seconds, and
  Z-up rotation before emitting a 512x512 PPM plus stable resource/draw/hash diagnostics.
- `cic-inspect w3d-view` uses the same installed profile or explicit BIG overlay path, opens a
  960x720 surface, auto-fits a fixed 45-degree elevated camera, and continuously rotates the model
  around W3D's Z-up axis. It samples hierarchy/one-bone clips at explicit integer frames; Left/Right
  switch clips, Escape closes, and the active name is visible in the title.
- Installed window smokes verified the complete `abarfrccmd.w3d` building remains framed throughout
  rotation and `aihero_skn.w3d` visibly animates across 39 switchable clips. The established bounded
  hidden-helper policy prevents legacy offscreen attachment translations from collapsing framing.
- Viewer center and scale are now computed only when a clip is selected. Animation ticks preserve
  that fixed framing and apply only pose plus Z-up rotation, removing bounds-driven bobbing and
  scale jitter.
- The initial pass-zero/stage-zero material gate expanded per-face UV seams and rendered
  VFS-resolved sRGB textures with source alpha, alpha testing, depth policy, and
  opaque/source-alpha/additive blending.
- The bounded texture resource manager reuses VFS decodes by resolved path, normalizes W3D aliases,
  deduplicates retained images by dimensions and RGBA SHA-256, and reuses effective GPU materials
  across stable file-order draw ranges.
- Installed visual smokes rendered the airstrip with 15 effective materials and 13 unique textures,
  including black-background-free additive lights, and textured the 39-clip infantry with four
  materials and four textures.
- Renderer staging now expands every decoded pass and stage in stable mesh/pass/stage/triangle
  order. Stage zero uses the pass shader's opaque/alpha/additive preview blend; later stages use an
  explicit multiplicative preview and only the first opaque pass/stage writes depth.
- Linear, scale, grid, rotate, sine, step, zigzag, deterministic-random, edge, and bump-linear
  mapper inputs are sampled from explicit seconds. The renderer owns no clock; the windowed viewer
  supplies presentation time and headless callers provide a deterministic value.
- The composed synthetic two-BIG fixture now captures two passes, two stages, animation frame 1,
  and a scrolling mapper at 0.5 seconds. Its checked RGBA SHA-256 is
  `b1f43b981348e99b89c5dcd15b64279cb1b9990df3996ae4b35e4939d8301672`.
- Final installed RTX 4080 SUPER smokes rendered the airstrip as 27 stable draws/17 materials/14
  textures with RGBA SHA-256 `6766e83c92df9746df08810a5ab074a51dd77ac9c2780c317046c580d8196c51`,
  and infantry animation frame 1 as four draws/materials/textures with SHA-256
  `a4634a811ba4b8af88ef33a8246d0ca99a70f2ba75c144b7a103d8dd339ac88f`. Captures remain local.

## Known limitations

- Core glTF cannot reproduce exact W3D fixed-function multi-pass blending or animated mapper
  behavior; complete decoded metadata remains available to the project renderer.
- Adaptive-delta animation is synthetic-verified but has not yet been observed in an installed
  export.
- Exact legacy fixed-function equations and spatial environment/screen coordinate generation remain
  compatibility research beyond R2's documented preview policy.
