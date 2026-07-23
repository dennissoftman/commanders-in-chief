# MAP Format Provenance

## GeneralsGameCode evidence

- Repository: <https://github.com/TheSuperHackers/GeneralsGameCode>
- Revision: `9f7abb866f5afd446db14149979e744c7216baaf`
- Container declarations and implementation:
  - `Generals/Code/GameEngine/Include/Common/DataChunk.h`
  - `Generals/Code/GameEngine/Source/Common/System/DataChunk.cpp`
- Version declarations:
  - `Generals/Code/GameEngine/Include/Common/MapReaderWriterInfo.h`
- Height reader and WorldBuilder writer:
  - `Core/GameEngineDevice/Source/W3DDevice/GameClient/WorldHeightMap.cpp`
  - `Core/GameEngineDevice/Include/W3DDevice/GameClient/WorldHeightMap.h`
  - `Generals/Code/Tools/WorldBuilder/src/WHeightMapEdit.cpp`
  - `GeneralsMD/Code/Tools/WorldBuilder/src/WHeightMapEdit.cpp`
- Blend, edge, texture-class, and cliff record declarations:
  - `Core/GameEngineDevice/Include/W3DDevice/GameClient/TileData.h`
- Terrain staging, packed tiles, procedural alpha, and texture-class image resolution:
  - `Core/GameEngineDevice/Source/W3DDevice/GameClient/WorldHeightMap.cpp`
  - `Core/GameEngineDevice/Source/W3DDevice/GameClient/HeightMap.cpp`
  - `Core/GameEngineDevice/Source/W3DDevice/GameClient/TerrainTex.cpp`
  - `Core/GameEngineDevice/Source/W3DDevice/GameClient/TileData.cpp`
  - `Core/GameEngineDevice/Source/W3DDevice/GameClient/W3DTerrainBackground.cpp`
  - `Core/GameEngine/Include/Common/FileSystem.h`
- Custom edge geometry, quarter-atlas UVs, and edge alpha classes:
  - `Generals/Code/GameEngineDevice/Source/W3DDevice/GameClient/W3DCustomEdging.cpp`
  - `Core/GameEngineDevice/Source/W3DDevice/GameClient/TerrainTex.cpp`
- Terrain INI declarations and default inheritance:
  - `Core/GameEngine/Source/Common/INI/INITerrain.cpp`
  - `Generals/Code/GameEngine/Source/Common/TerrainTypes.cpp`
- Water polygon input and appearance declarations:
  - `Generals/Code/GameEngine/Include/GameLogic/PolygonTrigger.h`
  - `Generals/Code/GameEngine/Source/GameLogic/Map/PolygonTrigger.cpp`
  - `GeneralsMD/Code/GameEngine/Include/GameLogic/PolygonTrigger.h`
  - `GeneralsMD/Code/GameEngine/Source/GameLogic/Map/PolygonTrigger.cpp`
  - `Core/GameEngine/Include/GameClient/Water.h`
  - `Core/GameEngine/Source/GameClient/Water.cpp`
  - `Core/GameEngine/Source/Common/INI/INIWater.cpp`
  - `Core/GameEngine/Source/Common/INI/INI.cpp`
  - `Core/GameEngineDevice/Source/W3DDevice/GameClient/Water/W3DWater.cpp`
- Object, waypoint, road/bridge endpoint, and world-metadata boundaries:
  - `Core/GameEngine/Include/Common/MapObject.h`
  - `Core/GameEngine/Source/GameClient/MapUtil.cpp`
  - `Core/GameEngine/Include/GameClient/TerrainRoads.h`
  - `Core/GameEngine/Source/Common/INI/INITerrainRoad.cpp`
  - `Core/GameEngine/Source/GameClient/Terrain/TerrainRoads.cpp`
  - `Generals/Code/GameEngineDevice/Include/W3DDevice/GameClient/W3DRoadBuffer.h`
  - `Generals/Code/GameEngineDevice/Source/W3DDevice/GameClient/W3DRoadBuffer.cpp`
  - `GeneralsMD/Code/GameEngineDevice/Source/W3DDevice/GameClient/W3DBridgeBuffer.cpp`
  - `Generals/Code/Tools/WorldBuilder/src/WHeightMapEdit.cpp`
