# Roadmap

Progress is measured by compatibility gates, not elapsed time.

## R0: Repository and resource-probe foundation

**Status:** Complete. GitHub CI run `29840005186` passed the completion suite.

**Scope:** GPL/provenance policy, Rust workspace, bounded reader, normalized VFS paths,
loose-directory mounts, deterministic manifest CLI, tests, and CI.

**Exclusions:** Archive formats, retail assets, rendering, simulation, AI, networking.

**Inputs:** Synthetic byte arrays and temporary directory trees.

**Outputs:** Structured parse errors and stable tab-separated resource manifests.

**Owner:** `cic-core`, `cic-vfs`, and `cic-tools`.

**Acceptance tests:** Truncation and invalid-seek tests; path normalization and traversal
rejection; overlay precedence and history; identical manifests for identical inputs.

**Determinism:** Sorted virtual paths, explicit mount order, no host enumeration order in
output.

**Documentation:** Repository spine, ADR 0001, binary parsing and determinism invariants.

**Completion artifact:** Passing CLI integration test with two synthetic overlay trees.

### Resource provider and mod-profile refinement (complete)

**Scope:** Replace eager disk payload retention with lazy bounded resource reads and add ordered,
declarative custom-base/mod mount plans without making built-in retail filenames engine
requirements.

**Exclusions:** Package/dependency management, Workshop integration, hot reload, mod authoring,
signing, scripting, and automatic interpretation of third-party mod conventions.

**Inputs:** Synthetic loose trees, arbitrarily named synthetic BIG files, bounded mount-profile
text, explicit built-in profiles, and repeated mod paths.

**Outputs:** Stable indexed manifests, caller-bounded owned resource reads, custom total-conversion
plans, and deterministic base-then-mod provider provenance.

**Owner:** `cic-vfs` for lazy providers and `cic-tools` for profile parsing and CLI composition.

**Acceptance tests:** Disk providers remain indexable after payload deletion and fail only on lazy
read; payload and directory-index limits reject before excess allocation; malformed/oversized
profiles fail structurally; arbitrary archive names and a loose mod produce the expected winning
manifest.

**Determinism:** Mount order is explicit, optional providers retain declaration order, built-in
host filenames resolve by ASCII case with ambiguity rejection, and no filesystem enumeration order
selects a winner.

**Documentation:** ADR 0008, architecture boundaries, README profile syntax, compatibility matrix,
and changelog.

**Completion artifact:** Synthetic custom profile plus repeatable mod CLI integration test and lazy
directory/BIG provider unit tests.

## R1: BIG and CSF resource probe

**Status:** In progress.

**Scope:** Evidence-backed BIG archive mounting and complete CSF decoding with resource
provenance.

**Exclusions:** Compression not present in verified variants, localization UI, retail
fixture distribution, W3D/MAP parsing.

**Inputs:** Synthetic BIG and CSF files plus user-owned archives for local verification.

**Outputs:** Resolved VFS manifests and deterministic localization reports.

**Owner:** `cic-vfs` for BIG and new `cic-formats` for CSF.

**Acceptance tests:** Valid variants, truncation at every field, invalid counts/offsets,
duplicates, overlay conflicts, string bounds, and fuzz targets.

**Determinism:** Stable archive entry ordering, last-mounted-wins policy, stable label
ordering and diagnostics.

**Documentation:** `docs/formats/big.md`, `docs/formats/csf.md`, compatibility matrix.

**Completion artifact:** Synthetic archive containing a CSF file and a checked-in stable
manifest snapshot.

**Progress:** BIGF indexing and mounting pass the complete local suite and all 18
installed Steam Generals archives. Mixed-endian fields, slash-normalized paths, and
none/`L225`/`L231` directory trailers are verified. The bounded CSF decoder, lossless
record IR, original fixture, deterministic report, and synthetic BIG-to-CSF CLI artifact
are implemented and verified against the installed Generals CSF. A 30-second AddressSanitizer
libFuzzer smoke run completed 4,077,155 CSF inputs without a finding. BIG4 retail
verification remains open.

## R2: W3D inspection and viewer

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

## R3: Complete MAP ingestion and terrain-scene presentation

**Status:** In progress. Terrain height/blend presentation is established; water presentation is
explicitly work in progress.

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
local installed/custom-map smokes that retain no retail data.

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

