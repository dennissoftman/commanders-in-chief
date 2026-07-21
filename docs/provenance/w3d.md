# W3D Format Provenance

## GeneralsGameCode evidence

- Repository: <https://github.com/TheSuperHackers/GeneralsGameCode>
- Revision: `9f7abb866f5afd446db14149979e744c7216baaf`
- Container files:
  - `Core/Libraries/Source/WWVegas/WWLib/chunkio.h`
  - `Core/Libraries/Source/WWVegas/WWLib/chunkio.cpp`
- Identifier file:
  - `GeneralsMD/Code/Libraries/Source/WWVegas/WW3D2/w3d_file.h`
- Permanent links:
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/Libraries/Source/WWVegas/WWLib/chunkio.h>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/Libraries/Source/WWVegas/WWLib/chunkio.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/GeneralsMD/Code/Libraries/Source/WWVegas/WW3D2/w3d_file.h>
- Upstream notice: Command & Conquer Generals Zero Hour; Copyright 2025 Electronic Arts
  Inc.; historical notices identify Westwood Studios.
- License: GNU GPL version 3 or later with the Electronic Arts Section 7 additional terms
  in the upstream repository's `LICENSE.md`.

The source establishes native 32-bit chunk type/size words, a payload-only 31-bit length,
the high-bit child-container flag, nested boundary accounting, and W3D identifiers.

## Runtime verification

On 2026-07-21, first chunk headers from 12 W3D members in user-owned Steam Generals BIG
archives were inspected in place. Hierarchy, animation, and mesh identifiers were present;
all 12 first chunks set the child-container bit, and several files had additional
top-level chunks after the first declared payload. A complete 113,980-byte W3D was then
parsed to exact closure as 525 chunk records. No retail bytes were copied.

## Implementation record

The Rust implementation in `crates/cic-formats/src/w3d.rs` was authored for this project
from the facts in `docs/formats/w3d.md`. No C++ source code was copied, translated line by
line, or imported. The immutable tree, structured errors, limits, absolute offsets, and
unknown-payload preservation policy are native to this repository.
