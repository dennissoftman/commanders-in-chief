# R3: Complete MAP ingestion and terrain-scene presentation

**Status:** Complete (2026-07-23). Bounded MAP ingestion, non-simulating scene presentation, and
deterministic overview capture are established.

**Scope:** Boundedly decode and retain every source-established MAP section needed to inspect a
map and construct its complete pre-simulation scene. This includes terrain, texture blends,
boundaries, world metadata, water, global lighting, polygon areas, placed objects, waypoints,
player starts, sides, teams, build lists, roads, bridges, and the complete map-script tree. Resolve
only the external INI definitions and VFS assets needed to present that scene: terrain and road
definitions, object draw definitions, W3D models, textures, ambient visual animation, and water
appearance. R3 ends with a useful map viewer containing terrain and all resolvable map-authored
static scenery, including buildings, trees, rocks, props, road/bridge geometry, decals, and light
objects.

**Exclusions:** Simulation mutation of any kind: script execution, player/team activation, object
logic modules, AI, pathfinding, locomotion, collision, damage and bridge state transitions,
economy, combat, networking, replay, and wall-clock-driven authoritative state. Asset editing,
retail fixtures, guessed layouts for unobserved versions, and a requirement to parse unrelated INI
gameplay fields are also excluded. MAP scripts are data in R3, never callbacks.

**Inputs:** Original synthetic MAP byte streams; user-owned installed and custom maps through the
VFS; and user-owned INI, W3D, texture, and animation resources referenced by decoded map data.

**Outputs:** Stable inventory and semantic reports; immutable renderer-neutral MAP values; a
cross-referenced but non-executing scenario description; a resolved presentation scene containing
terrain, water, roads, bridges, and placed drawables; deterministic synthetic captures; and an
opt-in interactive viewer. Unknown chunks, dictionary keys, enum/opcode values, and unresolved
resource names remain inspectable rather than being silently discarded.

**Owner:** `cic-formats` owns bounded MAP and narrowly scoped INI decoding into immutable values;
`cic-tools` composes VFS resources, definition lookups, cross-reference reports, and viewer launch
inputs; `cic-render` owns terrain/object/road staging and presentation. The future simulation layer
will consume the same scenario values but may not be called by any R3 parser or renderer.

**Acceptance tests:** Every semantic gate requires exact chunk closure, established-version
dispatch, truncation at every field, explicit count/string/recursion/allocation limits, unknown
preservation, stable file-order reports, original negative fixtures, and unresolved-reference
diagnostics. Presentation gates additionally require deterministic synthetic scene/capture hashes,
explicit animation and camera time, stable instance/draw ordering, missing-resource fallbacks, and
local installed/custom-map smokes that retain no retail data. Every source-derived constructor
default, modeled function input branch/output class, and persisted format field must be linked from
a pinned-source test matrix to an executable synthetic test; deliberately unmodeled runtime state
must be named as an exclusion instead of receiving guessed defaults.

**Determinism:** MAP file order is authoritative. Dictionary, side, team, build-list, waypoint,
road, object, trigger, script-group, script, condition, action, and parameter order is retained.
Cross-reference indices and renderer submissions use stable IDs derived from that order. Definition
overrides follow explicit VFS mount order. Diagnostic animation uses explicit time and seed inputs;
filesystem order, locale, randomized hash iteration, host time, and camera-driven simulation state
never affect reports or captures.

**Documentation:** `docs/formats/map.md`, `docs/provenance/map.md`, the compatibility matrix, ADR
0009, architecture boundaries, and capture instructions. Each implemented semantic gate must add
its source revision and notices, exact limits, version table, synthetic fixture, negative tests,
report schema, and completion evidence.

**Completion artifact:** The original synthetic MAP/INI/W3D fixture family spans terrain, water,
lighting, objects, roads, player starts, sides/teams, polygon areas, and a nested but non-executed
script tree. The pinned executable matrix ties those fixtures to checked reports and deterministic
scene captures; local user-owned verification retains only aggregate diagnostics and hashes.

### R3 gates

