# MAP Container, Semantic Gates, and Scene-Presentation Plan

- Status: container, terrain/water, world/object, and sides/script data boundaries implemented;
  complete R3 MAP semantics and scene presentation planned by ADR 0009; water presentation WIP
- Owning crate: `cic-formats`
- Last updated: 2026-07-22

## Evidence

The `DataChunk` reader and writer in TheSuperHackers/GeneralsGameCode revision
`9f7abb866f5afd446db14149979e744c7216baaf` establish the `CkMp` signature, symbol table,
ten-byte chunk header, native little-endian fields, payload-length meaning, and nested-parser
behavior. The cached input stream, compression manager, and RefPack decoder establish the
`EAR\0` wrapper used by installed MAP resources. `MapReaderWriterInfo.h`, `WorldHeightMap.cpp`,
`WorldHeightMap.h`, `TileData.h`, and the WorldBuilder writer establish `HeightMapData` versions 1
through 4 and `BlendTileData` versions 6 and 7. The WorldBuilder writer, `MapUtil`, `MapObject`,
`SidesList`, script, terrain-road, and global-light sources additionally establish the planned R3
object, road/bridge, waypoint/start, side/team, script-tree, and lighting boundaries. Exact source
and licensing details are in `docs/provenance/map.md`.

This first R3 gate is verified with original synthetic data and one user-owned installed MAP. No
retail map bytes or names are included.

## Optional compression wrapper

Installed MAP members may wrap the complete `CkMp` stream in `EAR\0` RefPack compression:

| Size | Field |
|---:|---|
| 4 | ASCII signature `EAR\0` |
| 4 | signed decompressed byte length |
| variable | RefPack stream |

The decompressed length is limited before allocation and must match the RefPack stream header and
the exact number of emitted bytes. Every literal and back-reference read is bounded, references
cannot precede decoded output, commands cannot exceed the declared output, an end marker is
required, and trailing compressed bytes are rejected. Bare `CkMp` inputs remain supported for
synthetic and custom tools.

## File layout

All multi-byte values are little-endian. Signed source fields are rejected when negative before
conversion or allocation.

| Size | Field |
|---:|---|
| 4 | ASCII signature `CkMp` |
| 4 | signed symbol count |
| variable | symbol-table entries |
| variable | top-level chunks through end of file |

Each symbol-table entry is:

| Size | Field |
|---:|---|
| 1 | name byte length |
| variable | name bytes without a terminator |
| 4 | numeric identifier |

The table may map the same identifier more than once. The legacy reader prepends entries while
loading them, so the last entry in file order wins lookup. The inventory preserves every table
entry and applies that resolution rule deterministically.

Each top-level chunk is:

| Size | Field |
|---:|---|
| 4 | symbol-table identifier |
| 2 | version |
| 4 | signed payload byte length |
| variable | payload bytes |

Unlike W3D, the MAP header has no generic container flag. Known callbacks decide whether a payload
contains fields or a nested chunk stream. The generic inventory therefore parses only the
top-level stream and preserves every payload as opaque bytes. Semantic decoders may open only
explicitly recognized labels and versions. This avoids guessing that arbitrary unknown data is a
child stream.

## `HeightMapData`

The height payload begins with signed 32-bit width and height values. Samples are row-major bytes,
and the declared sample count must equal `width * height` exactly.

| Version | Additional fields before sample count | Source cell size |
|---:|---|---:|
| 1 | none | 5 world units |
| 2 | none | 10 world units |
| 3 | signed border size | 10 world units |
| 4 | border size, boundary count, then signed `(x, y)` pairs | 10 world units |

Versions 1 through 3 expose one derived boundary of
`(width - 2 * border, height - 2 * border)`. Version 4 preserves signed boundary pairs in file
order, including negative coordinates; they are metadata rather than allocation dimensions.
Dimensions and borders remain nonnegative, dimensions must be positive, the doubled border must fit
both dimensions, counts are checked before allocation, and no trailing payload bytes are accepted.

The semantic value retains the stored version-1 grid exactly. The legacy engine contains a
version-1 compatibility downsampling step in some loading paths; that transform is deliberately
deferred until runtime observations establish which compatibility policy each consumer needs. The
opaque inventory still preserves the complete original payload.