- Initial object draw-definition boundary:
  - `Core/GameEngineDevice/Source/W3DDevice/GameClient/Drawable/Draw/W3DModelDraw.cpp`
  - `Core/GameEngineDevice/Include/W3DDevice/GameClient/Module/W3DModelDraw.h`
  - `Core/GameEngine/Source/Common/INI/INI.cpp`

At the pinned revision, `W3DRoadBuffer::loadRoads` builds ordinary segments first, then inserts
three-/four-way intersections, connected curves, and explicit cross-type joins. Curves use 1.5 or
0.5 road-width radii and 30-degree subdivision; small or authored angled turns miter. Dedicated
curve, tee, four-way, Y, slanted-tee, and alpha-join regions come from the road texture atlas. The
project implementation preserves those topology and atlas-selection rules in bounded immutable
vectors, then commits geometry in stable MAP order instead of mutating fixed-size Direct3D buffers.
`W3DRoadBuffer::loadTexture` requests exactly three texture mip levels, enables best mip filtering,
and repeats both texture axes. Its terrain-fitted quads use `MAP_HEIGHT_SCALE / 8` as their height
lift. Curve construction reverses the left-hand endpoint traversal before applying the same
clockwise 30-degree subdivision used on right turns. The project preserves those presentation facts;
its additional GPU depth bias and optional polygon-line wireframe are original renderer diagnostics,
not claims about the legacy rasterizer.

`W3DModelDrawModuleData::parseConditionState` also establishes that a default state may be written
as `DefaultConditionState` or as the first `ConditionState = NONE`. Shipped modules are delimited by
`End` tokens and do not require child fields to be indented farther than `Draw`; the project parser
therefore uses a bounded state machine rather than indentation to close draw modules.
- Tree draw and ambient breeze boundary:
  - `Core/GameEngineDevice/Source/W3DDevice/GameClient/Drawable/Draw/W3DTreeDraw.cpp`
  - `Core/GameEngineDevice/Source/W3DDevice/GameClient/W3DTreeBuffer.cpp`
  - `GeneralsMD/Code/GameEngine/Include/GameLogic/ScriptEngine.h`
  - `GeneralsMD/Code/GameEngine/Source/GameLogic/ScriptEngine/ScriptEngine.cpp`
- Side, team, build-list, and player-script boundary:
  - `Generals/Code/GameEngine/Include/GameLogic/SidesList.h`
  - `Generals/Code/GameEngine/Source/GameLogic/Map/SidesList.cpp`
  - `Generals/Code/GameEngine/Include/Common/Team.h`
  - `Generals/Code/GameEngine/Source/Common/RTS/Team.cpp`
- Nested script-data boundary:
  - `Generals/Code/GameEngine/Include/GameLogic/Scripts.h`
  - `Generals/Code/GameEngine/Source/GameLogic/ScriptEngine/Scripts.cpp`
  - `Generals/Code/GameEngine/Include/GameLogic/ScriptConditions.h`
  - `Generals/Code/GameEngine/Include/GameLogic/ScriptActions.h`
- Planned global-lighting boundary:
  - `Generals/Code/Tools/WorldBuilder/include/GlobalLightOptions.h`
  - `Generals/Code/Tools/WorldBuilder/src/GlobalLightOptions.cpp`
- Cached-stream and RefPack compression path:
  - `Core/Libraries/Source/Compression/Compression.h`
  - `Core/Libraries/Source/Compression/CompressionManager.cpp`
  - `Core/Libraries/Source/Compression/EAC/refdecode.cpp`
