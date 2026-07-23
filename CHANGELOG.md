# Changelog

All notable user-visible changes are recorded here.

## Unreleased

### Added

- Added a pinned-source MAP scene compatibility matrix and exhaustive synthetic tests for every
  currently modeled constructor/default, parser input branch, format structure and limit, blend
  version/stride boundary, water trigger version/filter, road diagnostic/topology/atlas output,
  road mip shape, viewer input transition, and wireframe/depth-bias diagnostic value.
- Added source-backed Zero Hour MAP compatibility for `BlendTileData` version 8's corrected cliff
  bitmap stride and `PolygonTriggers` version 4's bounded WorldBuilder layer name. Synthetic tests
  cover the stride delta, truncation, limits, and unsupported neighboring versions.
- Added bounded immutable `WorldInfo`, `ObjectsList`, `SidesList`, build-list, team, and complete
  nested player-script decoders. Stable `map-objects` and `map-sides` reports expose exact scalar
  bits, typed dictionaries, endpoint flags, spawn candidates, and raw script opcodes/parameters
  without validation repair, live object construction, or script execution.
- Added source-order scene staging for road/bridge endpoints, visible scenery placements, hidden
  records, waypoints, and one-based `Player_n_Start` candidates. Definition resolution and actual
  road, bridge, building, vegetation, and prop rendering remain separate presentation gates.
- Added WaterSet sky/environment texture resolution, sibling-map overrides, Modern bounded
  screen-space/environment reflection inputs, and a frozen explicit presentation-time mode for
  `map-view`.
- Added bounded `Road` INI decoding and deterministic regular-road rendering in `map-view`.
  Consecutive Point1/Point2 records resolve source textures and widths, tessellate at terrain-cell
  intervals, follow maximum underlying height, and alpha-overlay in stable MAP order. Missing
  definitions/textures remain explicit diagnostics. Connected endpoints now receive bounded
  edge-derived corner/junction polygons instead of oversized circular fillers.
- Added bounded intact `Bridge` model/scale decoding and source-style bridge presentation from
  consecutive endpoints. `BRIDGE_LEFT`/`BRIDGE_SPAN`/`BRIDGE_RIGHT` sections repeat and deform onto
  the terrain-sampled sloped axis; damage, repair, collision, towers, and state selection remain
  deferred.
- Added an on-demand full-scene wireframe diagnostic to `map-view` on M when the selected GPU
  exposes polygon-line rasterization. Unsupported adapters continue with the normal renderer.
- Added bounded initial Object draw-definition decoding, reskin inheritance, default W3D model and
  scale selection, standalone static-mesh composition, and GPU-instanced static scenery in
  `map-view`. Placements sample the exact rendered terrain triangle, including MAP border and
  diagonal selection, then add the authored relative Z offset verbatim, including negative
  offsets and with no clamp or renderer epsilon.
- Added a renderer-only translucent playable-boundary fence. Its base follows perimeter terrain and
  its global top clears the map's highest terrain sample without changing pathing or simulation.

### Changed

- Restored the source road texture's three-level mip budget and handed curve traversal, and added a
  renderer-only road depth bias on top of the legacy terrain lift. This avoids whole-atlas distant
  mip collapse and reduces road/terrain Z-fighting without mutating staged road coordinates.

- Documented the repository-wide Zero Hour layering invariant: enumerate and mount Generals first,
  apply Zero Hour second and mods last; replacement resources use the winner while cumulative
  definition formats parse the complete provider history in order.
- Expanded the R3 design from terrain-only presentation to complete bounded MAP ingestion and a
  non-simulating terrain scene: source lighting and WIP water, object/world records, roads and
  bridges, static scenery and ambient animation, waypoints/player starts, sides/teams/build lists,
  polygon areas, and lossless map scripts. ADR 0009 keeps all runtime activation and script
  execution behind the future deterministic R5 simulation boundary.
- Inserted an R4 WND/UI compatibility milestone before simulation. The design selects a custom
  retained WND model and `wgpu` renderer, bounded UI resource loading, safe menu callback routing,
  a versioned post-parse WND patch layer, modern resolution/refresh-rate settings with confirmed
  apply/rollback, and a navigable main-menu/skirmish/map-selection demo using R3 map previews and
  spawn candidates.

### Fixed

- Road and railroad intersections no longer stretch each approach texture across a generic shared
  fan. A deterministic topology pass now trims connected strips and uses legacy curve/miter and
  tee/Y/slanted/four-way atlas geometry. Different materials stay isolated unless an open endpoint
  explicitly requests the legacy alpha-join cap.