1. **Source lighting and water convergence (complete).** Retain the established bounded
   `GlobalLighting` terrain/object time variants and source light order. WaterSet sky/environment
   and sibling-map inputs now resolve alongside the standing texture, diffuse tint/alpha, blend
   choice, opacity, and terrain-depth shoreline. Modern presentation adds bounded screen-space and
   authored-environment reflection inputs. Water is a depth-tested forward pass over the resolved
   opaque scene, samples the shared directional shadow map, and is followed by bounded edge-aware
   anti-aliasing. Explicit-time full-scene hashes and repeatable user-owned comparisons close the
   integration gate without claiming exact Direct3D 8 pixel equivalence.
2. **Placed-object and world metadata boundary (implemented).** Established `WorldInfo`,
   `ObjectsList`, and nested `Object` versions decode without constructing live objects. They retain finite XYZ placement,
   angle, source flags, template name, typed dictionary, waypoint fields, mirror/draw policy, and
   unknown properties under explicit limits. Emit stable reports and cross-reference diagnostics,
   but never repair, canonicalize, or execute the source data during parsing.
3. **Road and bridge presentation (source road topology, intact bridges, and tower scenery
   implemented).**
   Source-established road/bridge endpoint flags stage in object order. Bounded `Road` definitions
   resolve regular consecutive pairs into source-textured, terrain-fitted strips. A stable topology
   pass trims connected approaches and inserts source-radius curves/miters, tee/Y/slanted/four-way
   atlas pieces, and explicit cross-material alpha caps. Cross-material contacts reproduce the
   source clipped-width cap and road-type stacking adjustment. Road textures use the source
   three-level mip budget. The bounded TerrainBridge subset resolves the pristine model/scale and
   paired endpoints through static instancing. It retains damaged/really-damaged/broken model and
   texture references, resolves the four optional tower object templates through the existing
   bounded object/W3D path, and presents those towers in stable source slot order. Transition
   effects remain future simulation inputs; R3 never selects a damage state or creates targetable
   towers, collision, repair, sounds, or effects.
4. **Definition resolution and complete static scene (R3 subset complete).** The bounded
   object-definition subset uses `End`-delimited draw modules and selects either explicit default
   states or the source-equivalent first `ConditionState = NONE`, plus reskin inheritance,
   referenced models, and per-draw scale. Default W3Ds reuse the R2 material/hierarchy path, standalone mesh
   W3Ds receive a neutral renderer-only root, and placements batch stably by first model use. Ground
   placement samples the exact staged terrain triangle and adds the authored relative Z offset.
   Header3 two-sided flags select culled or two-sided model pipelines. `W3DTreeDraw` resources use
   explicit-time source-default `BreezeInfo` sway without executing `SET_TREE_SWAY`. Buildings,
   trees, rocks, props, bridges, decals,
   and other placed drawables use stable placement IDs, culling, batching, and explicit
   presentation time. Vegetation waving and other ambient loops must use decoded source inputs or
   an explicitly documented profile policy; they may not advance simulation or read a renderer
   clock. Missing or unsupported definitions remain visible diagnostics/placeholders.
5. **Waypoints, player starts, sides, teams, and build lists (implemented data boundary).**
   Waypoint metadata preserves the one-based `Player_n_Start` convention as spawn candidates. Established
   `SidesList` versions, typed side/team dictionaries, ownership/alliance names, build-list
   placements, and nested player-script lists. Reports must distinguish spawn positions, side/team
   definitions, initial/build-list scenery, and dangling references. R3 does not assign human/AI
   controllers, instantiate teams, run build plans, or choose spawn slots.
6. **Complete script and trigger ingestion without execution (complete).** All established
   `PolygonTriggers` records are retained in source order and the water-only projection remains
   available. Established nested `PlayerScriptsList`, `ScriptList`,
   `ScriptGroup`, `Script`, `OrCondition`, `Condition`, `ScriptAction`, and `ScriptActionFalse`
   records. Preserve names, comments, activation/difficulty flags, evaluation delays, opcode
   integers, typed parameters, source versions, and unknown values in a bounded immutable tree.
   Stable reports and optional reference diagnostics are allowed; opcode dispatch, condition
   evaluation, timers, side effects, and compatibility rewrites belong to R5.