- Permanent links:
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Generals/Code/GameEngine/Include/Common/DataChunk.h>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Generals/Code/GameEngine/Source/Common/System/DataChunk.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Generals/Code/GameEngine/Include/Common/MapReaderWriterInfo.h>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/GameEngineDevice/Source/W3DDevice/GameClient/WorldHeightMap.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/GameEngineDevice/Include/W3DDevice/GameClient/WorldHeightMap.h>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Generals/Code/Tools/WorldBuilder/src/WHeightMapEdit.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/GeneralsMD/Code/Tools/WorldBuilder/src/WHeightMapEdit.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/GameEngineDevice/Include/W3DDevice/GameClient/TileData.h>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/GameEngineDevice/Source/W3DDevice/GameClient/HeightMap.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/GameEngineDevice/Source/W3DDevice/GameClient/TerrainTex.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/GameEngineDevice/Source/W3DDevice/GameClient/TileData.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/GameEngineDevice/Source/W3DDevice/GameClient/W3DTerrainBackground.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Generals/Code/GameEngineDevice/Source/W3DDevice/GameClient/W3DCustomEdging.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/GameEngine/Include/Common/FileSystem.h>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/GameEngine/Source/Common/INI/INITerrain.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Generals/Code/GameEngine/Source/Common/TerrainTypes.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Generals/Code/GameEngine/Include/GameLogic/PolygonTrigger.h>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Generals/Code/GameEngine/Source/GameLogic/Map/PolygonTrigger.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/GeneralsMD/Code/GameEngine/Include/GameLogic/PolygonTrigger.h>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/GeneralsMD/Code/GameEngine/Source/GameLogic/Map/PolygonTrigger.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/GameEngine/Include/GameClient/Water.h>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/GameEngine/Source/GameClient/Water.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/GameEngine/Source/Common/INI/INIWater.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/GameEngine/Source/Common/INI/INI.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/GameEngineDevice/Source/W3DDevice/GameClient/Water/W3DWater.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/GameEngine/Include/Common/MapObject.h>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/GameEngine/Source/GameClient/MapUtil.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/GameEngine/Include/GameClient/TerrainRoads.h>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/GameEngine/Source/Common/INI/INITerrainRoad.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Generals/Code/GameEngineDevice/Source/W3DDevice/GameClient/W3DRoadBuffer.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/GameEngineDevice/Source/W3DDevice/GameClient/Drawable/Draw/W3DModelDraw.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/GameEngineDevice/Include/W3DDevice/GameClient/Module/W3DModelDraw.h>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Generals/Code/Tools/WorldBuilder/src/WHeightMapEdit.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Generals/Code/GameEngine/Include/GameLogic/SidesList.h>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Generals/Code/GameEngine/Source/GameLogic/Map/SidesList.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Generals/Code/GameEngine/Include/GameLogic/Scripts.h>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Generals/Code/GameEngine/Source/GameLogic/ScriptEngine/Scripts.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Generals/Code/Tools/WorldBuilder/src/GlobalLightOptions.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/Libraries/Source/Compression/Compression.h>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/Libraries/Source/Compression/CompressionManager.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/Libraries/Source/Compression/EAC/refdecode.cpp>
- Upstream notice: Command & Conquer Generals Zero Hour; Copyright 2025 Electronic Arts
  Inc.; historical notices identify Electronic Arts Inc. and the named original authors.
- License: GNU GPL version 3 or later with the Electronic Arts Section 7 additional terms
  in the upstream repository's `LICENSE.md`.

The source establishes the `CkMp` tag, signed symbol count, one-byte symbol-name length, 32-bit
identifier, 10-byte chunk header, signed payload size, last-table-entry-wins lookup behavior, and
parser-selected nested interpretation. It also establishes height versions 1 through 4, stored
width and height, version-3 border, version-4 boundary pairs, exact width-times-height sample count,
row-major byte samples, and the 5/10-unit versioned grid spacing. Version-4 boundary X/Y values are
read as signed integers without a nonnegative check; the project therefore preserves them as
signed metadata while continuing to require nonnegative allocation dimensions and counts.

The pinned blend reader and declarations establish versions 6 through 8's four signed 16-bit cell
planes, tile/table counts, terrain and edge texture-class records, blend selectors and
`0x7ADA0000` sentinel, and cliff tile/UV/flag records. Version 6 contains no cliff bitmap and calls
the source height-based derivation: four-corner height range greater than 9.8 world units. With the
source 0.625 height scale, integer stored samples are cliffs at a delta of 16 or greater. Version 7
stores a legacy `(width + 1) / 8` cliff-bitmap row stride, which the project normalizes into a
zero-filled conventional stride while preserving all available bits and raw signed indices.
Version 8 preserves the remaining payload order and corrects the stored stride to
`(width + 7) / 8`.

