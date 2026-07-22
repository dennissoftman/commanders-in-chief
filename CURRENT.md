# Current Objective

## Objective

R3 now owns complete bounded MAP ingestion and pre-simulation scene presentation, not terrain
alone. The established terrain gate includes water-only polygon decoding, stable lake/river
staging, a hybrid-deferred viewer with forward water, source caustic/transparency inputs, Modern
macro variation, horizon-safe GPU page composition, persistent LRU residency, complete mip chains,
and anisotropic sampling. Water remains visibly work in progress. The next design sequence is
source lighting and water convergence, immutable object/world decoding, road and bridge staging,
bounded object-definition resolution, complete static scenery with explicit-time ambient animation,
and lossless sides/teams/spawns/scripts ingestion. Scripts must be inspectable in R3 but cannot be
executed until the deterministic simulation boundary begins in R5. After R3 closes, R4 will add
bounded WND/UI ingestion and a navigable `wgpu` main-menu/skirmish demo so map compatibility can be
inspected through the intended shell before simulation exists. Its Options path will use bounded
post-parse WND patches—not hardcoded window-name rendering—to add modern window mode, resolution,
refresh-rate, and UI-scale controls with transactional confirmation/rollback.

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
- Source-established bare and `EAR\0` RefPack-wrapped `CkMp` MAP streams decode under explicit
  compressed/decompressed size, symbol, name, chunk, payload, and back-reference limits.
- MAP inventories preserve symbol and top-level chunk file order, apply deterministic
  last-table-entry-wins name resolution, and retain every known or unknown payload opaquely because
  the format has no generic child-container marker.
- `HeightMapData` versions 1 through 4 decode into immutable stored dimensions, border, boundaries,
  cell spacing, and exact row-major byte samples under explicit dimension/allocation limits.
  Version-4 boundary pairs remain signed source metadata; negative coordinates are preserved.
- `cic-inspect map` and `map-height --report` produce stable VFS-backed reports; `map-height`
  writes exact row-major samples as a deterministic 8-bit grayscale PNG by default. The original
  synthetic MAP and BIG completion artifact pass, including version dispatch and negative tests.
- `BlendTileData` versions 6 and 7 decode into bounded immutable signed tile-index planes, terrain
  and edge texture classes, blend selector records, and finite cliff UV records. Version 6 derives
  cliff flags from neighboring heights using the source threshold; version 7 normalizes its stored
  short-stride bitmap. Source-compatible zero cliff-info counts retain raw zero and produce an
  empty table. `cic-inspect map-blend` preserves stable source order and exact UV bits.
- One user-owned installed RefPack MAP closed at 1,781,076 decompressed bytes as 46 symbols and 8
  chunks; its version-4 380-by-400 height field validated 152,000 samples. Its version-7 blend
  payload validated the same 152,000 cells, 204 bitmap tiles, 7,772 blend table entries, 14 terrain
  texture classes, and one cliff-info table entry. A 380-by-400 PNG smoke was verified and removed;
  only aggregate counts were retained.
- `map-height` now writes an 8-bit grayscale PNG by default, deriving the filename from the MAP
  resource basename; `--report` retains the stable text report and `--png` overrides the path.
- A bounded Terrain INI decoder preserves ordered declarations. `map-render` applies explicit
  `DefaultTerrain` inheritance/override semantics across every provider in stable base-to-overlay
  VFS history and resolves semantic MAP texture classes through VFS-backed `Art/Terrain` sheets.
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
- `map-view` uploads immutable semantic cells and compact 64/32-pixel source-tile atlases once, then
  composes bordered 16/32-texel pages in compute shaders. A persistent 256-layer physical cache and
  stable two-level page tables reuse revisited regions; the deterministic 8-texel background is the
  guaranteed fallback when a page is absent. Angled views use camera-space depth and projected page
  bounds, reserving coarse visible coverage before fine upgrades instead of selecting a world-axis
  square around the footprint midpoint.
- Detail demand extends slightly beyond the visible density crossover and the fragment path
  distance-cross-fades fine, coarse, and fallback samples. The larger cache prevents a long
  inward-facing frustum from exhausting residency before fine pages are considered, removing the
  map-edge-dependent half-turn asymmetry. Keyboard and wheel movement integrate an explicit
  first-order velocity response, so acceleration and deceleration are smooth and frame-rate
  independent.
