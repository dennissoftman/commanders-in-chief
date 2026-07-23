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
| Resources | Zero Hour Generals-base/delta profile | verified | Stable base-then-expansion mounts + installed layered exports + cumulative INI history tests |
| MAP | `CkMp` chunk inventory and unknown payload preservation | verified | Original synthetic fixture + installed RefPack MAP closure |
| MAP | `EAR\0` RefPack wrapper | verified | Synthetic back-reference/negative tests + installed MAP decompression |
| MAP | `HeightMapData` versions 1 through 4 | verified | Original fixture/signed-boundary tests + installed version-4 maps |
| MAP | `BlendTileData` versions 6 through 8 | verified | Original version-7 fixture, corrected-stride version-8 synthetic tests, and installed version-6/version-7/version-8 maps |
| MAP | Terrain INI resolution and layered terrain staging | verified | Synthetic ordered-history test/capture hashes + installed Generals-under-ZH class smoke |
| MAP | Water/river `PolygonTriggers` versions 2 through 4 | verified | Synthetic layer-name/truncation/seam reconstruction tests + installed lake/long-river/empty-marker smokes |
| MAP | Water transparency, standing texture, and source caustic inputs | verified | Synthetic constructor/history/map-overlay tests + installed texture/scalar/frame smokes |
| MAP | R3 water appearance baseline | verified | Forward depth/refraction/shoreline path, shared directional shadows, edge-aware AA, explicit-time hashes, and repeatable installed comparisons; exact D3D8 pixels excluded |
| MAP | `GlobalLighting` versions 1 through 3 | verified | Synthetic exact-layout/truncation tests + installed selected-time smoke |
| MAP | `WorldInfo` and `ObjectsList` placements | verified | Bounded synthetic layouts, truncation/limit tests, stable report, and immutable source-order staging |
| MAP | Roads and bridges | verified | Source-radius curves/miters, atlas joins, stacking, terrain fit, intact instanced bridges, retained body-state assets, and renderer-only tower scenery; runtime state transitions remain R5+ |
| MAP | R3 object draw-definition resolution and static scenery | verified | Default/initial-NONE W3D states, reskins, scales, standalone meshes, exact placement, Header3 culling, GPU instancing, and explicit-time default tree sway; unsupported/gameplay draw modules are excluded |
| MAP | Waypoints and `Player_n_Start` spawn candidates | verified | Immutable object projection preserves ordered candidates without assigning slots |
| MAP | `SidesList`, teams, and build lists | verified | Versions 1 through 3 decode under explicit limits without runtime repair or activation |
| MAP | Complete polygon-area semantics | verified | Versions 2 through 4 retain every source record/point under per-area and total limits; `map-water` remains a stable projection |
| MAP viewer | Waypoint paths, player starts, and polygon-zone diagnostics | verified | Stable path grouping/color/order, bounded terrain-following geometry, and installed waypoint/zone/start smokes |
| MAP | Nested player-script tree | verified | Bounded nested synthetic tree, truncation/limit tests, stable raw-opcode report; execution intentionally deferred to R5 |
| MAP viewer | R3 complete terrain scene and deterministic overview | verified | Source-topology roads, bridge towers, instanced models, default tree sway, boundary fence, shared shadows, AA, forward water, and repeatable explicit-time full-scene hashes |
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