## `BlendTileData` versions 6 and 7

The blend payload is interpreted only after `HeightMapData`, because its signed cell count must
equal the validated height sample count. Four row-major signed 16-bit planes follow: tile, blend,
extra-blend, and cliff-info indices. Signed values are retained without speculative correction.

Version 6 stores no cliff bitmap. It derives each non-boundary cell from the minimum and maximum of
its four neighboring height samples. The source rule is a range greater than 9.8 world units;
stored heights scale by 0.625, making the exact byte-height threshold 16. The last row and column
remain clear. Version 7 instead stores a bitmap with legacy row stride `(width + 1) / 8`. The
decoder copies each stored row into a zeroed conventional `ceil(width / 8)` stride, so unavailable
right-edge bits are deterministically clear. Bit `x % 8` of byte `x / 8` identifies a cliff cell.

The remaining fields are source-ordered tables:

- bitmap-tile, blended-tile, cliff-info, terrain-texture-class, edge-tile, and
  edge-texture-class counts;
- terrain classes with first tile, count, width, retained legacy integer, and bounded opaque name;
- edge classes with first tile, count, width, and bounded opaque name;
- blend entries 1 through `blended_tile_count - 1`, each containing one signed blend index, six
  byte selectors, one signed custom edge-class index, and the exact `0x7ADA0000` sentinel; and
- cliff entries 1 through `cliff_info_count - 1`, each containing one signed tile index, eight
  finite IEEE-754 UV values, and byte-valued flip and mutant flags.

Counts and nonnegative tile ranges are checked before allocation, texture ranges must stay within
their declared tile tables, UVs must be finite, blend sentinels must match, and trailing bytes are
rejected. Selector and cliff flag bytes are preserved as raw source values. The semantic value
remains immutable and renderer-neutral. A cliff-info count of zero is source-compatible: the raw
zero is retained and the table has no explicit entries, matching the source reader's bounded
`1..count` loop rather than inventing an implicit record.

## Default limits

- 512 MiB complete input, decompressed stream, and per-chunk payload
- 4,096 symbols and 255 bytes per symbol name
- 1,000,000 top-level chunks
- 16,384 samples on either height-field axis
- 16,777,216 total height samples
- 4,096 playable boundaries
- 2,047 bitmap tiles and 2,047 edge tiles
- 16,192 blended-tile entries and 32,384 cliff-info entries
- 256 terrain or edge texture classes and 1,024 bytes per texture-class name
- 65,536 polygon triggers and points per trigger, 1,000,000 retained water points, and 1,024 bytes
  per trigger name

Every complete chunk closes at its declared boundary. A suffix shorter than the ten-byte header,
negative length, truncated payload, invalid semantic count, or limit excess returns a structured
error.

## Diagnostic reports

`cic-inspect map <virtual-path> <mount>...` reports symbols and top-level chunks in file order,
including absolute offsets, identifiers, versions, payload lengths, resolved names, and unknown
identifiers. `cic-inspect map-height` reports versioned dimensions, border, boundaries, and every
height sample in stable row-major order when passed `--report`. By default it writes those exact
samples as an 8-bit grayscale PNG named from the MAP basename, without an sRGB or gamma
declaration; `--png <output.png>` overrides the destination.
`cic-inspect map-blend` reports every cell and source-ordered texture, blend, and cliff record;
floating-point UVs use exact hexadecimal bit patterns.
`cic-inspect map-water` reports only water-flagged polygon records and their integer points in
stable source order; non-water trigger semantics remain undecoded.

Version-3 river points describe a perimeter rather than adjacent bank pairs. `river_start` marks
the seam: staging starts on the two points around that seam, advances along one bank, retreats
along the other with wraparound, and emits paired cross-sections. A negative seam or one at/past
the final point is retained by the format model but safely produces no renderer geometry.

