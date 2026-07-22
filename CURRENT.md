# Current Objective

## Objective

The R3 terrain presentation gate now includes a bounded water-only MAP boundary, stable lake and
river staging, a modern hybrid-deferred viewer with depth-aware forward water, bounded source
caustic animation and transparency inputs, and deterministic Modern-profile macro variation. The
terrain viewer has horizon-safe, predictive nested viewport residency with independently
cancellable 16/32-texel detail bakes, complete mip chains, and anisotropic sampling. Keep scripts
and object loading deferred while presentation work moves to bounded source lighting inputs and
modern reflection/shadow quality.

## Implemented foundation

- R0 completion suite passed in GitHub CI run `29840005186`.
- Rust workspace and CI policy.
- Bounded, cursor-based binary reads with structured errors.
- Normalized, ASCII case-insensitive virtual paths.
- Deterministic last-mounted-wins overlays with full provider history.
- Disk-backed directory mounts retain file metadata and BIG mounts retain only bounded directory
  indices; winning payloads are read lazily under parser-selected allocation limits.
- Bounded declarative mount profiles support arbitrarily named custom bases, optional providers,
  total conversions, and repeatable ordered mod layers without retail archive sentinels.
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
- Source-established bare and `EAR\0` RefPack-wrapped `CkMp` MAP streams decode under explicit
  compressed/decompressed size, symbol, name, chunk, payload, and back-reference limits.
- MAP inventories preserve symbol and top-level chunk file order, apply deterministic
  last-table-entry-wins name resolution, and retain every known or unknown payload opaquely because
  the format has no generic child-container marker.
- `HeightMapData` versions 1 through 4 decode into immutable stored dimensions, border, boundaries,
  cell spacing, and exact row-major byte samples under explicit dimension/allocation limits.
- `cic-inspect map` and `map-height --report` produce stable VFS-backed reports; `map-height`
  writes exact row-major samples as a deterministic 8-bit grayscale PNG by default. The original
  synthetic MAP and BIG completion artifact pass, including version dispatch and negative tests.
- `BlendTileData` versions 6 and 7 decode into bounded immutable signed tile-index planes, terrain
  and edge texture classes, blend selector records, and finite cliff UV records. Version 6 derives
  cliff flags from neighboring heights using the source threshold; version 7 normalizes its stored
  short-stride bitmap. `cic-inspect map-blend` preserves stable source order and exact UV bits.
- One user-owned installed RefPack MAP closed at 1,781,076 decompressed bytes as 46 symbols and 8
  chunks; its version-4 380-by-400 height field validated 152,000 samples. Its version-7 blend
  payload validated the same 152,000 cells, 204 bitmap tiles, 7,772 blend table entries, 14 terrain
  texture classes, and one cliff-info table entry. A 380-by-400 PNG smoke was verified and removed;
  only aggregate counts were retained.
- `map-height` now writes an 8-bit grayscale PNG by default, deriving the filename from the MAP
  resource basename; `--report` retains the stable text report and `--png` overrides the path.
- A bounded Terrain INI decoder preserves ordered declarations. `map-render` applies explicit
  `DefaultTerrain` inheritance/override semantics and resolves semantic MAP texture classes through
  the VFS-backed `Art/Terrain` sheets from the Terrain and INI archive profiles.
- `cic-render` stages source-scaled height geometry, per-cell base/primary/extra layers, blend- and
  cliff-selected triangle diagonals, packed 64-pixel tile quadrants, source-rounded mip reduction,
  and deterministic procedural alpha masks without owning parser, VFS, filesystem, or clock state.
- `cic-inspect map-render` bakes the stable terrain layers and produces an sRGB PNG through the
  headless GPU boundary. The legacy-UV layered and custom-edge synthetic captures match RGBA
  SHA-256 values `d19dee6e96471515ab0b4902e99aa9bed44650b10f975e35a91c427e95f96cad`
  and `5f5761f44446d8784b7c0910adee7ede440c9e428a3d4b25be26ce470bfabd27`.
- `map-view` shares the staged base/edge GPU path and provides perspective WASD/vertical flight,
  boost, right-mouse look, wheel dolly, and reset controls. The installed Generals viewer remained
  live through resource staging, GPU upload, surface creation, and camera rendering.
- `map-view` retains the immutable terrain inputs and derives nested conservative detail regions
  from projected screen demand rather than one complete ground-footprint rectangle. The stable
  background remains 8 texels per cell; independently resident 16-texel mid-field and
  source-established 32-texel foreground tiers stop at the depth where the next coarser tier has
  sufficient screen density. Quantized margins cover upcoming movement under the existing
  4096-pixel/64-MiB per-texture limits.
- Viewer-only derivative normals and an explicit directional preview light improve slope
  readability without altering staged terrain or headless completion hashes. The installed
  Generals window remained live with the detail and lighting path active; no capture was retained.
- Terrain, custom-edge, and nested-detail pipelines cull clockwise back faces from the established
  counter-clockwise height-field winding. Synthetic headless capture hashes remain unchanged;
  water remains a separately ordered material rather than inheriting terrain culling policy.
- `PolygonTriggers` versions 2 and 3 now have a bounded water-only decoder that retains stable
  water/river flags, identifiers, names, seam indices, and integer points while skipping general
  trigger semantics and allocations for non-water points. Degenerate markers are preserved and
  safely produce no renderer geometry.
- `map-water` emits a stable water-only report. Synthetic tests cover every truncated prefix,
  explicit trigger/name/point limits, established version dispatch, degenerate markers, and stable
  lake-fan/river-strip triangulation.
