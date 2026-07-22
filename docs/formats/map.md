# MAP Container, Semantic Gates, and Scene-Presentation Plan

- Status: container/terrain/water subset implemented; complete R3 MAP semantics and scene
  presentation planned by ADR 0009; water presentation WIP
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
`(width - 2 * border, height - 2 * border)`. Version 4 preserves boundaries in file order.
Dimensions must be positive, the doubled border must fit both dimensions, counts are checked before
allocation, and no trailing payload bytes are accepted.

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
remains immutable and renderer-neutral.

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

`cic-inspect map-render` additionally decodes ordered `Terrain`/`Texture` declarations from the
mounted default and edition Terrain INIs, applies `DefaultTerrain` inheritance, resolves sheets
beneath `Art/Terrain`, stages source-scaled geometry and base/primary/extra layers, and writes an
sRGB headless PNG. Size and power-of-two pixels per cell are explicit; diagnostics include stable
geometry/layer counts and the captured RGBA SHA-256. `--terrain-policy legacy` is the default and
applies stored cliff UVs plus the bounded steep-slope retile; `modern` retains stored mappings but
skips the implicit steep-slope adjustment. Custom edges use a separate deterministic index/atlas
pass. `cic-inspect map-view` launches the same staged terrain in a perspective free-flight viewer.
The viewer overlays independent depth-capped 16- and 32-texel screen-space regions on the stable
8-texel background. Oblique horizon coverage cannot lower the nested foreground tiers. It
uses a hybrid-deferred terrain G-buffer and lighting resolve, followed by a modern depth-aware
forward water pass. Complete caller-supplied caustic frame sequences are mipmapped and sampled as a
world-projected texture array; global `WaterTransparency` values control deep opacity and the depth
at which it is reached. Obsolete detail bakes are cancelled off-thread without request throttling;
only the newest complete linear-light mip chain reaches upload, resident replacements overlap in
explicit presentation time, and patches are sampled trilinearly with supported
anisotropy. `modern`
additionally applies deterministic world-anchored macro variation without rotating authored
tiles. These are renderer-authored presentation policies, not decoded MAP lighting or
translations of the legacy water renderer.

The narrow Water INI boundary accepts ordered `WaterTransparency` blocks with
`TransparentWaterMinOpacity` in the inclusive 0-to-1 range and
`TransparentWaterDepth` in the finite, positive 0-to-10,000 range. Repeated fields use stable
file-order last-value-wins behavior. Input bytes, line count, line length, nesting, numeric values,
and exact block closure are bounded and produce structured errors. Other blocks, including
`WaterSet`, are deliberately ignored until their appearance semantics have a separate gate.

## Planned R3 semantic gates

The sections below define design targets, not currently implemented decoders. Every target keeps
the generic inventory's opaque payload and adds a separate label/version-aware immutable view. No
semantic decoder may create renderer or simulation resources.

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

The planned value retains source version, stable placement ID, exact float bits, flags, raw name,
typed dictionary entries, waypoint fields, and unknown properties. Duplicate names remain distinct.
The decoder checks nested closure, field finiteness, dictionary type/length/count limits, and total
placement allocation before returning. It does not resolve templates, normalize ownership, repair
teams, or instantiate objects.

### Roads and bridges

Roads are not merely painted `BlendTileData`. `MapObject` flags identify first/second road and
bridge endpoints plus corner/join policy. TerrainRoad/TerrainBridge INI definitions provide the
presentation resource data: road texture and width, or bridge model/scale plus state variants and
tower references.

R3 will preserve endpoint records in object order, diagnose incomplete or ambiguous pairs, and
derive a separate immutable road/bridge presentation list. The renderer will stage terrain-fitted
road strips, stable joins/corners, and the intact bridge presentation. Damaged/broken resources,
tower names, effect references, and repair data may be retained, but collision, damage, repair,
sound, effects, and state transitions are simulation concerns.

### Waypoints, player starts, sides, teams, and build lists

Waypoint objects are ordinary persisted object records with waypoint properties. The established
map-info reader recognizes one-based `Player_1_Start`, `Player_2_Start`, and subsequent names as
start positions. R3 exposes these as ordered spawn candidates without choosing slots or creating
players.

`SidesList` versions 1 through 3 are planned as separate established gates. Source evidence shows
ordered side dictionaries and build lists; version 2 adds team dictionaries; version 3 adds
build-list script/health/behavior fields; and the chunk nests `PlayerScriptsList`. The immutable
scenario view retains side/team ownership and alliance names, initial/build-list placements, and
script-list associations. Cross-reference reports may flag duplicate names, missing owners,
missing start waypoints, or dangling teams, but parsing never performs the legacy reader's repair or
validation mutations.

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

