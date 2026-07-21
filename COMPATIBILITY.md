# Compatibility Matrix

Status values are `unknown`, `observed`, `implemented`, and `verified`. `Verified`
requires synthetic fixtures plus comparison against legally obtained game data or
observable behavior.

| Area | Capability | Status | Evidence |
|---|---|---|---|
| VFS | ASCII case-insensitive virtual paths | verified | Unit tests + GitHub CI run 29840005186 |
| VFS | Deterministic loose-directory overlays | verified | Unit/CLI tests + GitHub CI run 29840005186 |
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
| Renderer | Material pass/stage submission | unknown | Next renderer gate |
| Renderer | Rigid/one-bone hierarchy bind pose | verified | Synthetic skinned fixture + installed rigid building capture |
| Renderer | Animated hierarchy pose | unknown | Explicit-frame sampling not connected yet |
| Resources | Generals install profile and Steam discovery | verified | Installed Steam export + synthetic `--game-dir` test |
| Resources | Zero Hour base/delta profile | verified | Installed layered `--zh` exports |
| MAP | Chunk inventory | unknown | Not started |
| Simulation | Fixed 30 Hz tick kernel | unknown | Not started |
| Profiles | `ZeroHourLegacy` policy set | unknown | Not started |
| Profiles | `Modern` policy set | unknown | Not started |