- Initial map objects whose W3D draw fields are aligned with their `Draw` declaration now render.
  The bounded parser follows `End`-delimited modules and recognizes the source-equivalent first
  `ConditionState = NONE`, restoring supply docks/stashes, command centers, and similarly authored
  campaign structures without constructing gameplay objects.

- `map-view` now uses an explicit legacy-preview W3D recovery policy for damaged shipped assets:
  missing optional HLOD meshes are skipped, invalid one-past-end HLOD/skin references fall back to
  a rigid root/pivot, and non-finite UVs become zero only at presentation/export boundaries while
  their immutable exact bits remain preserved. Strict W3D composition remains unchanged.
- Intact bridges no longer treat the complete W3D as a midpoint-scaled static prop. Their endpoint
  marker height, repeated sections, lateral scale, slope, and orientation now follow the dedicated
  bridge presentation path.

- Static W3D meshes now honor the Header3 two-sided flag: ordinary meshes cull back faces while
  explicitly two-sided foliage and planar props retain both sides, removing coplanar backface
  flicker caused by the previous global no-cull policy.

- Version-4 height maps now preserve signed playable-boundary coordinates instead of rejecting
  negative values accepted by the source reader. River staging now honors the stored seam index
  and walks the two perimeter banks in opposite directions, eliminating crossing or bank-only
  ribbons on long rivers.
- Terrain detail now uses a 256-page cache, a slightly farther screen-density threshold, and
  distance cross-fades between 32-, 16-, and 8-texel tiers. Large inward-facing frusta no longer
  consume the complete cache with coarse pages and expose direction-dependent blurry boundaries.
  Keyboard and wheel flight also use frame-rate-independent acceleration and deceleration.
- Generals standing water now starts from the source constructor defaults before ordered base,
  expansion, mod, and map-local INI overrides. This restores its default standing texture instead
  of falling back to a flat diagnostic surface and honors companion `Map.ini` water settings.
- Terrain and water definitions now accumulate every shadowed VFS provider in stable mount order.
  Zero Hour therefore retains inherited Generals terrain classes such as those used by CHI01.
- Source-compatible zero-entry cliff-info tables are accepted as empty instead of rejected,
  allowing affected version-7 maps such as USA07 to load.
- Zero Hour `WaterTransparency` standing and radar colors now accept the source byte-RGB syntax
  and normalize it at the immutable format boundary, allowing maps using the installed profile to
  pass water configuration loading.
- Default legacy water no longer replaces the scene with an opaque procedural gray surface. It
  resolves the source standing-water texture, selected diffuse tint/alpha, additive policy, and
  depth opacity, then alpha-composites with terrain-depth shoreline feathering. The existing
  refractive presentation remains available only under the explicit Modern policy.
- Streamed custom-edge transparency now remains authored coverage instead of being mistaken for a
  missing virtual page. The edge pass composites only albedo and no longer overwrites deferred
  normals or world positions; smooth height-field vertex normals also remove exaggerated
  per-triangle terrain faceting in the interactive viewer.
- Water INI integer RGBA fields now accept the source-established optional alpha channel and
  default omitted alpha to 255, allowing installed vertex-color definitions to load correctly.
- Headless terrain and map-render capture tests now skip when the host exposes no graphics adapter,
  matching the existing synthetic capture policy while preserving real renderer and hash failures.
- Linux and macOS builds no longer retain the Windows-only Steam registry command import.
- Angled terrain views now select virtual-texture detail in camera-space depth and rank projected
  page bounds instead of filling a world-axis square around the viewport footprint. Coarse visible
  coverage is retained before fine upgrades, removing the misplaced rectangular LOD island.

### Added

- Bounded `GlobalLighting` versions 1 through 3 with separate ordered terrain/object lights for
  morning through night, optional packed shadow color, exact-bit `map-lighting` reports, and
  selected-time viewer shading. The complete source-established `WaterSet` and
  `WaterTransparency` field tables are retained under explicit limits; selected diffuse color,
  standing-water texture/blend policy, opacity, and scroll inputs now feed the forward-water
  presentation.
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
- Added bounded immutable `BlendTileData` version-6/7/8 tile planes, version-6 source-equivalent
  height-derived cliff flags, version-7 legacy cliff-bitmap normalization, version-8 corrected
  cliff rows, terrain and edge texture classes, blend records, and cliff UV records, plus a stable
  VFS-backed `cic-inspect map-blend` report.
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