- Viewer-only smooth normals are derived from bounded neighboring height samples, and an explicit
  directional preview light improves slope readability without altering source geometry or
  headless completion hashes. The installed Generals window remained live with the detail and
  lighting path active; no capture was retained.
- Terrain, custom-edge, and nested-detail pipelines cull clockwise back faces from the established
  counter-clockwise height-field winding. Synthetic headless capture hashes remain unchanged;
  water remains a separately ordered material rather than inheriting terrain culling policy.
- `PolygonTriggers` versions 2 and 3 now have a bounded water-only decoder that retains stable
  water/river flags, identifiers, names, seam indices, and integer points while skipping general
  trigger semantics and allocations for non-water points. Degenerate markers are preserved and
  safely produce no renderer geometry.
- River staging uses the stored seam index exactly as the source renderer does: one bank advances
  through the perimeter while the other retreats with bounded wraparound, producing stable paired
  cross-sections instead of pairing adjacent points on the same bank. Invalid seam metadata safely
  produces no geometry.
- `map-water` emits a stable water-only report. Synthetic tests cover every truncated prefix,
  explicit trigger/name/point limits, established version dispatch, degenerate markers, and stable
  lake-fan/river-strip triangulation.
- `map-view` writes opaque terrain and near detail into albedo, normal/roughness, world-position,
  and depth targets. Custom edges alpha-composite only into albedo, leaving the base terrain's
  geometry buffers intact. Directional lighting resolves into linear `RGBA16F`, tone maps to the
  surface, then renders water in a depth-tested/no-depth-write forward pass. The original project
  shader applies thickness absorption, refraction, Fresnel sky response, specular, and shallow
  foam; no legacy water-rendering algorithm was translated.
- `Modern` terrain policy applies deterministic world-anchored integer macro variation after
  authored layer composition without rotating or mirroring content. Repeated staging and full
  versus streamed 32-pixel bakes are byte-identical; legacy headless output remains unchanged.
- A user-owned installed map with one nine-point lake remained live through water decode,
  triangulation, G-buffer submission, lighting resolve, composite, and forward water rendering.
  Another installed map's empty water markers were preserved and ignored safely. No retail capture
  or data was retained.
- Near-horizontal camera frusta are intersected conservatively with the bounded terrain height
  slab rather than relying on one unstable focus ray. Horizon distance cannot dilute foreground
  density: screen-space depth caps request 16/32 tiers independently. Camera motion updates only
  small page-table/job buffers; GPU composition preserves base/primary/extra masks, cliff UVs,
  custom edges, and Modern macro variation. Every physical page has a GPU-generated linear,
  alpha-aware mip chain and up to 16x anisotropic sampling.
- A bounded Water INI decoder supplies global minimum opacity and opaque depth. The reusable,
  VFS-independent `WaterAppearance` accepts an optional consistent luminance-frame sequence;
  installed tools resolve `caust00` through `caust31` into a mipmapped texture array. The
  project-authored water shader projects those subtle frames on the underwater bed, reaches source
  opacity 1.0 by depth 3.0 in the observed default profile, and restores shallow shoreline haze
  and an animated crest without translating legacy fixed-function equations.
- `GlobalLighting` versions 1 through 3 decode into four ordered time variants with separate
  terrain/object sun and versioned accent records, finite exact-bit scalars, a validated one-based
  selected time, and optional packed shadow color. `map-lighting` produces a stable VFS-backed
  report; its original synthetic BIG artifact and every-truncated-prefix tests pass.
- A user-owned installed USA05 MAP closed as `GlobalLighting` version 3 with afternoon selected,
  four time variants, three terrain and three object lights per variant, and a final shadow color.
  Only these aggregate facts were retained; no MAP bytes or scalar values were copied.
- The complete source-established `WaterSet` and `WaterTransparency` INI field tables now decode
  under explicit file, line, definition, string, scalar, count, color, nesting, and closure limits.
  `map-view` uses the MAP-selected terrain sun/accents for deferred terrain and forward-water
  specular response. Default legacy water starts from the source constructor defaults, applies
  every global provider in stable base-to-overlay order and then the sibling `Map.ini`, and resolves
  the selected WaterSet diffuse color/alpha plus the WaterTransparency standing texture, color
  override, additive policy, minimum opacity, opaque depth, and animation rate. Maps without a
  lighting chunk retain the explicitly documented preview fallback.