**Completion artifact:** An original synthetic scene MAP spanning terrain, water, lighting,
objects, roads, player starts, sides/teams, polygon areas, and a nested but non-executed script tree;
checked stable semantic reports and deterministic capture hashes; plus local user-owned installed
and custom-map verification records with no copied game content.

### R3 gates

1. **Source lighting and water convergence (WIP).** Retain the established bounded
   `GlobalLighting` terrain/object time variants and source light order. WaterSet sky/environment
   and sibling-map inputs now resolve alongside the standing texture, diffuse tint/alpha, blend
   choice, opacity, and terrain-depth shoreline. Modern presentation adds bounded screen-space and
   authored-environment reflection inputs; continue receiving/casting shadows, anti-aliasing, and
   headless explicit-time capture hashes. Water remains a depth-tested forward pass over the
   resolved opaque scene. Its completion gate requires synthetic scalar/layout tests and repeatable
   visual comparisons against user-owned maps; the current water appearance is not a completion
   baseline.
2. **Placed-object and world metadata boundary (implemented).** Established `WorldInfo`,
   `ObjectsList`, and nested `Object` versions decode without constructing live objects. They retain finite XYZ placement,
   angle, source flags, template name, typed dictionary, waypoint fields, mirror/draw policy, and
   unknown properties under explicit limits. Emit stable reports and cross-reference diagnostics,
   but never repair, canonicalize, or execute the source data during parsing.
3. **Road and bridge presentation (regular roads, bounded joins, and intact bridges implemented).** Source-established road/bridge
   endpoint flags stage in object order. Bounded `Road` definitions now resolve regular consecutive
   pairs into source-textured, terrain-fitted strips. Connected endpoint edges form deterministic
   corner/junction polygons without the overreach of circular fillers; this remains a project
   approximation rather than a claim of source curve/tee topology or UV equivalence. Continue with
   exact curve/tee/alpha-join geometry. The bounded TerrainBridge subset now resolves the intact
   model/scale and paired endpoints through static instancing. Stage roads in stable pair/source order with source textures, width, joins,
   corner policy, and terrain fit;
   continue with non-gameplay tower scenery through the existing W3D resource path. Retain
   damaged/broken model and effect references for future simulation, but R3 neither
   selects damage states nor creates collision or repair logic.
4. **Definition resolution and complete static scene (initial instancing implemented).** The bounded
   object-definition subset selects default `W3DModelDraw` states, reskin inheritance, referenced
   models, and per-draw scale. Default W3Ds reuse the R2 material/hierarchy path, standalone mesh
   W3Ds receive a neutral renderer-only root, and placements batch stably by first model use. Ground
   placement samples the exact staged terrain triangle and adds the authored relative Z offset.
   Header3 two-sided flags now select culled or two-sided model pipelines. Continue with shadows,
   additional draw modules and source-authored ambient visual
   animation through the R2 texture-mapper and animation paths. Buildings, trees, rocks, props, bridges, decals,
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
6. **Complete script and trigger ingestion without execution (script data implemented).** Expand
   `PolygonTriggers` beyond the water-only view. Established nested `PlayerScriptsList`, `ScriptList`,
   `ScriptGroup`, `Script`, `OrCondition`, `Condition`, `ScriptAction`, and `ScriptActionFalse`
   records. Preserve names, comments, activation/difficulty flags, evaluation delays, opcode
   integers, typed parameters, source versions, and unknown values in a bounded immutable tree.
   Stable reports and optional reference diagnostics are allowed; opcode dispatch, condition
   evaluation, timers, side effects, and compatibility rewrites belong to R5.
7. **Scene integration and R3 closure.** Present all resolved opaque scenery through the existing
   G-buffer, then ordered alpha/additive scenery and forward water. Add modern shadow quality and
   bounded reflection quality after source lighting and object geometry are available. Establish
   the observed `BlendTileData` version-8 boundary from pinned source/owned observations, decide the
   version-1 height compatibility policy explicitly, and cover source-established preview/auxiliary
   MAP metadata before claiming complete variant support. Verify one dense installed map and one
   original custom scene for load closure, spawn/team/script reporting, road continuity, object
   placement, ambient animation, water quality, stable capture output, and graceful diagnostics for
   unsupported resources.