7. **Scene integration and R3 closure (complete).** Present all resolved opaque scenery through the existing
   G-buffer, then ordered alpha/additive scenery and forward water. The viewer adds a shared primary
   directional shadow map and edge-aware anti-aliasing; `map-render --time` emits a deterministic
   full-scene overview. Both profiles retain version-1 height data at its native stored grid.
   Source-editor preview/auxiliary chunks remain opaque because they are not scene inputs; R4
   generates previews from the completed renderer. Dense installed and original synthetic fixtures
   verify closure, reporting, continuity, placement, ambient animation, water, stable output, and
   graceful diagnostics.

**Progress:** The initial source-backed gate inventories the `CkMp` symbol table and top-level
chunks with exact closure and opaque payload preservation. A separate semantic decoder accepts
`HeightMapData` versions 1 through 4, validates dimensions, border, boundaries, and exact row-major
sample cardinality, and retains the stored version-1 grid pending an explicit compatibility policy.
`BlendTileData` versions 6 through 8 decode bounded planes and source-ordered terrain, edge, blend,
and cliff tables, including the version-7 legacy and version-8 corrected cliff strides. Bounded
`EAR\0` RefPack decompression, Terrain INI resolution, deterministic layered
terrain capture, custom edges, legacy/modern policies, and the interactive viewer are established.
The viewer uses a persistent bounded GPU-composed virtual-texture cache with stable page tables,
LRU residency, bordered 16/32-pixel pages, compute-generated mip chains, trilinear filtering, and
anisotropy over the guaranteed 8-pixel fallback. Water-only `PolygonTriggers` versions 2 through 4,
including bounded version-4 layer names, lake/river staging,
global transparency scalars, optional source caustic frames, hybrid-deferred composition, and
Modern de-tiling are implemented. `GlobalLighting` versions 1 through 3 now retain four ordered
time variants, separate terrain/object sun and accent records, and optional shadow color;
`map-lighting` reports exact scalar bits and `map-view` shades terrain and water from the selected
terrain lights. The complete source `WaterSet` and `WaterTransparency` field tables are bounded and
retained, with selected transparent color and scroll driving the current forward-water pass.
Water now resolves standing, sky, and environment textures after sibling-map overrides; Modern
presentation includes bounded screen-space/environment reflection inputs, and the viewer can freeze
explicit presentation time. Immutable world/object, waypoint/start, side/team/build-list, and
complete nested script data decode under independent limits with stable reports. Source-order scene
staging separates endpoint, scenery, hidden, waypoint, and start records. Road INI definitions
resolve first-used materials and consecutive endpoint pairs; bounded strips sample the maximum
terrain cell height at source intervals. A stable topology pass applies source curve/miter
traversal, atlas-specific junctions, clipped-width cross-material caps, and road-type stacking.
The viewer retains the source three-level road mip budget, adds renderer-only depth bias, and offers
optional full-scene wireframe on M. `End`-delimited default/initial-NONE object draw resolution,
stable static-model instancing, exact terrain-triangle placement, a renderer-only playable-boundary
fence, intact bridges with retained state assets and tower scenery, and source mesh culling are
integrated. Complete polygon retention/reporting, explicit-time default-breeze tree sway, shared
terrain/scenery/water shadows, edge-aware anti-aliasing, and deterministic full-scene overview
capture close R3. Unsupported draw modules remain visible diagnostics and gameplay-bearing modules
remain excluded rather than blocking this presentation milestone.

## Closure summary

