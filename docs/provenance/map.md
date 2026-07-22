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
  - `Core/GameEngine/Include/GameClient/Water.h`
  - `Core/GameEngine/Source/GameClient/Water.cpp`
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
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/GameEngine/Include/GameClient/Water.h>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/GameEngine/Source/GameClient/Water.cpp>
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
row-major byte samples, and the 5/10-unit versioned grid spacing.

The pinned blend reader and declarations establish versions 6 and 7's four signed 16-bit cell
planes, tile/table counts, terrain and edge texture-class records, blend selectors and
`0x7ADA0000` sentinel, and cliff tile/UV/flag records. Version 6 contains no cliff bitmap and calls
the source height-based derivation: four-corner height range greater than 9.8 world units. With the
source 0.625 height scale, integer stored samples are cliffs at a delta of 16 or greater. Version 7
stores a legacy `(width + 1) / 8` cliff-bitmap row stride, which the project normalizes into a
zero-filled conventional stride while preserving all available bits and raw signed indices.

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
trigger ID, version-2 water byte, version-3 river byte and river-start integer, signed point count,
and integer XYZ triples. The project uses only this schema. Its hybrid-deferred render graph,
procedural wave normal, thickness absorption, refraction, Fresnel response, shoreline treatment,
and Modern macro variation are original implementation and are not derived from the legacy water
renderer. Caustic illumination is likewise project-authored, but samples caller-owned animation
frames rather than inventing a procedural pattern or copying legacy fixed-function equations.

The pinned renderer also establishes that file-stored cliff mappings apply only within one terrain
texture class, that the legacy fallback stretches UVs only beyond explicit 1.5/2.0/2.4 thresholds
and caps spans at four tiles, and that stretched cells choose their diagonal from the height
differences. Custom edges are separate quads over primary blend cells; their quarter-atlas offsets
depend on direction, inversion, diagonal length, and row/column parity. Edge texels distinguish
black mask, white material, and colored decorative regions. The project stages those source facts
as a bounded deterministic RGBA preview rather than reproducing the complete Direct3D 8 texture
stage state machine.

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

Viewport-frustum intersection, projected screen-density depth caps, nested 16/32-texel residency,
quantized safety margins, containment checks, independent generation cancellation, stale-result
suppression, explicit-time resident overlap, linear-light alpha-aware mip generation, and
anisotropic sampling are project-authored modern renderer work. They are not translations of the
legacy terrain renderer.

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

## Implementation record

The Rust implementations in `crates/cic-formats/src/map.rs`, `map_blend.rs`, `map_water.rs`,
`refpack.rs`, `terrain_ini.rs`, and `water_ini.rs`, terrain/water staging in
`crates/cic-render/src/terrain.rs` and `water.rs`, plus report/CLI integration
in `crates/cic-tools`, were authored for this project from the documented facts. No C++ source code
was copied, translated line by line, or imported. The immutable values, structured errors, explicit
limits, exact closure checks, top-level opaque-payload policy, stable staging/report schemas, and
synthetic fixtures are native to this repository.