Scripts retain names, comments, active/one-shot/subroutine flags, difficulty flags, evaluation
delay, source versions, integer condition/action opcodes, and ordered typed parameters. Conditions
preserve OR groups and source-ordered AND chains. Unknown opcode and parameter values remain data.
Default limits must independently bound player lists, groups, scripts, conditions, actions,
parameters, strings, and nested depth; malformed or excessive trees return structured errors.

R3 reports this tree and may diagnose unresolved object, waypoint, side, team, or script names. It
does not consult live opcode templates, apply implicit compatibility rewrites, evaluate a condition,
schedule a delay, or execute either action branch. All dispatch and mutation belong to R5.

### `GlobalLighting` and water appearance (WIP)

The WorldBuilder writer establishes `GlobalLighting` version 3. The associated source distinguishes
terrain lighting from terrain-object lighting and provides ordered sun/accent inputs across
time-of-day variants. The planned decoder retains finite ambient/diffuse colors and light vectors
as immutable renderer-neutral values with exact version and source order. The renderer will replace
its fixed preview light only after synthetic field/layout tests close this gate.

Water is not considered visually complete. Remaining work includes established `WaterSet` and
map-specific appearance inputs, interaction with decoded time-of-day lighting, shadows received by
water and cast onto its bed where appropriate, color/opacity/absorption, refraction, foam and
shorelines, source caustics, specular response, anti-aliasing, and bounded reflection quality. Water
stays outside the opaque G-buffer in an ordered forward pass. Completion requires repeatable
explicit-time synthetic captures and user-owned visual comparisons; the current shader output is a
WIP diagnostic, not the target look.

### Object-definition and static-scene resolution

Placed records name definitions; they do not directly contain complete draw resources. A bounded
object-definition subset will resolve only initial presentation data, beginning with
`W3DModelDraw` model/condition-state selection, scale, shadow presentation, textures, and
source-authored ambient visual animation. Unknown gameplay modules and unsupported draw modules
remain preserved or diagnosed and are never instantiated by the viewer.

The resolved R3 scene includes every supported map-authored drawable: buildings, trees, rocks,
props, bridges, decals/scorches, static light objects, and other terrain scenery. Existing R2 W3D
hierarchy, material, mapper, animated-texture, and animation paths are reused. Tree/vegetation sway
and other ambient loops use decoded source parameters or an explicit documented profile policy and
sample explicit presentation time; they do not advance fixed ticks. Stable placement IDs control
culling, batching, draw order, and diagnostics. Missing models or textures use visible placeholders
and retain their requested names.

### Planned reports and completion order

- `map-objects` should report world/object records, flags, typed properties, waypoints, road/bridge
  endpoints, and stable placement IDs.
- `map-scenario` should report polygon areas, player-start candidates, sides, teams, ownership,
  alliances, build lists, and cross-reference diagnostics without runtime validation or repair.
- `map-scripts` should report the complete nested script tree with versions, raw opcodes, typed
  parameters, and no execution.
- `map-view` should progressively integrate source lighting and WIP water, roads/bridges, all
  supported static drawables, explicit-time ambient animation, shadows, and reflections.

Implementation order is lighting/water, object/world decoding, roads/bridges, object-definition
resolution and static scenery, sides/teams/spawns, complete polygons/scripts, then integrated scene
closure. Each step adds its own synthetic fixture, negative tests, stable report, documentation,
and completion artifact before the next parser family opens.

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

## Remaining exclusions and open questions

- Blend versions other than 6 and 7 remain opaque in the current implementation. The observed
  version 8 is an R3 completion gate; unobserved versions are never guessed.
- Object placement, scripts, lighting, `WaterSet`/map-specific water appearance, and non-water
  polygon-trigger semantics remain opaque in the current implementation; the planned R3 gates
  above replace their former roadmap exclusion.
- Version-1 compatibility resampling is not applied.
- No unobserved version or compression wrapper is assumed to share an established layout.
- Exact legacy fixed-function custom-edge multipass equations remain outside the established
  preview. Gameplay simulation, player/team activation, pathfinding, collision, AI, damage/repair,
  and script execution are R5-or-later work. The deterministic edge preview preserves atlas
  selection, material/decorative regions, and separate geometry without claiming pixel identity.
