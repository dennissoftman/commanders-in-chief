# CSF Format Provenance

## GeneralsGameCode evidence

- Repository: <https://github.com/TheSuperHackers/GeneralsGameCode>
- Revision: `9f7abb866f5afd446db14149979e744c7216baaf`
- Files:
  - `Core/GameEngine/Source/GameClient/GameText.cpp`
  - `Core/GameEngine/Include/GameClient/GameText.h`
- Permanent source link:
  <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/GameEngine/Source/GameClient/GameText.cpp>
- Upstream notice: Command & Conquer Generals Zero Hour; Copyright 2025 Electronic Arts
  Inc.; historical file notice also states copyright 2001-2003 Electronic Arts Inc.
- License: GNU GPL version 3 or later with the Electronic Arts Section 7 additional terms
  in the upstream repository's `LICENSE.md`.

The source establishes the `CSF`, `LBL`, `STR`, and `STRW` integer tags, version 3 header,
record layout, complemented wide-character encoding, optional wave-name bytes, and the
version 2 language-field boundary. The source client's use of only the first string variant
is not imposed on the format IR; all declared variants are retained.

## Runtime verification

On 2026-07-21, the CSF member in a user-owned Steam installation of Generals was inspected
in place through its BIG entry boundaries. Its 24-byte header reported version 3, 2,806
labels, 2,805 strings, reserved value zero, and language zero. A bounded structural pass
reached exactly byte 282,246, observed all documented tags, found one zero-string label,
and found no case-insensitive duplicate label. No retail bytes or strings were copied into
the repository.

## Implementation record

The Rust implementation in `crates/cic-formats/src/csf.rs` was authored for this project
from the format facts recorded in `docs/formats/csf.md`. No C++ source code was copied,
translated line by line, or imported. Its types, error model, limits, lossless raw-name
policy, and preservation of all variants are native to this repository.