- `map-view` now writes opaque terrain, custom edges, and near detail into albedo,
  normal/roughness, world-position, and depth targets, resolves directional lighting into linear
  `RGBA16F`, tone maps to the surface, then renders water in a depth-tested/no-depth-write forward
  pass. The original project shader applies thickness absorption, refraction, Fresnel sky response,
  specular, and shallow foam; no legacy water-rendering algorithm was translated.
- `Modern` terrain policy applies deterministic world-anchored integer macro variation after
  authored layer composition without rotating or mirroring content. Repeated staging and full
  versus streamed 32-pixel bakes are byte-identical; legacy headless output remains unchanged.
- A user-owned installed map with one nine-point lake remained live through water decode,
  triangulation, G-buffer submission, lighting resolve, composite, and forward water rendering.
  Another installed map's empty water markers were preserved and ignored safely. No retail capture
  or data was retained.
- Near-horizontal camera frusta are intersected conservatively with the bounded terrain height
  slab rather than relying on one unstable focus ray. Horizon distance cannot dilute foreground
  density: screen-space depth caps bound the 16/32 tiers independently. Resident containment
  suppresses redundant work; new generations immediately cancel obsolete rows, tiles, and
  composition without the previous 120-ms request throttle. Each tier keeps its previous GPU patch
  during a short explicit-time overlap, avoiding visible blurry-to-sharp replacement. Trilinear
  sampling uses up to 16x anisotropy, with backend fallback handled by `wgpu`.
- A bounded Water INI decoder supplies global minimum opacity and opaque depth. The reusable,
  VFS-independent `WaterAppearance` accepts an optional consistent luminance-frame sequence;
  installed tools resolve `caust00` through `caust31` into a mipmapped texture array. The
  project-authored water shader projects those subtle frames on the underwater bed, reaches source
  opacity 1.0 by depth 3.0 in the observed default profile, and restores shallow shoreline haze
  and an animated crest without translating legacy fixed-function equations.
- Controlled release-viewer flight and wheel-dolly probes each compared 47,838 screen samples
  immediately after motion and four seconds later. Both produced zero mean RGB delta and no pixels
  above a three-level RGB threshold, so detail no longer visibly rises after camera motion. No
  capture or retail data was retained.
- The final optimized Bridge Busters viewer remained live for 12 seconds with installed caustic
  frames, source opacity/depth, shoreline effects, predictive LOD, complete mips, and anisotropic
  sampling active, then accepted a normal window close and exited with code zero. No capture or
  retail data was retained.
- The installed USA05 version-6 blend payload closed exactly at 400 by 320 / 128,000 cells, two
  terrain classes, no edge classes, and source-derived cliff flags. Its complete stable report
  exited zero, and the optimized viewer remained live for 12 seconds before a normal code-zero
  close. No MAP bytes or capture were retained.
- An automated release USA05 probe raised the camera to a shallow terrain angle and compared the
  immediate frame with one three seconds later. The nested tiers kept the terrain visually stable;
  the small remaining pixel delta was consistent with animated water rather than a visible terrain
  quality rise. Both temporary captures were deleted.
- A local user-owned installed smoke resolved all 14 semantic terrain classes, staged 151,221 cells
  and 907,326 indices, and rendered a coherent 768-by-768 capture. The capture was inspected and
  removed; only aggregate diagnostics are retained.

## Known refinements

- The current nested 16/32-texel terrain detail path is a bounded presentation milestone, not the
  final runtime texture architecture. Its frustum math is inexpensive, but CPU patch composition,
  mip generation, uploads, overlap, and replacement can still waste work or expose latency during
  camera movement, especially in debug builds. A later renderer refinement should retain the full
  height-field geometry, move semantic terrain-layer composition to the GPU, and rely on mipmapping
  and anisotropic filtering for screen-space detail. It must avoid a monolithic full-map
  high-resolution atlas; software virtual-texture paging remains an option only if substantially
  larger maps justify it.

## Known blockers

- `BIG4` remains implemented from corroborating source but unverified against retail data.
- Core glTF cannot reproduce exact W3D fixed-function multi-pass blending or animated mapper
  behavior; complete decoded metadata remains available to the project renderer.
- Adaptive-delta animation is synthetic-verified but has not yet been observed in an installed
  export.
- Exact legacy fixed-function equations and spatial environment/screen coordinate generation remain
  compatibility research beyond R2's documented preview policy.
- Version-1 MAP downsampling differs between legacy loading paths and remains preserved-but-unapplied
  until user-owned observations justify an explicit compatibility policy.
- MAP wrappers other than the source-established and installed-verified `EAR\0` RefPack form remain
  unsupported rather than guessed.
- Blend payload versions other than 6 and 7 remain opaque. Version 7's source-defined short
  cliff-row stride is normalized with unavailable right-edge flag bits cleared.
- The custom-edge preview preserves source atlas selection and separate geometry but does not claim
  bit-identical Direct3D 8 multipass blending.
- The installed Zero Hour Alpine Assault overlay uses unsupported `BlendTileData` version 8; the
  installed Generals version-7 map remains the verified terrain presentation artifact.
- MAP-authored terrain lighting is not decoded yet; `map-view` labels its fixed directional light
  as a presentation preview rather than source-equivalent lighting.
- Source `WaterSet` colors/textures, time-of-day appearance, and map-specific overrides are not
  decoded yet; global/default `WaterTransparency` opacity and opaque depth are decoded. SSR, planar
  reflection probes, and shadows remain later render-quality work.

## Next verified step

Decode bounded `GlobalLighting` and remaining source water-set appearance into renderer-neutral
inputs, then add modern shadowing and reflection quality without moving transmissive water into the
opaque G-buffer. Scripts and general object loading remain explicitly deferred.