`cic-inspect map-render` additionally decodes ordered `Terrain`/`Texture` declarations from every
provider version of the mounted default and edition Terrain INIs in stable base-to-overlay order,
applies `DefaultTerrain` inheritance, resolves sheets beneath `Art/Terrain`, stages source-scaled
geometry and base/primary/extra layers, and writes an sRGB headless PNG. Size and power-of-two
pixels per cell are explicit; diagnostics include stable
geometry/layer counts and the captured RGBA SHA-256. `--terrain-policy legacy` is the default and
applies stored cliff UVs plus the bounded steep-slope retile; `modern` retains stored mappings but
skips the implicit steep-slope adjustment. Custom edges use a separate deterministic index/atlas
pass. `cic-inspect map-view` launches the same staged terrain in a perspective free-flight viewer.
The viewer overlays independent depth-capped 16- and 32-texel screen-space regions on the stable
8-texel background. Oblique horizon coverage cannot lower the nested foreground tiers. It
uses a hybrid-deferred terrain G-buffer and lighting resolve, followed by a depth-aware forward
water pass. The default legacy policy resolves the selected standing texture, diffuse tint/alpha,
blend choice, minimum opacity, and opaque depth; terrain depth feathers its shoreline. The explicit
Modern policy retains the refractive presentation. Complete caller-supplied caustic frame sequences
are mipmapped and sampled as a world-projected texture array. Obsolete detail bakes are cancelled
off-thread without request throttling;
only the newest complete linear-light mip chain reaches upload, resident replacements overlap in
explicit presentation time, and patches are sampled trilinearly with supported
anisotropy. `modern`
additionally applies deterministic world-anchored macro variation without rotating authored
tiles. These are renderer-authored presentation policies, not decoded MAP lighting or
translations of the legacy water renderer.

The Water INI boundary accepts all source-established `WaterSet MORNING|AFTERNOON|EVENING|NIGHT`
fields: sky/water texture names, four vertex colors, diffuse and transparent diffuse colors,
U/V scroll per millisecond, sky texel density, and water repeat count. It also accepts the complete
`WaterTransparency` table: depth/minimum opacity, standing/radar colors, standing texture,
additive policy, and five skybox textures. Input bytes, lines, line length, definition count, string
length, scalar magnitude, texture repeat, color channels, block nesting, and exact closure have
explicit limits. Repeated blocks accumulate stable last-field-wins values, matching partial
definition overlays without consulting the VFS or renderer. Integer RGBA values require RGB in
order, accept an optional alpha component, and default omitted alpha to 255 as the source parser
does. Transparency standing/radar colors require ordered integer RGB channels from 0 through 255
and are normalized once into immutable renderer-neutral values. Installed-profile tools seed the
source constructor defaults, apply all shadowed global INI providers from earliest to latest, and
then apply the sibling `Map.ini`; the parser itself remains VFS-independent.

## R3 semantic gates

The sections below define the implemented and remaining semantic gates. Every decoder keeps the
generic inventory's opaque payload and adds a separate label/version-aware immutable view. No
semantic decoder creates renderer or simulation resources.

### Terrain-version and auxiliary metadata closure

R3 completion includes source-backed research and a bounded decoder for the observed
`BlendTileData` version 8 rather than treating the Zero Hour variant as permanently outside the
terrain milestone. It also requires an explicit profile policy for version-1 height resampling and
established semantic views for presentation/inspection metadata such as map preview data and any
remaining WorldBuilder auxiliary chunk used by supported maps. Unknown or unobserved versions still
remain opaque; compatibility claims are version-specific and no nearby layout is inferred.

Custom-map fixtures must cover omitted optional sections, reordered known chunks, unknown chunks,
missing resource definitions, and profile overrides so R3 does not accidentally require retail
archive names or a single WorldBuilder output shape.

### `WorldInfo`, `ObjectsList`, and `Object`

The pinned reader/writer establishes a nested `ObjectsList` containing source-ordered `Object`
records. Established fields include finite XYZ location, finite angle, integer flags, object or
template name, and a typed property dictionary in later versions. The WorldBuilder version-3 writer
also persists waypoint ID/name fields for waypoint objects. `WorldInfo` carries a typed world
dictionary and receives its own bounded view rather than being folded into global state.

