# W3D Format Provenance

## GeneralsGameCode evidence

- Repository: <https://github.com/TheSuperHackers/GeneralsGameCode>
- Revision: `9f7abb866f5afd446db14149979e744c7216baaf`
- Container files:
  - `Core/Libraries/Source/WWVegas/WWLib/chunkio.h`
  - `Core/Libraries/Source/WWVegas/WWLib/chunkio.cpp`
- Identifier file:
  - `GeneralsMD/Code/Libraries/Source/WWVegas/WW3D2/w3d_file.h`
- Static mesh readers:
  - `GeneralsMD/Code/Libraries/Source/WWVegas/WW3D2/meshmdlio.cpp`
  - `GeneralsMD/Code/Libraries/Source/WWVegas/WW3D2/meshgeometry.cpp`
- Permanent links:
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/Libraries/Source/WWVegas/WWLib/chunkio.h>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/Libraries/Source/WWVegas/WWLib/chunkio.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/GeneralsMD/Code/Libraries/Source/WWVegas/WW3D2/w3d_file.h>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/GeneralsMD/Code/Libraries/Source/WWVegas/WW3D2/meshmdlio.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/GeneralsMD/Code/Libraries/Source/WWVegas/WW3D2/meshgeometry.cpp>
- Upstream notice: Command & Conquer Generals Zero Hour; Copyright 2025 Electronic Arts
  Inc.; historical notices identify Westwood Studios.
- License: GNU GPL version 3 or later with the Electronic Arts Section 7 additional terms
  in the upstream repository's `LICENSE.md`.

The source establishes native 32-bit chunk type/size words, a payload-only 31-bit length,
the high-bit child-container flag, nested boundary accounting, W3D identifiers, the
116-byte Header3 layout, 12-byte vectors, 32-byte triangles, and header-driven record
counts.

## Runtime verification

On 2026-07-21, first chunk headers from 12 W3D members in user-owned Steam Generals BIG
archives were inspected in place. Hierarchy, animation, and mesh identifiers were present;
all 12 first chunks set the child-container bit, and several files had additional
top-level chunks after the first declared payload. A complete 113,980-byte W3D was then
parsed to exact closure as 525 chunk records. No retail bytes were copied.

On the same date, the semantic decoder read one normal-geometry version 4.2 mesh from that
user-owned member. Its header declared 24 vertices and 12 triangles; the vertex, normal,
and triangle payload lengths matched exactly and all 36 triangle indices were below 24.
No retail bytes, names, or float values were retained in the repository.

## Implementation record

The Rust implementations in `crates/cic-formats/src/w3d.rs` and `w3d_mesh.rs` were
authored for this project from the facts in `docs/formats/w3d.md`. No C++ source code was
copied, translated line by line, or imported. The immutable tree and mesh values,
structured errors, limits, exact-size checks, index validation, absolute offsets, and
unknown-payload preservation policy are native to this repository.
