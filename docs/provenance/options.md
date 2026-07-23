# Options INI Format Provenance

## GeneralsGameCode evidence

- Repository: <https://github.com/TheSuperHackers/GeneralsGameCode>
- Revision: `9f7abb866f5afd446db14149979e744c7216baaf`
- Files:
  - `Core/GameEngine/Source/Common/OptionPreferences.cpp`
  - `Core/GameEngine/Include/Common/OptionPreferences.h`
  - `Generals/Code/Libraries/Source/WWVegas/WW3D2/ww3d.h` (`MultiSampleModeEnum`)
- Permanent source links:
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/GameEngine/Source/Common/OptionPreferences.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/GameEngine/Include/Common/OptionPreferences.h>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Generals/Code/Libraries/Source/WWVegas/WW3D2/ww3d.h>
- Upstream notice: Command & Conquer Generals Zero Hour; Copyright 2025 Electronic Arts
  Inc.; historical file notice also states copyright 2001-2003 Electronic Arts Inc.
- License: GNU GPL version 3 or later with the Electronic Arts Section 7 additional terms
  in the upstream repository's `LICENSE.md`.

`OptionPreferences` is a flat `key = value` map (`UserPreferences`) with no section headers,
loaded from `Options.ini` (or `Options_Instance<NN>.ini` for secondary client instances) in the
per-user Documents data directory. The source establishes:

- Every boolean preference (`LanguageFilter`, `SendDelay`, `UseAlternateMouse`,
  `BuildingOcclusion`, `DynamicLOD`, `ExtraAnimations`, `HeatEffects`, `Retaliation`,
  `ShowSoftWaterEdge`, `ShowTrees`, `UseCloudMap`, `UseDoubleClickAttackMove`, `UseLightMap`,
  `UseShadowDecals`, `UseShadowVolumes`, `DrawScrollAnchor`, `MoveScrollAnchor`, and others) uses
  one uniform rule: an exact case-insensitive `"yes"` is true, and everything else — `"no"`,
  garbage, or a field left blank — is false. Retail files do leave some of these fields blank
  (`DrawScrollAnchor`/`MoveScrollAnchor` in particular), which only makes sense under this
  always-false-unless-"yes" rule rather than a stricter yes/no dichotomy.
- `getAntiAliasing()` reads `AntiAliasing` with `atoi`, clamps the result to
  `WW3D::MULTISAMPLE_MODE_NONE..=WW3D::MULTISAMPLE_MODE_8X` (`0..=8`, per `ww3d.h`), then retains
  only the highest set bit (`highestBit`), yielding one of `0`, `2`, `4`, or `8` active MSAA
  samples. A raw value of `5` therefore resolves to 4x MSAA, not 5x.
- `getResolution()` reads `Resolution` with `sscanf(str, "%d%d", &x, &y)`; retail files store it
  as two whitespace-separated integers (for example `Resolution = 3840 2160`).
- `IPAddress` and `GameSpyIPAddress` are free-form strings compared against enumerated adapter
  addresses; they are not validated as IPv4 dotted-quad text by this file.
- `IdealStaticGameLOD` and `StaticGameLOD` are free-form strings resolved through
  `TheGameLODManager`'s name table rather than a fixed enum in this file, so this project's
  decoder retains them as opaque strings instead of a fixed set of named levels.

Unlike this file's `atoi`/`atof`-based reader — which never fails a parse and silently falls back
to engine defaults for missing or garbage values — the project's decoder in
`crates/cic-formats/src/options_ini.rs` validates numeric fields explicitly and reports
resource-limit and value failures, consistent with this repository's other bounded INI decoders
(see `docs/provenance/map.md` for `water_ini.rs`/`terrain_ini.rs`). Field *names* and the
boolean/MSAA/resolution *semantics* above are taken from the source; the strict validation and
diagnostics behavior is project-authored.

## Runtime verification

On 2026-07-23, real retail `Options.ini` files from user-owned Generals and Zero Hour
installations were inspected in place. Both were flat, unquoted `key = value` files with no
section headers, matching every field name and value convention recorded above (including blank
`DrawScrollAnchor`/`MoveScrollAnchor` values and an `AntiAliasing` value that only resolves to a
power-of-two MSAA sample count after the clamp-and-highest-bit rule). No retail file bytes are
copied into this repository; the test fixture in `options_ini.rs` is an author-written synthetic
file using the same field vocabulary.

## Implementation record

The Rust implementation in `crates/cic-formats/src/options_ini.rs` was authored for this project
from the format facts recorded above. No C++ source code was copied, translated line by line, or
imported. Its types, bounded resource limits, and strict-validation error model are native to this
repository.