The implemented value retains source version, stable placement ID, exact float bits, flags, raw
name, typed dictionary entries, waypoint fields, and unknown nested chunks. Duplicate names remain
distinct. The decoder checks nested closure, field finiteness, dictionary type/length/count limits,
and total placement allocation before returning. It does not resolve templates, normalize
ownership, repair teams, or instantiate objects. `cic-inspect map-objects` emits the stable view.

### Roads and bridges

Roads are not merely painted `BlendTileData`. `MapObject` flags identify first/second road and
bridge endpoints plus corner/join policy. TerrainRoad/TerrainBridge INI definitions provide the
presentation resource data: road texture and width, or bridge model/scale plus state variants and
tower references.

R3 preserves endpoint records in object order, diagnoses records with ambiguous road/bridge roles,
and derives separate immutable road/bridge endpoint lists. A bounded road INI decoder retains
`Road` texture/width fields and intact `BridgeModelName`/`BridgeScale` fields under explicit
file/line/count/string limits. Omitted intact bridge fields inherit from the source-order
`DefaultBridge` visible when that declaration is loaded.
Provider and sibling-map declarations resolve in stable order; only definitions referenced by MAP
Point1 records load textures.

The renderer pairs a Point1 only with the immediately following Point2, matching the established
map-object walk. It tessellates the regular strip at terrain-cell intervals, samples bounded points
across its physical width, places each column above the maximum supporting height, applies the
established regular-sheet UV scale, and submits alpha overlays in MAP order. Missing pairs,
definitions, textures, invalid widths, and zero-length segments remain stable diagnostics. Corner,
tight-corner, and join flags now group connected endpoints into deterministic polygons built from
the physical strip edges. This bounded project-authored fill avoids oversized circular overreach,
but does not claim the source curve/tee topology or texture mapping. Exact curve/tee/alpha-join
insertion remains open. Consecutive bridge endpoints now use the source-specific endpoint height
of terrain plus `0.25`; bridge endpoint marker Z is not an authored scenery offset. The renderer
selects the configured model's `BRIDGE_LEFT`, optional `BRIDGE_SPAN`, and optional `BRIDGE_RIGHT`
subobjects, rounds the repeat count from the available length, and deforms their X/Y/Z basis onto
the sloped endpoint axis while applying `BridgeScale` across the section. Ordinary scenery still
uses terrain plus authored Z verbatim. Damaged/broken resources, tower names, effect
references, and repair data remain unapplied; damage, repair, sound, effects, collision, and state
transitions are simulation concerns.

### Waypoints, player starts, sides, teams, and build lists

Waypoint objects are ordinary persisted object records with waypoint properties. The established
map-info reader recognizes one-based `Player_1_Start`, `Player_2_Start`, and subsequent names as
start positions. R3 exposes these as ordered spawn candidates without choosing slots or creating
players.

`SidesList` versions 1 through 3 now decode as a separate established gate. Source evidence shows
ordered side dictionaries and build lists; version 2 adds team dictionaries; version 3 adds
build-list script/health/behavior fields; and the chunk nests `PlayerScriptsList`. The immutable
scenario view retains side/team ownership and alliance names, initial/build-list placements, and
script-list associations. Cross-reference reports may flag duplicate names, missing owners,
missing start waypoints, or dangling teams, but parsing never performs the legacy reader's repair or
validation mutations. `cic-inspect map-sides` emits the complete immutable side/team/build/script
view in source order.

Team definitions and spawn candidates are related but distinct. A start waypoint describes a
possible map position; side/team dictionaries describe scenario identity and ownership. R4 will
display these values in the non-simulating skirmish UI; R5 will decide controller assignment, spawn
selection, runtime teams, and initial live objects.

### Polygon areas and scripts

The current decoder projects only water/river records from `PolygonTriggers`. R3 will add a complete
immutable polygon-area view for established versions while retaining the existing water projection.
General trigger names, IDs, points, and flags become inspectable and cross-referenceable, but they
do not register callbacks or spatial gameplay queries.

The source-established script chunk graph is:

```text
PlayerScriptsList
  ScriptList
    ScriptGroup
      Script
        OrCondition
          Condition
        ScriptAction
        ScriptActionFalse
```

