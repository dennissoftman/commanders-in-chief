# Licensing posture

This tree deliberately carries **no license file yet**. That is the point of the reset.

## Why the previous tree was GPL-3.0-only

The predecessor repository was GPL-3.0-**only** by obligation, not by preference: parts of it
derived from `GeneralsGameCode` revision `9f7abb866f5afd446db14149979e744c7216baaf`, which is
GPL-3.0-or-later with Electronic Arts Section 7 terms. Once any file derives from that source, the
whole distributed work inherits the copyleft obligation, and the choice of license is gone.

Dropping legacy format support removes that obligation — but **only if the derived logic goes with
it**. Deleting the parsers is not sufficient; derived *constants and policies* were scattered into
otherwise-original engine code. Those were audited file by file before anything was seeded here.

## Audit result

Every file seeded into this tree is project-authored and carries no `GeneralsGameCode` derivation.

**Seeded clean, verbatim:**

| File | Finding |
|---|---|
| `cic-core/src/binary.rs`, `lib.rs` | No derivation markers. |
| `cic-render/src/resource.rs` | No derivation markers. |
| `cic-render/src/ui_text.rs` | No derivation markers. |
| `cic-render/src/terrain_virtual.rs` | No derivation markers. |
| 13 of 15 WGSL shaders | No derivation markers. |

**Seeded after stripping a derived region:**

| File | What was removed |
|---|---|
| `cic-camera/src/lib.rs` | `RtsCameraProfile::GENERALS_DEFAULT` — its pitch/yaw/height/rate limits were the source `GameData` camera fields. The camera *model* is original and was kept; only the constant table was replaced with a project-authored default. |

**Deliberately not seeded, pending clean reimplementation:**

| File | Why |
|---|---|
| `cic-render/src/scenery.rs` | Its sway defaults and ten sway families derive from `ScriptEngine.cpp` and `W3DTreeBuffer.cpp`. The instancing structure is original and worth redoing; the sway constants must be re-authored. |
| `cic-render/src/water_viewer.wgsl` | Standing-water texture scale, source tint/alpha, and depth-feather policy derive from `W3DWater.cpp`. The bounded screen/sky reflection in the same file *is* original project work and can be salvaged into a re-authored shader. |
| `cic-render/src/terrain_viewer.rs` | Only one derived detail — road texture mip count following `W3DRoadBuffer.cpp`. The remaining ~6,290 lines are original. Held back only because it depends on the not-yet-ported terrain pipeline, not because of the derivation, which is a two-line deletion. |

## Consequence

Nothing in this tree constrains the license. Permissive (MIT / Apache-2.0), copyleft, source-available,
or fully proprietary are all still available, including a commercial release.

Two obligations survive regardless of what is chosen here, because they are not about code:

- No Electronic Arts trademark or publicity rights may be claimed. The predecessor's `NOTICE.md`
  disclaimer of affiliation remains good practice for any project in this genre.
- No retail game assets may be redistributed.

Pick a license before the first public push. Until then every file header is intentionally bare.