**Progress:** The initial source-backed gate inventories the `CkMp` symbol table and top-level
chunks with exact closure and opaque payload preservation. A separate semantic decoder accepts
`HeightMapData` versions 1 through 4, validates dimensions, border, boundaries, and exact row-major
sample cardinality, and retains the stored version-1 grid pending an explicit compatibility policy.
`BlendTileData` versions 6 and 7 decode bounded planes and source-ordered terrain, edge, blend, and
cliff tables. Bounded `EAR\0` RefPack decompression, Terrain INI resolution, deterministic layered
terrain capture, custom edges, legacy/modern policies, and the interactive viewer are established.
The viewer uses a persistent bounded GPU-composed virtual-texture cache with stable page tables,
LRU residency, bordered 16/32-pixel pages, compute-generated mip chains, trilinear filtering, and
anisotropy over the guaranteed 8-pixel fallback. Water-only `PolygonTriggers`, lake/river staging,
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
staging separates endpoint, scenery, hidden, waypoint, and start records. Regular Road INI
definitions now resolve first-used materials and consecutive endpoint pairs; bounded strips sample
the maximum terrain cell height at source intervals and alpha-overlay through the G-buffer in MAP
   order. Bounded endpoint-edge joins, default object draw/reskin resolution, stable static-model
   instancing, exact terrain-triangle placement, and a renderer-only playable-boundary fence are now
   integrated. Intact bridge models and source mesh culling policy are also integrated. Exact
   curve/tee UV continuity, bridge towers/states, water shadows, headless capture hashes,
   anti-aliasing, remaining object draw modules/ambient animation, blend version 8, complete polygon
   triggers, and custom-map closure remain open.

## R4: WND user interface and navigable shell

**Status:** Planned. Begins after R3 produces the complete non-simulating MAP scene and scenario
description.

**Scope:** Boundedly decode the complete source-established WND grammar and the UI definition
resources required by it, then present those values through a retained, non-gameplay UI runtime.
Cover nested layouts, exact creation rectangles, resolution scaling, status/style flags, draw and
text states, named callbacks, tooltips, focus/tab order, shell layout stacking, transition groups,
bounded post-parse WND patches, mapped images, fonts, CSF localization, cursors, and the classic
gadget vocabulary: push/radio/check buttons, vertical/horizontal sliders, scroll list boxes, entry
fields, static text, progress bars,
user windows, mouse-tracking/animated windows, tab controls/panes, and combo boxes. The interactive
artifact must render a working main menu and navigate in demo mode through the skirmish setup and
map-selection screens.

**Exclusions:** Gameplay simulation, MAP-script execution, match launch, AI, networking, account or
online services, save/replay behavior, operating-system dialogs, and arbitrary execution of callback
names from untrusted WND data. R4 does not distribute retail WND files, images, fonts, sounds,
logos, or strings. Unsupported menu actions remain disabled or produce explicit demo diagnostics.

**Inputs:** Original synthetic WND layouts and UI definitions; original synthetic images/fonts/CSF
labels; user-owned installed or modded WND, mapped-image, font, texture, CSF, transition, and menu
resources through the VFS; project-owned or mod-supplied bounded WND patches; an explicit platform
display-mode catalog; and R3 map metadata, preview images, playable boundaries, and ordered spawn
candidates for the skirmish/map-selection demo.

**Outputs:** Stable WND/UI inventories and semantic reports; immutable UI definitions; a retained
menu/gadget state tree; deterministic render-neutral UI frames and headless capture hashes; and an
interactive `wgpu` shell demo. The demo renders the user-owned main-menu composition, supports
mouse/keyboard focus and established buttons/text controls, switches layouts through a bounded menu
stack, opens skirmish options/map selection, enumerates supported maps, displays map preview and
spawn markers, edits demo player slots, and returns safely without starting simulation. Profiles
that select a 3D shell map may display its completed R3 presentation scene behind the WND overlay,
without running that map's scripts or objects as gameplay. The Options path presents modern
monitor/window-mode, resolution, refresh-rate, and UI-scale controls, applies display changes with a
bounded confirmation/rollback transaction, and persists only accepted settings.

**Owner:** `cic-formats` owns bounded WND and narrowly scoped UI INI decoding. A planned `cic-ui`
crate owns retained layout instances, control state, focus/input, safe action routing, transitions,
menu stack, and render-neutral UI frames. `cic-render` owns the `wgpu` UI backend and text/image GPU
resources. `cic-tools` composes the VFS, CSF/map data, callback registry, diagnostics, headless
captures, and interactive demo launch. No R4 layer may depend on the future simulation crate.