The terrain renderer sources establish 10 world units per grid sample, 0.625 world units per
height byte, four quadrant indices per 64-by-64 source tile, bottom-origin texture-class tile rows,
base/primary/extra layer order, procedural corner masks, triangle flips, and repeated 2-by-2 mip
averaging with integer rounding. The Terrain INI sources establish the `Texture` field, ordered
redefinition, inheritance from `DefaultTerrain`, and image lookup beneath `Art/Terrain`.
`W3DTerrainBackground.cpp` establishes the eight-pixel background density and bounded higher-detail
texture regeneration; the WorldBuilder cliff generator establishes 32 texels per cell for authored
foreground coordinates. The project's camera-centered region size, refresh threshold, and fixed
directional preview light are original implementation policy rather than copied source behavior.

The pinned polygon-trigger reader establishes a signed trigger count, 16-bit byte-string length,
trigger ID, version-2 water byte, version-3 river byte and river-start integer, version-4 bounded
WorldBuilder layer-name byte string, signed point count, and integer XYZ triples. The project uses
only this schema. Its hybrid-deferred render graph,
procedural wave normal, thickness absorption, refraction, Fresnel response, shoreline treatment,
and Modern macro variation are original implementation and are not derived from the legacy water
renderer. Caustic illumination is likewise project-authored, but samples caller-owned animation
frames rather than inventing a procedural pattern or copying legacy fixed-function equations.

For the ADR-0009 gates, the pinned WorldBuilder writer establishes top-level `WorldInfo`,
`ObjectsList`, `PolygonTriggers`, `GlobalLighting`, and `SidesList` output. `MapUtil.cpp` establishes
nested `Object` location, angle, flags, name, later-version typed dictionary data, waypoint
collection, and one-based `Player_n_Start` recognition. `MapObject.h` establishes distinct road and
bridge endpoint, corner, join, mirror, and no-draw flags. The TerrainRoad declarations and INI
reader establish road texture/width inputs and bridge model/scale/state/tower references. The
bounded bridge definition now retains all four body-state model/texture pairs and all four tower
object names; only the pristine state is presented. These facts support the immutable placement
view and endpoint staging implemented here. The regular-road
gate additionally follows `W3DRoadBuffer.cpp` for consecutive Point1/Point2 pairing, the product of
road width and in-texture width, terrain-cell tessellation, maximum supporting height, small
height offset, regular-segment UV scale, handed curve traversal, atlas junction selection, repeated
addressing, and three-level road texture mip budget. Road depth bias and polygon-line wireframe are
project-authored modern renderer policy. Cloud/noise lighting is not claimed yet.
`W3DBridgeBuffer.cpp` establishes
that bridge marker Z is rebuilt as terrain plus `BRIDGE_FLOAT_AMT` (`0.25`), and establishes the
`BRIDGE_LEFT`/`BRIDGE_SPAN`/`BRIDGE_RIGHT` section lookup, rounded span repetition, X offsets, and
endpoint-axis deformation used by the intact bridge preview. Its WorldBuilder tower path establishes
the from-left/from-right/to-left/to-right slot order, lateral min/max-Y corners, opposite facing for
from-side towers, first `W3DModelDraw` selection, and final bridge-info endpoint Z written by
`updateTowerPos`. The project resolves those templates into renderer-only instances; it does not
construct targetable objects. Texture/material staging is native to this repository. It does not
claim source-equivalent collision, damage-state selection, transition effects, repair, or tower
behavior.

`W3DTreeDraw.cpp`/`.h` establish tree model/texture plus move-outward, move-inward, darkening,
topple, and shadow module fields. `ScriptEngine.h`/`.cpp` establish global `BreezeInfo`, its default
direction/intensity/lean/period/randomness, and the `SET_TREE_SWAY` script action; `W3DTreeBuffer.cpp`
establishes explicit-frame cosine sway consumption. R3 uses those default scalar inputs with
explicit presentation time. Selection of ten placement-ID sway families and the stable bounded
random factors is project-authored deterministic policy. R3 does not execute `SET_TREE_SWAY`;
custom scripted wind remains data until R5.

`W3DModelDraw.cpp`/`.h` and the generic INI reader establish top-level `Object`/`ObjectReskin`
declarations, draw-module naming, `DefaultConditionState` model selection, and per-draw `Scale`.
The bounded `End`-delimited parser, reskin ancestry resolution, stable first-use GPU instancing, and
neutral identity root/HLOD wrapper for validated standalone meshes are project-authored. The
renderer-only placement composition samples the exact already-staged terrain triangle, including
the height-field border and selected diagonal, then adds the persisted relative Z offset verbatim;
there is no clamp or renderer epsilon, and this is not an upstream gameplay or construction policy.
The modeled source-derived defaults, input branches, binary structures, and road output primitives
are cross-referenced to executable tests in
[`docs/testing/source-derived-map-scene.md`](../testing/source-derived-map-scene.md).

