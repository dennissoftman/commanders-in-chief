# Licensing

The engine is dual-licensed **MIT OR Apache-2.0**, at your option. See [LICENSE-MIT](LICENSE-MIT) and
[LICENSE-APACHE](LICENSE-APACHE).

Contributions are accepted under the same terms, with a Developer Certificate of Origin sign-off and no
copyright assignment. See [CONTRIBUTING.md](CONTRIBUTING.md).

## Why this document exists

A permissive licence here was not a default. It was made available by deliberately removing a
derivation, and this file records that work so nobody has to reconstruct it — and so nobody undoes it
by accident.

## What the predecessor was, and why it could not be permissive

The project this engine replaces was GPL-3.0-**only** by obligation. Parts of it derived from
`GeneralsGameCode` revision `9f7abb866f5afd446db14149979e744c7216baaf`, which is GPL-3.0-or-later with
Electronic Arts Section 7 terms. Once any file derives from that, the whole distributed work inherits
the copyleft obligation and the choice of licence is gone.

Dropping legacy format support removed the obligation, but **only because the derived logic went with
it**. Deleting the parsers was not sufficient: derived *constants and policies* had leaked into
otherwise-original engine code, and each had to be found and removed individually.

## Audit result

Every file in this tree is project-authored and carries no `GeneralsGameCode` derivation. Verified by
sweep: no SPDX headers, no copyright notices, and no references to the predecessor's source anywhere in
code. A test in `cic-render` fails if a licence header is ever pasted back into a shader.

**Seeded clean, verbatim**

| File | Finding |
|---|---|
| `cic-core/src/binary.rs`, `lib.rs` | No derivation. |
| `cic-render/src/resource.rs` | No derivation. |
| `cic-render/src/terrain_virtual.rs` | No derivation. |
| 13 of 15 original WGSL shaders | No derivation. |

**Seeded after removing a derived region**

| File | What was removed |
|---|---|
| `cic-camera/src/lib.rs` | `RtsCameraProfile::GENERALS_DEFAULT` — its pitch, yaw, height and rate limits were the source's `GameData` camera fields. The camera *model* is original and was kept; only the constant table was replaced, and the three tests that asserted those constants were rewritten to assert invariants instead. |
| `cic-render/src/terrain_deferred.wgsl` | Its legacy camera layout and three-light `GameData` model. The lighting technique, the world-position-from-depth reconstruction, and the shadow and occlusion floor interaction are original and were kept with their reasoning. |
| `cic-render/src/terrain_ao.wgsl` | Same camera layout. The occlusion integral is from Jimenez et al., *Practical Realtime Strategies for Accurate Indirect Occlusion* (2016) — a published technique, cited in the file. |
| `boundary_viewer.wgsl`, `road_viewer.wgsl`, `terrain_viewer.wgsl` | The legacy camera struct, so no shader in the tree carries it. These three are staged for passes not yet built. |

**Deliberately not seeded — must be written from scratch**

| File | Why |
|---|---|
| `cic-render/src/scenery.rs` | Its sway defaults and ten sway families derive from `ScriptEngine.cpp` and `W3DTreeBuffer.cpp`. The instancing structure was original and is worth redoing; the constants must be re-authored. |
| `cic-render/src/water_viewer.wgsl` | Standing-water texture scale, tint and alpha, and depth-feather policy derive from `W3DWater.cpp`. The bounded screen and sky reflection in the same file *was* original work and can be salvaged into a re-authored shader. |

**Water rendering is on M3's remaining list.** Whoever writes it must not consult the original. This is
the single easiest way to silently reintroduce the obligation this document exists to record.

## Dependencies

Every transitive dependency is permissive — MIT, Apache-2.0, Zlib, BSD-2/3-Clause, ISC, Unlicense, CC0,
or a choice among them. None is copyleft, and none is unlicensed. Nothing in the dependency graph
constrains the licence either.

Two obligations follow from that and apply at release rather than now:

- **Attribution.** Permissive licences require their notices to accompany a binary. See
  [NOTICES.md](NOTICES.md), which is generated and must be regenerated when dependencies change.
- **Re-check on a dependency bump.** A new dependency is a new licence. The generator makes this a
  diff rather than an audit.

## Two obligations unrelated to code

- No Electronic Arts trademark or publicity rights are claimed. This project is independent and not
  affiliated with or endorsed by any game publisher.
- No retail game assets are distributed with this repository, and none may be.