**Acceptance tests:** Every supported WND field and gadget receives original positive fixtures,
every-token truncation/unterminated-record tests, explicit byte/line/token/string/window/depth/list
limits, duplicate/stable-ID policy, unknown token and callback preservation, exact hierarchy
closure, and deterministic reports. UI behavior tests cover hit testing, clipping, z/order, focus,
tab traversal, hover/press/disabled/selected states, radio/check invariants, slider/list/combo bounds,
Unicode text entry, menu push/pop, transition sampling, localization fallback, resolution scaling,
and missing resources. Patch tests cover target/precondition failures, inserted/modified controls,
stable overlay order, provenance, and source immutability. Display-setting tests inject synthetic
monitor/video-mode catalogs and cover stable filtering, deduplication, dependent resolution/refresh
choices, windowed/borderless/exclusive behavior, apply/confirm, timeout rollback, and persistence.
Renderer tests use explicit viewport/DPI/time/input sequences and checked synthetic hashes.
Installed smoke tests retain no retail output.

**Determinism:** WND file and child order control hierarchy, hit testing, focus order, and draw
submission. Stable IDs derive from decorated source names plus deterministic duplicate diagnostics,
never host hashes. VFS mount order controls definitions and assets. Captures specify viewport,
scale policy, locale, font set, transition time, cursor position, focus, input events, selected map,
demo slot values, and a complete display-mode catalog. Mode lists sort deterministically by monitor
key, width, height, refresh millihertz, bit depth, and stable source index. Host DPI, monitor
enumeration order, filesystem order, locale, wall clock, and platform font discovery cannot silently
affect diagnostic output.

**Documentation:** `docs/formats/wnd.md`, `docs/provenance/wnd.md`, ADR 0010, architecture and
compatibility updates, synthetic UI authoring instructions, and user-owned capture guidance. Every
implemented UI family records source revision/notices, exact limits, unsupported fields, resource
fallbacks, input behavior, and completion evidence.

**Completion artifact:** An original synthetic multi-layout WND suite using every established
gadget family, mapped images, Unicode text, callbacks-as-data, focus navigation, and transitions;
checked inventory/semantic reports and headless hashes; plus local user-owned verification that the
main menu renders and can navigate Main Menu -> Options -> display-mode apply/confirm or rollback ->
Main Menu -> Skirmish Options -> Map Select -> Skirmish Options -> Main Menu with map preview/spawn
markers and no simulation launch.

### R4 architecture decision

R4 uses a project-owned retained WND model and a custom UI renderer on the existing `wgpu`/`winit`
stack. Full GUI toolkits are not the compatibility boundary: egui is immediate-mode, while iced
introduces a separate widget/layout/application model. Either would require a lossy translation of
WND rectangles, hierarchy, state images, focus, callbacks, and shell transitions. Focused libraries
remain appropriate below the compatibility layer: prefer `cosmic-text` for Unicode shaping/layout
and `glyphon` for `wgpu` glyph-atlas rendering after verifying compatibility with the workspace's
selected `wgpu`; fall back to a small project-owned glyph upload backend over `cosmic-text` rather
than changing WND semantics. Modern controls absent from a retail or modded layout are introduced by
a versioned declarative WND patch applied after parsing and before retained-state instantiation; no
source WND bytes are edited and no renderer path searches for special window names.

### R4 implementation gates

1. **WND inventory and bounded syntax.** Specify file versions, `STARTLAYOUTBLOCK`, layout
   init/update/shutdown names, nested `WINDOW`/`CHILD` blocks, creation resolution/rectangles,
   defaults, fields, `DATA`, and exact `END` closure. Preserve callback names and unknown tokens as
   data; never resolve a WND string to a native function pointer in the parser.
2. **Immutable control definitions.** Decode all established status/style names, fonts, text and
   tooltip labels, state colors/borders, image offsets, draw-data arrays, header templates, and
   gadget-specific records. Apply explicit limits to every nesting and variable-length surface.
   Stable reports must be sufficient to compare a modded WND without rendering it.