Decoded scripts retain names, comments, active/one-shot/subroutine flags, difficulty flags, evaluation
delay, source versions, integer condition/action opcodes, and ordered typed parameters. Conditions
preserve OR groups and source-ordered AND chains. Unknown opcode and parameter values remain data.
Default limits must independently bound player lists, groups, scripts, conditions, actions,
parameters, strings, and nested depth; malformed or excessive trees return structured errors.

R3 reports this tree through `map-sides` and may diagnose unresolved object, waypoint, side, team,
or script names. It
does not consult live opcode templates, apply implicit compatibility rewrites, evaluate a condition,
schedule a delay, or execute either action branch. All dispatch and mutation belong to R5.

### `GlobalLighting` and water appearance (WIP presentation)

The decoder accepts source-established `GlobalLighting` versions 1 through 3. Every version starts
with a one-based selected time and four ordered morning/afternoon/evening/night records. Version 1
stores one terrain sun and one object sun per time; version 2 adds two object accents; version 3
adds two terrain accents. Each light is nine finite little-endian floats: ambient RGB, diffuse RGB,
and a source direction vector. A final packed shadow color is optional, as established by the
reader. Payloads close exactly, duplicate chunks fail, and invalid selected times, non-finite
components, unsupported versions, truncation, and trailing bytes are structured errors.

`cic-inspect map-lighting` emits selected time, optional shadow color, and every light as exact
float bits in stable time/set/source order. `map-view` copies the selected terrain sun/accents into
renderer-owned values and evaluates all three; only maps with no lighting chunk use the documented
project preview fallback. The selected `WaterSet` diffuse color/alpha and U/V scroll plus
`WaterTransparency` standing texture/color/blend/opacity inputs cross the renderer boundary. The
standing, WaterSet sky, and WaterSet environment textures resolve through the VFS after ordered
global and sibling `Map.ini` overrides.

Water is not considered visually complete. The Modern policy now combines authored sky/environment
inputs with a bounded screen-space reflection, and `map-view --time` freezes presentation time for
repeatable interactive comparison. Remaining work includes shadows received by water and cast onto
its bed where appropriate, anti-aliasing, headless explicit-time capture hashes, and quality
validation. Water stays outside the opaque G-buffer in an ordered forward pass. Completion requires
repeatable synthetic captures and user-owned visual comparisons; legacy compatibility and Modern
presentation remain explicit separate policies.

### Object-definition and static-scene resolution

Placed records name definitions; they do not directly contain complete draw resources. A bounded
object-definition subset now decodes top-level `Object` and `ObjectReskin` blocks, default-condition
`W3DModelDraw` models, and per-draw scale under explicit byte/line/name/module/model limits. Indented
gameplay modules and unsupported draw modules are ignored as data and never instantiated by the
viewer. Reskin lookup is ancestry-bounded and cycle-safe.

The resolved scene batches supported buildings, trees, rocks, and props by first model use while
retaining placement IDs and source order within each instance buffer. Existing R2 W3D hierarchy,
material, mapper, and texture paths are reused. A validated W3D that contains only top-level mesh
chunks receives a neutral renderer-only identity root/HLOD wrapper; this does not infer animation
or behavior. A ground placement samples the exact staged terrain triangle at its XY coordinate,
including the height-field border and chosen diagonal, then adds the MAP-authored relative Z
offset. This prevents underground placement without lifting objects to an unrelated uphill cell
corner or collapsing deliberately stacked scenery onto one plane. The offset is used verbatim,
including negative values; it is neither clamped nor given a renderer epsilon. Definitions that exist but have no default W3D draw, such as non-visual markers, are
excluded without an error; missing definitions/models and malformed resources remain stable
diagnostics. W3D Header3's two-sided mesh flag selects between back-face-culled and two-sided
static pipelines. The map viewer's explicit legacy-preview composition policy may skip a missing
optional HLOD mesh, root a one-past-end HLOD/skin reference rigidly, and replace a non-finite UV
with zero only at the renderer/export boundary; strict W3D composition still rejects damaged
hierarchy references and exact UV bits remain available as immutable metadata.

