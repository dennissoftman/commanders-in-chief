# ADR 0009: Complete MAP Scene and Script-Data Boundary

- Status: Accepted
- Date: 2026-07-22
- R3 closure recorded: 2026-07-23

## Context

R3 began as a terrain-height and blend presentation gate. A useful MAP viewer also needs the
scenario data that gives the terrain meaning: source lighting and water appearance, roads and
bridges, buildings and foliage, waypoints and player starts, sides and teams, polygon areas, and
map scripts. Treating these as unrelated later features would leave the MAP format half decoded and
would force future simulation work to rediscover persistence rules inside runtime systems.

The persisted data must nevertheless remain separate from behavior. Loading a MAP must not create
authoritative objects, choose players, execute a condition, start a timer, mutate a team, or invoke
a renderer. Presentation also needs external INI definitions and W3D resources, but format decoders
cannot own VFS or GPU resources.

Pinned GeneralsGameCode revision `9f7abb866f5afd446db14149979e744c7216baaf` establishes that
WorldBuilder writes `WorldInfo`, `ObjectsList`/`Object`, `PolygonTriggers`, `GlobalLighting`, and
`SidesList`; object flags identify road and bridge endpoints; waypoint properties include the
one-based `Player_n_Start` convention; `SidesList` versions contain sides, build lists, teams, and a
nested `PlayerScriptsList`; and scripts form a nested versioned chunk graph. Exact source paths,
notices, and permanent links are recorded in `docs/provenance/map.md`.

## Decision

- R3 covers every source-established MAP section needed for lossless inspection and complete
  pre-simulation presentation. It may add narrowly scoped TerrainRoad/TerrainBridge, Object draw,
  lighting, and water INI decoders when those definitions are required to resolve the map scene.
- `cic-formats` returns immutable, renderer-neutral values. Known fields are decoded only for
  established versions; unknown chunks, dictionary entries, property types, flags, enum/opcode
  integers, and unresolved names remain preserved or explicitly reported.
- The generic MAP inventory remains top-level and unknown-preserving. Each nested semantic family
  receives a separate label/version-aware decoder with exact closure and explicit count, string,
  recursion, allocation, and expansion limits.
- A non-executing scenario description cross-references placements, waypoints, player-start
  candidates, sides, teams, build lists, polygon areas, and scripts using stable source-order IDs.
  Cross-reference diagnostics never repair or rewrite persisted data.
- Road and bridge presentation is derived from immutable object endpoints plus bounded road/bridge
  definitions. R3 stages road surfaces, joins, corners, terrain fit, intact bridge geometry, and
  non-gameplay scenery. Collision, damage states, repair, tower behavior, and state transitions are
  retained as references where available but are not activated.
- The presentation resolver selects only initial drawable states and assets. It reuses the R2 W3D
  material, hierarchy, animation, mapper, and texture paths for buildings, trees, rocks, props,
  bridges, decals, and other placed geometry. The initial implemented subset accepts default
  `W3DModelDraw` models, reskin ancestry, and per-draw scale. Validated standalone mesh W3Ds receive
  a neutral renderer-only identity root, and supported placements batch stably by first model use.
  Existing definitions with no default visual draw are treated as non-visual data; missing or
  malformed drawable resources produce stable diagnostics. Visible placeholders remain future
  presentation work.
- Grounded static placement adds authored relative Z to the exact staged terrain triangle at the
  placement XY coordinate verbatim, including negative offsets, border offset, and diagonal
  choice. It does not clamp or add a renderer epsilon, and it does not use a
  whole-cell maximum, which can float objects on slopes, and it preserves deliberately stacked or
  elevated placements.
- Static mesh backface policy comes from the decoded W3D Header3 two-sided flag; the renderer
  keeps explicit culled and two-sided pipelines rather than applying one global policy.
- Road presentation uses a bounded immutable topology pass before terrain tessellation. It groups
  exact shared endpoints by road material, trims approaches, selects source-radius 30-degree
  curves or miters, and inserts source-atlas tee, Y, slanted-tee, and four-way meshes. Different
  materials never share a generic junction fill; only authored open `ROAD_JOIN` endpoints receive
  the source-atlas cross-material alpha cap.
- Road textures retain the source three-level mip budget rather than allowing the complete atlas to
  collapse into a distant average. The source terrain lift remains part of immutable staging; an
  additional renderer-only depth bias is presentation policy and does not alter persisted or staged
  world coordinates. Optional polygon-line wireframe is likewise a diagnostic renderer mode.