3. **Bounded WND patch overlays.** Define a versioned project-owned patch format targeting one WND
   virtual path and exact decorated window names. Support explicit preconditions, known-field
   replacement, hide/show/enable defaults, reparent/reorder where safe, and insertion of complete
   project-owned window subtrees. Apply patches in VFS/profile then file-operation order to produce
   a new immutable definition with per-field provenance; preserve the source document unchanged.
   Missing required targets, duplicate inserted IDs, cycles, limit excess, and invalid gadget data
   are structured errors. Version 1 has no wildcards, arbitrary callbacks, or imperative code.
4. **UI resource resolution.** Add bounded mapped-image, font/language, transition/scheme, cursor,
   and required menu-definition subsets. Resolve CSF labels through the existing localization
   decoder and images/fonts through the VFS. Missing resources use visible placeholders and stable
   diagnostics; system-font fallback is opt-in and never used by deterministic captures.
5. **Retained UI runtime.** Instantiate immutable definitions into an isolated menu state tree with
   show/hide/enable, parent-relative layout, classic/modern resolution policies, clipping, z-order,
   hit testing, capture, focus, tab order, hover, press, selection, text editing, scrolling, and
   control-specific invariants. UI state is presentation state, not simulation state.
6. **Custom `wgpu` presentation.** Render ordered colored/image quads, borders, state overlays,
   scissor rectangles, cursors, and shaped Unicode text over either a 2D background or an R3 scene.
   Support source alpha and explicit color-space handling, bounded atlases, batched stable draws,
   explicit transition time, and surface-free deterministic capture.
7. **Safe callbacks, shell stack, and transitions.** Retain source system/input/draw/tooltip and
   layout callback names, then route only allowlisted demo actions through typed events. Implement
   push/pop/bring-forward/hide semantics and established transition groups without invoking MAP
   scripts or arbitrary symbols. Unknown callbacks remain reportable and inert.
8. **Working main-menu artifact.** Load the user-owned `Menus/MainMenu.wnd`, mapped images, fonts,
   and CSF labels; render its original controls and text; support hover/focus/click, established
   subpanels, Back, Options, Skirmish navigation, and safe Exit. When configured,
   compose the R3-rendered shell MAP as a non-simulating 3D background beneath the UI. No retail
   capture is checked in.
9. **Modern Options/display settings.** Load `Menus/OptionsMenu.wnd`, reuse its established
   `ComboBoxResolution`, and apply a bounded project patch that adds missing monitor, window-mode,
   refresh-rate, and UI-scale labels/controls without changing user-owned bytes. Enumerate platform
   modes into a stable catalog. Windowed and borderless use explicit desktop/presentation refresh
   semantics; exclusive fullscreen selects an advertised resolution/refresh pair. Apply through
   `winit`/surface reconfiguration, show a timed confirmation dialog, roll back on timeout/failure,
   and persist only confirmed project-owned preferences. Deterministic tests inject the catalog and
   explicit confirmation time rather than reading host monitors or a clock.
10. **Skirmish and map-selection compatibility harness.** Load the user-owned skirmish and map-select
   WND layouts. Bind R3's deterministic map catalog, display name, preview/minimap, playable bounds,
   and `Player_n_Start` candidates. Support demo player-name entry, open/closed/AI slot choices,
   color/faction/team combos, start-position selection, map switching, Back, and a non-executing
   Start validation result. This UI must expose unsupported MAP versions/resources visibly instead
   of hiding incompatible maps.
11. **R4 closure.** Inventory every user-owned WND in the selected profile under parser limits,
   exercise all control families and patch operations synthetically, verify the complete main-menu,
   settings, and skirmish navigation loop at multiple aspect ratios/refresh catalogs, and document
   fields/callbacks that remain retained-but-inert until R5 or later.

## R5: Deterministic simulation kernel

Consume R3's immutable scenario description to introduce fixed 30 Hz ticks, stable runtime IDs,
versioned seeded RNG streams, ordered scheduling, command recording, replay, and subsystem state
hashes. R5 owns player/team activation, spawn assignment, live-object construction, script opcode
dispatch, conditions, actions, timers, and all mutation implied by MAP data. Script support begins
from the raw versioned R3 tree; unsupported actions fail or remain inert deterministically rather
than being guessed. R4 UI may submit typed commands and display immutable snapshots but cannot
execute scripts or own authoritative objects.

## R6: Navigation analysis and gameplay slice

Derive terrain and locomotor-aware regions, portals, choke points, and dynamic obstacles,
then complete one build-harvest-combat loop using normal player commands through the R4 UI.
