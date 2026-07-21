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
| W3D | Compressed animation | unknown | Not implemented |
| W3D | glTF 2.0 preview export | verified | Synthetic CLI test + Blender 3.3 headless import |
| Resources | Generals install profile and Steam discovery | verified | Installed Steam export + synthetic `--game-dir` test |
| Resources | Zero Hour base/delta profile | verified | Installed layered `--zh` exports |
| MAP | Chunk inventory | unknown | Not started |
| Simulation | Fixed 30 Hz tick kernel | unknown | Not started |
| Profiles | `ZeroHourLegacy` policy set | unknown | Not started |
| Profiles | `Modern` policy set | unknown | Not started |
