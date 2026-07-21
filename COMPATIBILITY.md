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
| W3D | Chunk inventory | unknown | Not started |
| MAP | Chunk inventory | unknown | Not started |
| Simulation | Fixed 30 Hz tick kernel | unknown | Not started |
| Profiles | `ZeroHourLegacy` policy set | unknown | Not started |
| Profiles | `Modern` policy set | unknown | Not started |