The height reader establishes signed version-4 boundary coordinates and the terrain sources
establish their relationship to the height grid. The translucent fence, terrain-following base,
and common top above the greatest MAP height are project-authored viewer policy. They do not alter
collision or reachable terrain.

`SidesList.cpp` establishes versions 1 through 3, ordered side dictionaries and build lists,
version-2 team dictionaries, version-3 build-list extensions, and nested `PlayerScriptsList` data.
`Scripts.cpp` establishes the `PlayerScriptsList` -> `ScriptList` -> `ScriptGroup` -> `Script`
nesting, OR/AND condition records, true/false action lists, typed parameters, source flags/comments,
and versioned evaluation delay. ADR 0009 requires the project to preserve these records as bounded
data in R3 without calling the upstream runtime validation/repair or opcode-dispatch behavior.
The project implementation follows that boundary: it retains ordered raw values and unknown
opcodes, but exposes no evaluator, scheduler, runtime objects, or repair pass.

The WorldBuilder writer and runtime reader establish `GlobalLighting` versions 1 through 3. The
payload begins with the one-based selected time and then four ordered time variants. Version 1 has
terrain sun followed by object sun; version 2 adds two object accents; version 3 adds two terrain
accents. Every light writes ambient RGB, diffuse RGB, and XYZ source direction as nine reals. The
reader consumes one optional final packed shadow color when bytes remain. The project preserves
these distinctions and exact source order without importing editor defaults.

`Water.cpp` establishes every `WaterSet` field and the complete `WaterTransparency` table;
`INIWater.cpp` establishes time-name selection, direct `WaterSet` accumulation, and partial
transparency override behavior. The generic `INI::parseRGBAColorInt` establishes ordered RGB,
optional alpha defaulting to 255, and inclusive 0-through-255 channels. The project decoder retains
all fields under explicit resource and value limits. The generic `INI::parseRGBColor` establishes
that standing/radar colors are also ordered integer 0-through-255 channels and are divided by 255
when stored as renderer-neutral real RGB values.

`Water.h` establishes the `WaterTransparencySetting` constructor values used before any INI is
loaded: full minimum opacity, a three-world-unit opaque depth, white standing color, the default
standing-water texture, and non-additive blending. The installed Generals profile relies on that
constructor texture because its Water INI does not repeat the field. Resource loading applies all
mounted provider versions in base-to-overlay order, followed by a sibling map INI, so partial
edition and map definitions accumulate without mutating the immutable parser output.

`W3DWater.cpp` establishes that standing lakes use `WaterTransparency`'s standing texture and
optional standing-color override, otherwise modulate the selected `WaterSet` diffuse color/alpha,
honor the additive flag, repeat the texture at a 150-world-unit scale, draw after opaque terrain
without writing depth, and use terrain-produced destination alpha for the soft shoreline when
available. It dispatches non-river water polygons as ordered trapezoid spans and subdivides each
span at eight terrain cells, while rivers remain paired strips. The project's legacy policy derives
those resource, blend, scale, and depth-feather semantics but implements them in original bounded
`wgpu` staging/WGSL rather than copying the Direct3D 8 state machine or shader assembly. Modern
refraction, absorption, foam, and Fresnel behavior remains separate project-authored policy.

`W3DWater.cpp::drawRiverWater` establishes that a river's stored points are one perimeter and that
`river_start` is the seam between its banks. It initializes indices on either side of that seam,
increments one with wraparound, decrements the other with wraparound, emits the resulting pairs,
and then connects consecutive pairs as quads. The project stages the same bounded point ordering
without copying the fixed-function river material or shader implementation.

The pinned renderer also establishes that file-stored cliff mappings apply only within one terrain
texture class, that the legacy fallback stretches UVs only beyond explicit 1.5/2.0/2.4 thresholds
and caps spans at four tiles, and that stretched cells choose their diagonal from the height
differences. Custom edges are separate quads over primary blend cells; their quarter-atlas offsets
depend on direction, inversion, diagonal length, and row/column parity. Edge texels distinguish
black mask, white material, and colored decorative regions. The project stages those source facts
as a bounded deterministic RGBA preview rather than reproducing the complete Direct3D 8 texture
stage state machine.