- The primary playable boundary may be visualized by a renderer-only translucent fence whose base
  follows terrain and whose common top clears the MAP's greatest height. It conveys the persisted
  extent but does not create collision, navigation, or simulation reachability.
- Source-authored ambient visual animation, including vegetation waving, W3D clips, texture
  mappers, and animated textures, is presentation state sampled from explicit time. It cannot read
  or mutate simulation state and is not authoritative. `W3DTreeDraw` owns tree resources and
  interaction/topple fields, while global `BreezeInfo` owns ordinary sway. R3 may use the
  source-default breeze as an explicit `ZeroHourLegacy` presentation input; a decoded
  `SET_TREE_SWAY` action remains inert until R5 executes scripts.
- `GlobalLighting` supplies separate immutable terrain/object lighting inputs. Water remains a
  forward transmissive pass; it shares the primary directional shadow map with opaque scenery and
  is followed by bounded edge-aware post-process anti-aliasing.
- `HeightMapData` version 1 remains an immutable native stored grid in both presentation profiles.
  The parser does not hide a legacy downsampling transform. A future historically exact derived
  view must be an explicit versioned compatibility policy and cannot replace the retained samples.
- Source-editor preview and auxiliary chunks that are not needed to construct the pre-simulation
  scene remain available through the opaque MAP inventory. R4 map selection derives its preview
  from deterministic `map-render` output; it neither decodes nor redistributes cached retail
  thumbnails.
- Headless scene completion uses a deterministic fixed-isometric overview. Terrain is the existing
  GPU capture; roads, water, and placement markers are composited in authoritative source order,
  and tree markers sample explicit-time sway. The interactive viewer remains the detailed visual
  reference; the overview is an integration and regression artifact, not a pixel-equivalent
  replacement for it.
- R3 decodes the complete established script tree, including groups, scripts, OR/AND conditions,
  true/false actions, typed parameters, comments, activation/difficulty flags, and delays. It does
  not dispatch opcodes, evaluate conditions, schedule timers, or apply side effects.
- R5 is the sole owner of runtime activation: fixed ticks, player/team assignment, spawn selection,
  live objects, script execution, commands, RNG, replay, and state hashes. It consumes R3 values
  rather than reparsing MAP bytes.

## Acceptance and determinism

Each added MAP family requires synthetic positive and negative fixtures, every-field truncation,
limit checks, stable reports, established-version dispatch, and exact nested closure. Scenario IDs,
cross-references, roads, object instances, and script nodes retain file order. Definition overrides
use explicit VFS mount order. Diagnostic captures use explicit camera, animation time, time-of-day,
and seed inputs. No parser or report may depend on randomized map iteration, locale, host time, or
filesystem enumeration order.

R3 completion is represented by the original synthetic fixture family and pinned executable matrix
covering terrain, water, lighting, roads, objects, player starts, sides/teams, polygon areas, and
nested scripts, plus the deterministic full-scene overview. User-owned installed smokes retain
only aggregate counts and hashes and never retain captures or retail data.

## Consequences

- R3 is larger than a terrain renderer, but it becomes the single persistence and presentation
  milestone for MAP files instead of leaking MAP parsing into simulation work.
- The map viewer can become visually representative before gameplay exists, including intact
  roads, buildings, foliage animation, static props, source lighting, and improved water.
- Scripts, team metadata, and starts become inspectable early without creating a hidden partial
  simulation.
- Some object INI fields and bridge variants will remain retained-but-unapplied until their runtime
  systems exist. That is preferable to guessing or constructing gameplay modules in the viewer.
- R5 receives a bounded, immutable scenario input and can focus on deterministic execution rather
  than file-format discovery.

## Rejected alternatives

- **Defer all objects and scripts until simulation:** rejected because it leaves MAP ingestion
  incomplete, prevents a representative viewer, and couples persistence discovery to runtime code.
- **Execute harmless-looking scripts in the viewer:** rejected because conditions, timers, and
  actions create authoritative mutation and nondeterministic ownership ambiguity.
- **Build a universal INI engine before object presentation:** rejected. R3 adds bounded semantic
  subsets demanded by map presentation and preserves unsupported declarations for later gates.
- **Treat roads as ordinary terrain blend textures:** rejected because source road/bridge endpoints
  are object records resolved through dedicated definitions and geometry policies.
