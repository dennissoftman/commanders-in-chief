# BIG Format Provenance

## GeneralsGameCode evidence

- Repository: <https://github.com/TheSuperHackers/GeneralsGameCode>
- Revision: `9f7abb866f5afd446db14149979e744c7216baaf`
- File: `Core/GameEngineDevice/Source/StdDevice/Common/StdBIGFileSystem.cpp`
- Permanent link:
  <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/GameEngineDevice/Source/StdDevice/Common/StdBIGFileSystem.cpp>
- Upstream notice: Command & Conquer Generals Zero Hour; Copyright 2025 Electronic Arts
  Inc.; historical file notice also states copyright 2001-2003 Electronic Arts Inc.
- License: GNU GPL version 3 or later with the Electronic Arts Section 7 additional terms
  in the upstream repository's `LICENSE.md`.

## Corroborating OpenSAGE evidence

- Repository: <https://github.com/OpenSAGE/OpenSAGE>
- Revision: `588ac477367a0022adf29f20a084e8873014e6ce`
- File: `src/OpenSage.FileFormats.Big/BigArchive.cs`
- Permanent link:
  <https://github.com/OpenSAGE/OpenSAGE/blob/588ac477367a0022adf29f20a084e8873014e6ce/src/OpenSage.FileFormats.Big/BigArchive.cs>
- License: GNU GPL version 3.

## Independent EternalBig evidence

- Repository: <https://github.com/dennissoftman/eternalbig>
- Revision inspected: `cdcabab6ed2cbbcbcf453baf6c16f619736b540f`
- Original reader commit: `579e572b83822871a2c5ac77a69649063cd0ebb6`
- Original reader date: 2022-09-15
- File: `src/main/java/org/dennissoftman/format/bigf/BigfReader.java`
- Permanent link:
  <https://github.com/dennissoftman/eternalbig/blob/cdcabab6ed2cbbcbcf453baf6c16f619736b540f/src/main/java/org/dennissoftman/format/bigf/BigfReader.java>
- Copyright: Copyright (c) 2022 Den Softman.
- License: MIT.

EternalBig independently records the little-endian archive size, big-endian count and
offset fields, and `0x4C323331` archive-end marker. The integer is ASCII `L231`; source
comments spelling it `L321` are typos and are not repeated as format facts.

## Implementation record

The Rust code in `crates/cic-vfs/src/big.rs` was authored for this project from format
facts recorded in `docs/formats/big.md`. No C++ or C# source code was copied, translated
line by line, or imported. Names and API boundaries are native to this repository.

The upstream evidence was inspected on 2026-07-21. On the same date, 18 user-owned
Steam-distributed Generals BIG headers and file tables were inspected. Every declared
little-endian size matched its physical file. Directory trailers were absent in 3,
`L225` plus a zero word in 10, and `L231` plus a zero word in 5. No retail assets or
member content were copied or added.