`WorldHeightMap.cpp` reads the stored cliff-info count without requiring it to be positive, then
reads explicit entries only for indices one through count minus one. Runtime index repair falls
back to the implicit zero entry. The project therefore retains a raw zero count as an empty table
instead of rejecting the payload or allocating a fabricated record.

The cached input path detects `EAR\0`, reads a signed decompressed length, and dispatches RefPack.
The pinned decoder establishes the accepted type words, three- or four-byte big-endian inner size,
four literal/copy command forms, overlapping back-references, and explicit end command.

## Synthetic verification

On 2026-07-21, the original `minimal.map.hex` fixture closed exactly as two symbol entries and
three top-level chunks. Its version-4 height payload decoded to six row-major samples and one
boundary. Project-authored tests construct and decode versions 1 through 4 and reject every
truncated non-boundary prefix, negative chunk sizes, unsupported height versions, limit excess,
and inconsistent sample counts. A synthetic BIG archive resolves the MAP through the VFS and
produces checked stable inventory and height reports.

The original `blend.map.hex` fixture closes exactly as a version-4 eight-by-two height payload and
a version-7 255-byte blend payload. Tests cover exact decoded planes and records, all truncated
semantic prefixes, unsupported versions, invalid cell counts, sentinels and non-finite UVs, limit
excess, and stable BIG-backed reporting. Exact height PNG bytes are decoded and checked as a
three-by-two grayscale image.

The same original blend fixture drives deterministic GPU completion captures. Its legacy-adjusted
layered path matches RGBA SHA-256
`d19dee6e96471515ab0b4902e99aa9bed44650b10f975e35a91c427e95f96cad`; its separate synthetic
custom-edge pass matches `5f5761f44446d8784b7c0910adee7ede440c9e428a3d4b25be26ce470bfabd27`.

On 2026-07-21, one user-owned Steam Generals MAP member was inspected in place. Its 275,524-byte
`EAR\0` wrapper decompressed to a 1,781,076-byte `CkMp` stream, closed exactly as 46 symbols and 8
top-level chunks, and exposed a version-4 380-by-400 height grid with 70-sample border, one
boundary, and 152,000 samples. Its version-7 blend payload then validated 152,000 cells, 204 bitmap
tiles, 7,772 blended-tile entries, 14 terrain texture classes, and one cliff-info entry. A
380-by-400 grayscale PNG was header-checked and deleted. Only aggregate counts and versions are
recorded; no retail bytes, names, height values, images, or captures are retained.

On 2026-07-22, the same user-owned installation supplied MAP, Terrain, and INI archives to the
headless renderer. All 14 semantic terrain classes resolved, 151,221 cells produced 152,000
vertices and 907,326 stable indices, and a 768-by-768 capture was visually checked for layer,
quadrant, and geometry alignment. The capture was deleted after inspection. Only these aggregate
counts are retained.

After the cliff-UV policy and viewer were added on 2026-07-22, the same installed Generals MAP
again staged 151,221 cells and 907,326 base indices under `legacy`; it contained no active custom
edge cells. A 768-by-768 capture was visually checked with RGBA SHA-256
`ec0c67274ae2837526a0d4d245d97012436a513e45d815a0cc99d1826beba523`, and the perspective viewer
remained live through loading, GPU upload, surface creation, and camera rendering. Captures were
then deleted; no retail image or MAP data is retained.

The same installed viewer smoke was repeated after bounded near-field streaming and directional
shading were added. Its initial 32-pixel-per-cell detail window uploaded and rendered over the
stable background, and the process remained live for 12 seconds. No screenshot or retail-derived
output was retained.

Viewport-frustum intersection, camera-space screen-density depth caps, projected page ranking with
coarse-visible coverage priority, fixed bordered virtual pages, stable two-level page tables,
deterministic LRU residency, GPU semantic composition, linear-light
alpha-aware mip generation, and anisotropic sampling are project-authored modern renderer work.
They are not translations of the legacy terrain renderer.