Tree movement spans two data owners. `W3DTreeDraw` INI supplies the tree model/texture and
push-aside/topple presentation parameters. Normal ambient sway instead reads global `BreezeInfo`:
the legacy constructor provides a default direction, intensity, lean, period, and randomness, and
the map script action `SET_TREE_SWAY` can change those values. R3 may sample the documented default
breeze as explicit-time presentation, but it will not execute `SET_TREE_SWAY`; custom scripted wind
remains decoded data until R5 owns script execution. Shadows, bridge towers/states,
decals/scorches, static lights, and explicit-time vegetation sway remain open.

### Playable-boundary presentation

The first positive signed height-field boundary is presented as a renderer-only translucent fence.
Its base follows each perimeter terrain sample and every segment shares a top above the greatest
height sample in the MAP, so high cliffs cannot protrude through it. The fence visualizes the
playable extent only: it does not create collision, pathing, or simulation state.

### Reports and completion order

- `map-objects` reports world/object records, flags, typed properties, waypoints, road/bridge
  endpoint roles, player starts, and stable placement IDs.
- `map-sides` reports sides, teams, build lists, and the complete nested script tree with versions,
  raw opcodes, typed parameters, and no execution.
- A future scenario report will add complete polygon areas and cross-reference diagnostics without
  runtime validation or repair.
- `map-view` integrates source lighting and WIP water, regular roads with bounded joins, initial
  instanced static drawables, intact bridges, and the playable-boundary fence. Remaining draw modules,
  explicit-time ambient animation, shadows, and reflection closure remain open.

Lighting/water inputs, object/world decoding, endpoint staging, sides/teams/spawns, and nested script
data are implemented, as are regular roads/bounded joins and initial object-definition/static-scene
instancing. Remaining order is exact curve/tee continuity and bridge towers/states, additional object draw and
ambient-animation coverage, complete polygons, then integrated scene closure. Each step adds its
own synthetic fixture, negative tests, stable report, documentation, and completion artifact.

## Synthetic fixture

`crates/cic-formats/tests/fixtures/minimal.map.hex` is original project data. It contains a
version-4 three-by-two height field, one known opaque chunk, and one identifier absent from the
symbol table. Unit tests cover every established height version, truncation, negative lengths,
resource limits, unsupported versions, and inconsistent sample counts. A synthetic BIG-backed CLI
test verifies VFS resolution and stable reports. Additional project-authored streams exercise
RefPack literal, overlapping copy, high-distance copy, invalid-reference, and output-limit paths.

`crates/cic-formats/tests/fixtures/blend.map.hex` is also original project data. Its eight-by-two
height grid is paired with version-7 tile planes, a two-row cliff bitmap, terrain and edge texture
classes, one blend record, and one cliff record. Tests reject every truncated semantic prefix,
unsupported versions, invalid counts/ranges/sentinels/UVs, and configured limit excess. A
synthetic BIG-backed CLI test verifies stable semantic reporting.

Project-authored in-memory MAP builders exercise all established world/object dictionary kinds,
waypoints/player starts, endpoint flags, unknown nested chunks, every truncated object prefix, and
configured count/depth limits. Separate builders cover all `SidesList` versions, build-list
extensions, teams, grouped and ungrouped scripts, both action branches, coordinate/scalar
parameters, truncation, non-finite values, and independent tree limits. No retail content is used.

## Remaining exclusions and open questions

- Blend versions other than 6 and 7 remain opaque in the current implementation. The observed
  version 8 is an R3 completion gate; unobserved versions are never guessed.
- Object placement, sides/teams/build lists, nested scripts, and sibling-map water overrides are
  decoded. Non-water polygon-trigger semantics remain opaque. Regular road definitions and strips
  are implemented; road join/curve geometry, bridge and object definitions, static-scenery
  rendering, real water shadows, and final capture convergence remain WIP.
- Version-1 compatibility resampling is not applied.
- No unobserved version or compression wrapper is assumed to share an established layout.
- Exact legacy fixed-function custom-edge multipass equations remain outside the established
  preview. Gameplay simulation, player/team activation, pathfinding, collision, AI, damage/repair,
  and script execution are R5-or-later work. The deterministic edge preview preserves atlas
  selection, material/decorative regions, and separate geometry without claiming pixel identity.