- Source integer RGBA syntax permits alpha to be omitted and defaults it to 255. The installed
  Generals Water INI exercised this form on vertex colors; USA01 then cleared parsing and resource
  staging and remained in the live viewer until the controlled smoke timeout. No capture or retail
  values were retained.
- `WaterTransparency` standing/radar colors use source byte-RGB syntax and normalize once into the
  renderer-neutral model. The installed Zero Hour profile exercised this form while loading Bridge
  Busters, which cleared configuration/resource staging and remained live for a controlled
  12-second optimized viewer smoke. No retail configuration bytes or values were retained.
- Virtual-page residency is now independent of sampled custom-edge alpha, preserving authored
  transparent gaps rather than filling them from lower-resolution fallbacks. Edge draws no longer
  blend normal/world-position targets, and viewer vertex normals smooth the source height grid.
  The optimized USA01 viewer remained live for a controlled 12-second GPU smoke after these fixes;
  no capture or retail data was retained.
- USA01's version-7 blend data reports 23 terrain classes and 8,425 ordinary blend records but zero
  custom-edge tiles/classes. Its isolated stair-step road transition is therefore map-authored
  ordinary cell blending, not a custom-edge renderer artifact. Only these aggregate facts were
  retained.
- The default legacy water policy now samples the installed source standing-water texture with its
  selected diffuse tint/alpha, source additive choice, and depth-derived shoreline coverage. It
  alpha-composites over the scene instead of overwriting it with the Modern diagnostic material;
  `--terrain-policy modern` explicitly retains the refractive branch. The optimized USA01 viewer
  resolved those inputs and remained live for a controlled 12-second smoke. No capture or retail
  asset was retained.
- Optimized smokes verified that Generals USA01 resolves its constructor-default standing texture,
  Zero Hour CHI01 inherits a terrain class from the shadowed Generals Terrain INI, and USA07 accepts
  its source-compatible empty cliff-info table. Each viewer remained live until the controlled
  timeout; no retail bytes or captures were retained.
- USA06 contains one renderable static reservoir water polygon and two degenerate water markers.
  Its downstream channel is traced by ambient water-loop objects and dam mission state rather than
  a second static water polygon. The viewer now applies its map-local water override to the static
  reservoir; R3 preserves but does not execute the downstream mission script state. Only aggregate
  observations were retained.
- Final Crusade's version-4 height data verified signed boundary preservation and Heartland Shield's
  long river verified nonzero-seam bank reconstruction. Both optimized viewers remained live for
  controlled smokes; no retail bytes, coordinate values, or captures were retained.
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
- The optimized USA05 viewer remained live for a controlled 12-second smoke with GPU page
  composition, page-table fallback, custom-edge cache output, full mip generation, and deferred
  terrain/water rendering active. The test process was terminated without retaining a capture.
- The optimized USA06 viewer remained live for 15 seconds after angled LOD selection moved from a
  radial/world-axis approximation to camera-space depth and projected page ranking. Regression
  tests preserve the angled cutoff and coarse-visible-before-fine policy; no capture was retained.

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
- Source standing-water texture/color/blend/opacity values now drive the default legacy path, but
  WaterSet sky/environment textures, map-embedded overrides, shadow receiving/casting, SSR or
  planar probes, and anti-aliasing remain open. Water is not accepted as the final R3 visual
  baseline until explicit-time captures and repeatable user-owned comparisons pass.
- `WorldInfo`, complete `ObjectsList`/`Object` records, road and bridge endpoints, object draw
  definitions, static scenery placement, waypoint/player-start metadata, `SidesList`, teams, build
  lists, non-water polygon semantics, and the nested player-script tree remain opaque. Their R3
  boundary is immutable inspection and presentation only; runtime activation and script execution
  belong to R5.

## Next verified step

Complete the source-lighting/water presentation gate with WaterSet sky/environment resolution,
map-embedded water overrides, explicit-time synthetic captures, and repeatable user-owned visual
comparisons. Then open the immutable `WorldInfo`/`ObjectsList` gate that unlocks roads, bridges,
buildings, trees, props, waypoints, and player-start inspection. Continue through the ordered R3
gates in `ROADMAP.md`; decode the complete map-script tree only as data, with all execution deferred
to R5. R4 then consumes the completed R3 map catalog and spawn previews for its main-menu,
skirmish-options, and map-selection UI demo.