The optimized USA06 viewer remained live for a controlled 15-second smoke after projected page
ranking replaced the radial/world-axis approximation. No screenshot or retail data was retained.

Back-face culling is project-authored render policy over the stable counter-clockwise height-field
winding. No legacy culling state was copied. Both deterministic terrain captures retained their
existing hashes after culling was enabled.

The same user-owned installation exposed a complete 32-frame `caust00` through `caust31` TGA
sequence. Each frame was 64-by-64, 32-bit, with a much subtler luminance range and adjacent-frame
change than the alternate high-contrast sequence. Its default `WaterTransparency` values reached
opacity 1.0 at depth 3.0. Only these aggregate dimensions and scalar observations were retained;
the frames, INI text, temporary contact sheet, and viewer capture were not copied into the project.
The implementation loads the resources through the user's VFS at runtime and stores only bounded
decoded luminance frames in `WaterAppearance`.

On 2026-07-22, USA01's installed version-7 blend data reported 23 terrain classes, 8,425 ordinary
blend records, and zero custom-edge tiles/classes. This establishes that the isolated stair-step
road transition observed in the viewer comes from map-authored ordinary cell blends, not the custom
edge renderer. After the legacy water policy began resolving the installed standing-water texture,
diffuse alpha, additive flag, and depth opacity, the optimized viewer remained live for a controlled
12-second smoke. No texture, MAP bytes, or capture were retained.

On 2026-07-22, original synthetic version-1 through version-3 `GlobalLighting` payloads verified
the exact version additions, four time variants, separate terrain/object source order, optional
shadow color, every truncated prefix, invalid time, non-finite scalar, and trailing-byte failures.
A synthetic BIG-backed `map-lighting` artifact checks exact float-bit reporting. Original Water INI
tests cover every source field, partial repeated definitions, bounded strings/scalars/counts,
malformed colors/times, nesting, and exact block closure. GPU tests validate the expanded uniform
layout while existing deterministic terrain capture hashes remain unchanged.

The user-owned installed USA05 MAP then decoded in place as `GlobalLighting` version 3 with
afternoon selected, four ordered time variants, three terrain and three object lights per variant,
and a final packed shadow color. The report exited zero and closed exactly. Only these aggregate
facts were retained; no MAP bytes, scalar values, or report output were copied into the project.

The installed Generals Water INI then exercised three-channel integer vertex colors with omitted
alpha. After matching the source default-alpha rule, USA01 cleared Water INI parsing, MAP/resource
staging, GPU upload, and viewer launch, and remained live until the controlled smoke timeout. The
process was stopped without retaining a capture, INI text, or source color values.

The installed Zero Hour Water INI then exercised byte-RGB standing/radar transparency colors.
After normalizing those bounded channels at the immutable format boundary, Bridge Busters cleared
configuration and resource staging and remained live for a controlled 12-second optimized viewer
smoke. No retail INI bytes, color values, MAP data, or capture were retained.

Controlled release-viewer probes compared screen pixels immediately and four seconds after a
forward-camera move and, separately, a four-notch wheel dolly. Both 47,838-sample comparisons had
zero changed pixels above a three-level RGB threshold and zero mean RGB delta, demonstrating that
detail did not visibly rise after motion once the cancellable predictive residency path was active.
No probe capture was retained.

After the final appearance uniform and bounded Water INI integration, the optimized Bridge Busters
viewer remained live for 12 seconds with installed caustic frames, source opacity/depth, shoreline
effects, predictive detail, complete mips, and anisotropic sampling active. It then accepted a
normal window close and exited with code zero. No retail capture or data was retained.

The installed USA05 map supplied a version-6 `BlendTileData` completion observation: 400 by 320,
128,000 cells, 40 bitmap tiles, one implicit blend entry, one implicit cliff-info entry, two terrain
classes, and no edge classes. The complete 128,008-line stable report exited zero. The optimized
viewer then remained live for 12 seconds and accepted a normal code-zero close. No MAP bytes,
report, or capture were retained.

After nested screen-space detail replaced the single horizon-sized rectangle, an automated release
USA05 probe raised the camera to a shallow terrain view and captured the immediate frame and one
three seconds later. Visual inspection found no terrain-quality rise; a 63,180-sample comparison
had mean summed RGB delta 0.236, with the small remainder consistent with animated water. Both
temporary captures were deleted.