R3 is complete and owns bounded MAP ingestion and pre-simulation scene presentation, not terrain
alone. The established terrain gate includes water-only polygon decoding, stable lake/river
staging, a hybrid-deferred viewer with forward water, source caustic/transparency inputs, Modern
macro variation, horizon-safe GPU page composition, persistent LRU residency, complete mip chains,
anisotropic sampling, shared directional shadows, and edge-aware anti-aliasing. Immutable
world/object and
sides/teams/build-list/script data now decode under explicit limits, and source-order scene staging
classifies endpoints, scenery, waypoints, and player starts without constructing live objects.
Bounded `Road` definitions now resolve regular Point1/Point2 pairs into terrain-fitted textured
strips after a stable topology pass inserts legacy-radius curves/miters, dedicated atlas
tee/Y/slanted/four-way junctions, and authored cross-material alpha caps in `map-view`. Bounded
initial Object draw definitions resolve `End`-delimited default/initial-NONE W3D models, including
standalone meshes, and render
placements composed from the exact rendered terrain triangle plus verbatim authored Z offsets
through stable GPU instance batches. Header3 two-sided flags now select model culling policy, and
bounded intact bridge models now stitch and deform named left/span/right sections between paired
terrain-sampled endpoints. Bridge definitions retain pristine/damaged/really-damaged/broken
model/texture references plus four source-ordered tower object names. The pristine preview resolves
optional towers through the bounded object/W3D path and places renderer-only instances at the
source bridge corners and facing without constructing targetable objects. An explicit legacy-preview
W3D policy recovers missing optional meshes,
bad one-past-end hierarchy references, and non-finite UV presentation without weakening strict
composition. The primary playable boundary is
visible as a terrain-following translucent fence whose top clears the map's highest terrain. The
viewer now exposes optional full-scene wireframe on M, limits road textures to the source three mip
levels, and applies project-authored render depth bias after the source height lift so distant roads
remain inspectable without changing immutable placement geometry. The
source-derived MAP scene boundary now has a pinned-source executable test matrix covering every
modeled constructor/default, function input/output branch, binary structure field, version boundary,
limit, road topology, and atlas primitive; deliberately unmodeled legacy runtime state is recorded
as an exclusion rather than assigned speculative values. The
completed scene also retains all polygon areas, applies explicit-time default-breeze tree sway, and
supports deterministic fixed-isometric full-scene overview capture. Renderer-only diagnostic
geometry now shows ordinary waypoints, per-player start candidates, and terrain-following polygon
perimeters in both the interactive viewer and overview capture. Named waypoint paths receive
deterministic distinct colors and continuous terrain-following ribbons in stored waypoint-ID order;
multi-path waypoints remain members of every declared path. Scripts
are inspectable in R3 but cannot be executed until the deterministic simulation boundary begins in
R5.

## Completion evidence

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
- `BlendTileData` versions 6 through 8 decode into bounded immutable signed tile-index planes, terrain
  and edge texture classes, blend selector records, and finite cliff UV records. Version 6 derives
  cliff flags from neighboring heights using the source threshold; version 7 normalizes its stored
  short-stride bitmap; version 8 reads the corrected conventional row stride. Source-compatible zero
  cliff-info counts retain raw zero and produce an empty table. `cic-inspect map-blend` preserves
  stable source order and exact UV bits.
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
- `PolygonTriggers` versions 2 through 4 now have a bounded complete decoder that retains every
  source-ordered area, water/river flags, identifier, name, version-4 WorldBuilder layer name,
  seam index, and integer point under per-area and total-point limits. `map-water` remains a stable
  compatibility projection; degenerate water markers safely produce no renderer geometry.
- `map-polygons` emits the complete immutable area report. Synthetic tests cover established
  versions, all truncated prefixes, neighboring-version rejection, strings, per-area points, total
  retained points, and stable source indices.
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
- The viewer renders terrain and alpha-tested static scenery into one 2048-square primary
  directional shadow map and samples it from deferred opaque lighting and forward water with
  bounded 3-by-3 PCF. Its final composite uses edge-aware post-process anti-aliasing.
- `W3DTreeDraw` resources now resolve separately from ordinary model draws and receive
  source-default `BreezeInfo` direction, lean, intensity, five-second period, bounded randomness,
  and one of ten deterministic placement-ID sway families. Presentation samples explicit seconds;
  decoded `SET_TREE_SWAY` remains inert until R5.
- `map-render --time` now emits a deterministic fixed-isometric full-scene overview rather than a
  terrain-only image. It layers source-ordered road and water triangles plus scenery markers over
  the GPU terrain capture and reports all scene counts with the RGBA hash.
