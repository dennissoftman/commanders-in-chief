# Current Objective

## Objective

R2 W3D inspection and renderer ingestion are complete. Prepare the bounded R3 MAP terrain
inspection and viewer gate.

## Implemented foundation

- R0 completion suite passed in GitHub CI run `29840005186`.
- Rust workspace and CI policy.
- Bounded, cursor-based binary reads with structured errors.
- Normalized, ASCII case-insensitive virtual paths.
- Deterministic last-mounted-wins overlays with full provider history.
- Loose-directory manifest CLI and synthetic tests.
- Evidence-backed `BIGF`/`BIG4` indexing with explicit limits and synthetic fixture.
- BIG duplicate-name history with deterministic last-entry-wins resolution.
- Mixed directory/BIG manifests through `cic-inspect`.
- Evidence-backed CSF version 3 decoding with raw names, complemented UTF-16, optional
  wave names, zero-string labels, and all variants preserved.
- Deterministic `cic-inspect csf` reports through loose-directory or BIG mounts.
- Original CSF fixture and synthetic BIG-to-CSF CLI completion artifact.
- Bounded W3D chunk-tree inventory with opaque unknown payload preservation, stable
  slash-separated tree paths, and 73 known chunk names.
- Deterministic `cic-inspect w3d` reports through loose-directory or BIG mounts.
- Original nested W3D fixture and synthetic BIG-to-W3D CLI completion artifact.
- Local formatting, strict Clippy, and the complete workspace test suite pass.
- All 18 installed Steam Generals BIG archives have matching declared sizes and bounded
  verified directory trailers; `INI.big` resolves 92 deterministic manifest entries.
- The installed Steam Generals CSF parses exactly to its 282,246-byte member boundary and
  reports version 3, 2,806 labels, and 2,805 strings.
- A 113,980-byte installed Steam Generals W3D parses exactly into 525 stable inventory
  records; 12 sampled W3Ds use the documented container flag.
- The CSF AddressSanitizer/libFuzzer smoke gate completed 4,077,155 inputs in 31 seconds
  without a crash or sanitizer finding.
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
- Generals is the default installed-resource profile. `--zh` deterministically overlays
  Zero Hour on its Generals base; `--game-dir`, saved configuration, environment roots, and
  validated Steam discovery avoid repeated archive arguments.
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

## Known blockers

- `BIG4` remains implemented from corroborating source but unverified against retail data.
- Core glTF cannot reproduce exact W3D fixed-function multi-pass blending or animated mapper
  behavior; complete decoded metadata remains available to the project renderer.
- Adaptive-delta animation is synthetic-verified but has not yet been observed in an installed
  export.
- Exact legacy fixed-function equations and spatial environment/screen coordinate generation remain
  compatibility research beyond R2's documented preview policy.

## Next verified step

Start R3 with a bounded, unknown-preserving MAP chunk inventory and original synthetic fixture;
identify version headers and terrain-height records before adding renderer ingestion.