A 2026-07-22 release smoke of the user-owned Bridge Busters map exited normally after 23 seconds
with that viewport LOD and filtering path active. No capture or retail-derived data was retained.

Controlled optimized viewer smokes then verified the constructor-default standing-water resource
on a Generals map, inheritance of a Generals terrain definition beneath a Zero Hour overlay, and a
version-7 map whose stored cliff-info count is zero. Each remained live until its timeout. Synthetic
tests reproduce constructor/global/map water precedence, shadowed Terrain INI accumulation, and the
empty cliff table without retaining retail bytes or values.

The installed USA06 map exposed one renderable static reservoir polygon and two degenerate water
markers. Ambient water-loop objects trace the downstream channel, while dam mission state governs
its dynamic presentation. The map-local INI is now part of static water resolution, but R3 does not
execute mission scripts or fabricate absent water geometry. Only these aggregate observations were
retained.

A user-owned version-4 map with multiple boundaries then exercised a negative stored boundary
coordinate and decoded successfully after the signed metadata correction. Another installed map
supplied one long river perimeter with a nonzero midpoint seam; the source-established bank walk
reconstructed its paired strip, and both optimized viewers remained live for controlled smokes.
Only aggregate facts were retained; no retail points, boundary values, MAP bytes, or captures were
copied into the repository.

## Implementation record

The Rust implementations in `crates/cic-formats/src/map.rs`, `map_blend.rs`, `map_water.rs`,
`map_scenario.rs`, `object_ini.rs`,
`refpack.rs`, `road_ini.rs`, `terrain_ini.rs`, and `water_ini.rs`, terrain/water/road/scenery staging in
`crates/cic-render/src/terrain.rs`, `water.rs`, `map_scene.rs`, `road.rs`, `scenery.rs`, and
`boundary.rs`, the project-authored `scene_shadow.wgsl` and edge-aware composite in
`terrain_deferred.wgsl`, plus report/CLI integration
in `crates/cic-tools`, were authored for this project from the documented facts. No C++ source code
was copied, translated line by line, or imported. The immutable values, structured errors, explicit
limits, exact closure checks, top-level opaque-payload policy, stable staging/report schemas, and
synthetic fixtures are native to this repository.

Project-authored scenario tests synthesize complete dictionaries, world/object records,
waypoints/player starts, endpoint flags, side/team/build-list versions, and grouped/ungrouped nested
scripts. They cover exact closure, every truncated object prefix, finite-value checks, independent
allocation/depth limits, both action branches, and raw coordinate/scalar parameters. Water tests
use synthetic VFS providers to reproduce constructor/global/sibling-map precedence for standing,
sky, and environment textures; the WGSL parser/validator test checks the expanded bindings. No
retail bytes, strings, maps, or images are retained.

The fixed-isometric headless scene overview is project-authored integration policy. It reuses the
deterministic GPU terrain capture, then composites source-ordered road/water triangles and
placement markers with explicit-time tree offsets. It does not translate an upstream thumbnail
renderer and does not claim interactive-view pixel equivalence.

Waypoint/start octahedra, connected waypoint-path ribbons, and polygon perimeter walls in
`crates/cic-render/src/map_overlay.rs` are project-authored diagnostics over the retained
source-classified values. The source dictionary supplies up to three waypoint path-label
properties; case folding, lexical color-group order, waypoint-ID connection order, hue selection,
ribbon dimensions, subdivision, terrain sampling, and geometry limits are original project policy.
They do not imply trigger registration, player construction, pathing, or simulation ownership.

Project-authored road tests cover source-order road/bridge field replacement, malformed structure,
finite reals, definition limits, consecutive endpoint pairing, terrain fitting,
first-use materials, unresolved definitions, and edge-derived corner/junction approximation.
Object-definition tests cover multiple draws, reskins, invalid scales, and independent resource
limits; renderer tests cover stable instance transforms, exact terrain-triangle sampling, the
global boundary top, Header3 two-sided selection, and WGSL validation. A controlled user-owned
Bridge Busters staging smoke resolved seven intact bridges through seven endpoint-deformed model
batches while roads, scenery, water, and 760 boundary segments remained loadable. Only aggregate counts were retained; no retail
bytes, strings, models, textures, MAP data, or captures were copied into the repository.