- `map-view` and `map-render` stage bounded octahedral markers for every waypoint and larger,
  stable-color markers for one-based `Player_n_Start` candidates. All polygon areas render as
  source-ordered translucent terrain-following perimeter walls; water polygons are visibly
  distinct. Up to three retained path labels per waypoint form case-insensitive, lexically ordered
  color groups whose members connect in stored waypoint-ID order with bounded terrain-following
  ribbons. These diagnostics neither register spatial triggers nor create players.
- The deferred-lighting, composite, terrain virtual-texture, and terrain-shadow pipeline layouts
  now match their exact WGSL bindings. A GPU regression test constructs the deferred pipelines;
  the reported release map remains live with shadows, AA, 197 waypoint markers, and 18 zones.
- A user-owned path-bearing map grouped those 197 waypoints into 26 named paths and emitted 1,678
  bounded terrain-following ribbon sections. Two 768-square release captures at explicit time 2
  matched RGBA SHA-256 `cdaac067e69fb61423bf688d3364d9d36f66474f947435be7b76dc8951d98461`;
  both temporary captures were deleted.
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
- The final installed overview smoke retained all six polygon areas and 54 points on one dense map.
  A separate water-bearing map staged 448 road draws, 1,995 scenery instances across 119 models,
  862 boundary segments, and three water areas; two 256-square captures at explicit time 2 matched
  RGBA SHA-256 `ba60f7a4aeb92680366ab15170cfe1521de9acfdbf3f6abf5d5d7fc6dc71660e`.
  Temporary captures were deleted and no retail bytes, names, coordinates, or images were retained.


## Known limitations and deferred observations

- Version-1 MAP downsampling differs between legacy loading paths. R3 explicitly retains and
  presents the native stored grid; any future historical downsampled view must be a separate
  versioned compatibility policy.
- MAP wrappers other than the source-established and installed-verified `EAR\0` RefPack form remain
  unsupported rather than guessed.
- Blend payload versions other than 6 through 8 remain opaque. Version 7's source-defined short
  cliff-row stride is normalized with unavailable right-edge flag bits cleared.
- The custom-edge preview preserves source atlas selection and separate geometry but does not claim
  bit-identical Direct3D 8 multipass blending.
- The installed Zero Hour Alpine Assault overlay validated `BlendTileData` version 8 at 380 by 400
  cells with a corrected 48-byte cliff stride. Its optimized `map-view` smoke remained live for 30
  seconds and staged 198 scenery instances across 70 models without missing or invalid resources.
- Source standing-water texture/color/blend/opacity and WaterSet sky/environment textures drive the
  selected appearance, including sibling `Map.ini` overrides. Modern water adds bounded
  screen-space/environment reflection inputs; shared shadows, edge-aware anti-aliasing,
  explicit-time overview hashes, and repeatable user-owned comparisons complete the R3 baseline.
  Exact legacy fixed-function pixel equivalence remains excluded.
- `WorldInfo`, complete `ObjectsList`/`Object` records, waypoint/player-start metadata,
  `SidesList`, teams, build lists, and the nested player-script tree now have bounded immutable
  decoders and stable reports. Source-order staging classifies road/bridge endpoints, scenery,
  hidden records, waypoints, and player starts. Regular `Road` definitions now resolve source
  texture/width inputs and render terrain-fitted strips plus bounded source-atlas curve, miter,
  tee, Y, slanted-tee, four-way, and explicit cross-material alpha-join geometry in stable MAP
  order. Initial W3D draw states and reskins now resolve `End`-delimited default or initial-NONE
  states to static model instances whose ground placement includes the MAP border, exact rendered
  triangle, and authored relative Z; standalone mesh W3Ds receive a neutral renderer-only root. The
  boundary fence is renderer-only. Intact TerrainBridge models, retained body-state assets,
  renderer-only tower scenery, and Header3-driven static culling are implemented; damage selection
  stays deferred. Explicit-time default-breeze tree animation and complete polygon retention are
  implemented without script execution.
  The installed Alpine Assault completion run staged 678 road draws with zero road diagnostics and
  592 scenery instances across 74 models; its greatest emitted road-triangle edge was an ordinary
  75.519-unit edge on a 52-unit-wide DirtRoad strip rather than inserted junction geometry.
