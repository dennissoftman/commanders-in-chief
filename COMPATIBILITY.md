# Compatibility Matrix

Status values are `unknown`, `observed`, `implemented`, and `verified`. `Verified`
requires synthetic fixtures plus comparison against legally obtained game data or
observable behavior.

| Area | Capability | Status | Evidence |
|---|---|---|---|
| VFS | ASCII case-insensitive virtual paths | verified | Unit tests + GitHub CI run 29840005186 |
| VFS | Deterministic loose-directory overlays | verified | Unit/CLI tests + GitHub CI run 29840005186 |
| VFS | Lazy disk-backed directory and BIG payload reads | implemented | Synthetic provider deletion + bounded-read tests |
| VFS | Declarative custom base and ordered mod profiles | implemented | Bounded profile tests + arbitrary-name CLI overlay fixture |
| BIG | `BIGF` index and mounting | verified | 16-test suite + 18 Steam Generals archives |
| BIG | Directory trailers | verified | None, `L225`+zero, and `L231`+zero across 18 archives |
| BIG | `BIG4` index and mounting | implemented | Corroborating GPL evidence; runtime/Generals use unverified |
| BIG | Duplicate entry resolution | implemented | Static tests: last table entry wins; history retained |
| CSF | Version 3 header and record boundaries | verified | Synthetic tests + installed Steam Generals CSF |
| CSF | Complemented UTF-16 text | verified | Original fixture + installed Steam Generals CSF |
| CSF | Optional wave names and zero-string labels | verified | Original fixture + installed Steam Generals CSF |
| CSF | Duplicate-label preservation | implemented | Synthetic duplicate-label test; retail file has none |
| CSF | Deterministic VFS-backed report | verified | Synthetic BIG-to-CSF CLI integration test |
| W3D | Nested chunk inventory | verified | Original fixture + 113,980-byte installed asset |
| W3D | Unknown payload preservation | verified | Nested synthetic round-trip values and CLI report |
| W3D | Known identifier reporting | implemented | 73 identifiers from pinned GPL header |
| W3D | Static mesh geometry | verified | Original fixture + version 4.2 Steam Generals mesh |
| W3D | Deterministic static mesh report | verified | Synthetic BIG-backed CLI integration test |
| W3D | Vertex-material diffuse colors and pass IDs | verified | Colored fixture + installed version 4.2 meshes |
| W3D | Per-vertex DCG diffuse colors | implemented | Synthetic override and negative tests |
| W3D | Shaders, textures, and UV references | verified | Composed two-BIG fixture + Generals/Zero Hour exports |
| W3D | Hierarchy and highest-detail HLOD composition | verified | Split synthetic fixture + installed composed models |
| W3D | One-bone skin influences | verified | Synthetic glTF assertions + installed Zero Hour infantry |
| W3D | Classic raw animation | verified | Synthetic clip + 23-action Blender import |
| W3D | Time-coded compressed animation | verified | Synthetic interpolation/step/negative tests + installed infantry GLB |
| W3D | Adaptive-delta compressed animation | implemented | Synthetic packet/decompression/negative tests |
| W3D | Secondary material passes/stages | verified | Synthetic two-pass/two-stage GLB metadata + installed two-pass meshes |
| W3D | Mapper modes and argument strings | verified | Synthetic bounded-string tests + installed environment mapper |
| W3D | Animated-texture descriptors | implemented | Validated type/count/rate tests + deterministic GLB metadata |
| W3D | Additive light-sprite glTF preview | verified | Synthetic source/derived image test + installed `abarfrccmd.w3d` airstrip lights |
| W3D | Exact fixed-function visual blending | unknown | Exact shader/pass/stage data retained; core glTF preview is explicitly pass 0/stage 0 |
| W3D | GLB/glTF 2.0 preview export | verified | Synthetic CLI/PNG tests + Blender 3.3 GLB import |
| Renderer | Stable validated W3D geometry staging | verified | Original fixture + synthetic two-BIG CLI + installed building |
| Renderer | Headless RGBA8 triangle/pose capture | verified | Checked-in SHA-256 + local RTX 4080 SUPER capture |
| Renderer | BIG-backed composed bind-pose capture | verified | Synthetic CLI PPM + installed `abarfrccmd.w3d` capture |
| Renderer | Pass-zero/stage-zero textured material submission | verified | Synthetic resource-manager/material tests + installed airstrip and infantry window smokes |
| Renderer | Stable additional pass/stage preview submission | verified | Synthetic two-pass/two-stage capture + installed airstrip smoke |
| Renderer | Rigid/one-bone hierarchy bind pose | verified | Synthetic skinned fixture + installed rigid building capture |
| Renderer | Interactive animated hierarchy pose | verified | Explicit integer-frame sampling + installed 39-clip infantry window smoke |
| Renderer | Texture image and effective-material deduplication | verified | Alias/content unit tests + installed material/texture counts |
| Renderer | Explicit-time animated mapper transforms | implemented | Pinned-formula unit tests + deterministic scrolling-mapper capture |
| Renderer | Textured explicit-animation-frame capture | verified | Checked synthetic SHA-256 + installed infantry frame smoke |
| Resources | Generals install profile and Steam discovery | verified | Installed Steam export + synthetic `--game-dir` test |
| Resources | Zero Hour base/delta profile | verified | Installed layered `--zh` exports |
| MAP | `CkMp` chunk inventory and unknown payload preservation | verified | Original synthetic fixture + installed RefPack MAP closure |
| MAP | `EAR\0` RefPack wrapper | verified | Synthetic back-reference/negative tests + installed MAP decompression |
| MAP | `HeightMapData` versions 1 through 4 | verified | Original fixture/version tests + installed version-4 map |
| MAP | `BlendTileData` versions 6 and 7 | verified | Original version-7 fixture + installed version-6/version-7 maps |
| MAP | `BlendTileData` version 8 | unknown | Observed in a user-owned Zero Hour map; layout not guessed |
| MAP | Terrain INI resolution and layered terrain staging | verified | Synthetic capture hashes + installed 14-class terrain smoke |
| MAP | Water/river `PolygonTriggers` versions 2 and 3 | verified | Synthetic truncation/triangulation tests + installed lake/empty-marker smokes |
| MAP | Water transparency and source caustic inputs | verified | Synthetic limits + installed scalar/frame observations; final appearance remains WIP |
| MAP | Final water appearance quality | unknown | Explicit R3 WIP: source `WaterSet`, lighting, shadows, reflections, and comparisons remain open |
| MAP | `GlobalLighting` | unknown | Source-established design gate; decoder not started |
| MAP | `WorldInfo` and `ObjectsList` placements | unknown | Source-established ADR-0009 gate; decoder not started |
| MAP | Roads and bridges | unknown | Source-established object flags/INI definitions; staging not started |
| MAP | Object draw-definition resolution and static scenery | unknown | Planned initial-presentation subset; no gameplay modules |
| MAP | Waypoints and `Player_n_Start` spawn candidates | unknown | Source-established ADR-0009 gate; decoder not started |
| MAP | `SidesList`, teams, and build lists | unknown | Source-established versions 1 through 3; decoder not started |
| MAP | Complete polygon-area semantics | unknown | Water-only projection implemented; general areas not started |
| MAP | Nested player-script tree | unknown | Source-established non-executing R3 data gate; decoder not started |
| MAP viewer | Complete terrain scene with roads, static objects, and ambient animation | unknown | Planned R3 integration gate |
| WND | File/layout blocks, nested windows, fields, and unknown preservation | unknown | Source-established R4 gate; decoder not started |
| WND | Complete classic status/style and gadget vocabulary | unknown | Source-established R4 gate; immutable control definitions not started |
| WND | Versioned post-parse patch overlays with provenance | unknown | Planned project-owned R4 format; no source WND mutation |
| UI resources | Mapped images, explicit fonts, CSF labels, cursors, and transitions | unknown | Planned bounded R4 resource-resolution gate |
| UI runtime | Retained controls, focus/input, menu stack, and safe callback routing | unknown | Planned `cic-ui`; arbitrary callback execution forbidden |
| UI renderer | Custom deterministic `wgpu` WND presentation | unknown | ADR 0010; text backend compatibility review pending |
| UI shell | Working main-menu navigation | unknown | Planned user-owned Main Menu completion artifact |
| UI shell | Modern resolution and refresh-rate Settings path | unknown | Planned patched Options WND + injected mode-catalog/rollback tests |
| UI shell | Skirmish/map selection with R3 preview and spawn candidates | unknown | Planned non-simulating R4 compatibility harness |
| Simulation | Fixed 30 Hz tick kernel and MAP-script runtime | unknown | Planned R5; not started |
| Gameplay | Navigation and build-harvest-combat slice | unknown | Planned R6; not started |
| Profiles | Legacy terrain UV/diagonal presentation policy | verified | Synthetic hashes + installed terrain smokes |
| Profiles | Modern terrain macro-variation policy | implemented | Deterministic full/streamed byte-equivalence tests |
