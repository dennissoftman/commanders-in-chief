# Licensing

## Scope: two licences, one boundary

| What | Licence | File |
|---|---|---|
| The engine — `crates/`, `tools/`, build configuration, format specs, engineering docs | **Apache-2.0** | [LICENSE](LICENSE) |
| The game — `docs/design/`, and narrative, art, audio and prose assets added later | **Reserved, all rights** | [LICENSE-CONTENT](LICENSE-CONTENT) |

The boundary follows the repository layout so that it stays checkable: a file that teaches you how the
engine works is Apache-2.0, and a file that tells you what the game *is* is reserved. Nothing under
`docs/design/` is needed to build or run the engine, so a redistributor can take the software under
Apache-2.0 without touching the reserved half at all.

The engine is meant to be reused. The world it was built to run is not, and saying so explicitly is what
lets the engine stay permissive without also giving away the fiction.

## Why Apache-2.0, and why alone

The Rust ecosystem's convention is dual `MIT OR Apache-2.0`, and that convention is about *publishing*:
offering MIT keeps a crate combinable with anything downstream. These crates are `publish = false`, so
the convention's benefit does not apply, and what remains to choose on is what the licence text actually
does. Apache-2.0 says four things MIT is silent on:

- **An explicit patent grant** (§3), with termination for anyone who initiates patent litigation over the
  work. MIT grants copyright permissions and says nothing about patents.
- **Inbound contributions arrive under the same terms** (§5) unless stated otherwise. This is the clause
  that makes a DCO sign-off sufficient and a CLA unnecessary — see [CONTRIBUTING.md](CONTRIBUTING.md).
- **Trademarks are reserved** (§6). Relevant here: this project must not be able to imply a publisher's
  endorsement, and the licence itself now says so.
- **Redistribution obligations that survive** (§4): keep the `NOTICE` file, and state your changes.
  Attribution that a downstream fork cannot quietly drop.

One licence also means one set of obligations to satisfy at packaging time rather than a choice to
re-explain every time someone asks.

A permissive licence was available here at all only because a derivation was removed. That is what the
rest of this document records.

## Why the audit section exists

A permissive licence was not a default. It was made available by deliberately removing a derivation, and
this file records that work so nobody has to reconstruct it — and so nobody undoes it by accident.

## What the predecessor was, and why it could not be permissive

The project this engine replaces was GPL-3.0-**only** by obligation. Parts of it derived from
`GeneralsGameCode` revision `9f7abb866f5afd446db14149979e744c7216baaf`, which is GPL-3.0-or-later with
Electronic Arts Section 7 terms. Once any file derives from that, the whole distributed work inherits the
copyleft obligation and the choice of licence is gone.

Dropping support for the predecessor's data formats removed the obligation, but **only because the
derived logic went with it**. Deleting the parsers was not sufficient: derived *constants and policies*
had leaked into otherwise-original engine code, and each had to be found and removed individually.

## The history carries two licences

This repository has two unrelated roots. The predecessor's line was merged in with `-s ours` so that its
commits stay reachable from `main` rather than being orphaned by a force-push, which means **a clone
carries the GPL history as well as the current tree**. That is deliberate. It is also not a
contradiction — but a reader does need to know which region they are holding.

| Region | Range | Licence |
|---|---|---|
| The predecessor | root `3e53e1c` … tip `2216924` — 86 commits | **GPL-3.0-only.** Carries the GPL text as `LICENSE.md`. |
| This engine, before a licence was chosen | `5e824cf` … `76b99d1` — 5 commits, all one day | No licence file. Retroactively offered under Apache-2.0, below. |
| This engine, dual-licensed | `91964e0` … `eb65483` | `MIT OR Apache-2.0`, as offered at the time. |
| This engine, current | `8839332` onward | **Apache-2.0.** |

`d58f31f` is the merge that joined the two. It took zero bytes from the predecessor's tree.

**Why the GPL region does not reach the current tree.** Copyleft attaches to derivative works, not to
commits that happen to share a version-control ancestor. Git ancestry is bookkeeping, not authorship, and
the question that decides a licence is only ever whether *this snapshot* contains or derives from the GPL
material. For the current tree it does not, which is what the audit below establishes file by file. The
content at `main` therefore has no path back to the predecessor; only the commit graph does. Were it
otherwise, deleting GPL code from a repository would be impossible and no project could ever change its
licence.

**The predecessor's snapshots stay GPL-3.0-only, permanently.** Anyone who checks out that region
receives it on those terms and may use and fork it accordingly. They are not this project's to
relicense: that obligation came from the third-party derivation rather than from any choice made here.
The same principle running the other way is why the unlicensed commits below *can* be granted after the
fact, and why the dual-licensed region keeps its MIT option for anyone already holding a copy. A licence
once given cannot be withdrawn.

**Retroactive grant.** Every snapshot in this project's own line, from its root `2115ce9` onward, is
offered under Apache-2.0. Five of those commits predate the licence file and so granted nothing at the
time; they were an unlicensed window of a few hours in a repository with no users, and for a sole
copyright holder a sentence like this one is all that closing it requires. No history was rewritten to
achieve it, and none should be: the seed history is the evidence that the derivation was removed, and
that record is worth considerably more than a tidy log.

### The rule this section exists to state

**Nothing may be copied backward across `5e824cf`.** No revert, no cherry-pick, no `git show` of a
predecessor file into a current one, from any commit in the GPL region into this tree.

That region still contains every constant removed on purpose: the camera profile, the standing-water
policy, and the scenery sway families. They are one command away for anyone who knows they are there, and
attaching the ancestry to `main` is what made them easy to reach by accident. A single pasted table
reinstates the copyleft obligation for every snapshot after it and silently voids the Apache-2.0 offer
this file records — silently, because nothing in the build would fail.

The risk is highest for whoever implements water and scenery, since the removed code is precisely the
code they would most like to consult. Do not consult it.

## Audit result

Every file in this tree is project-authored and carries no `GeneralsGameCode` derivation. Verified by
sweep: no SPDX headers, no copyright notices, and no reference to the predecessor's source anywhere in
code.

No source file asserts a licence of its own — the licence is declared once, in the workspace manifest.
That is deliberate rather than an omission: it makes *any* licence header appearing in this tree an
inherited one, which is precisely the copy-paste worth catching. A test in `cic-render`
(`no_shader_carries_an_inherited_licence_header`) fails if one is ever pasted back into a shader.

**Seeded clean, verbatim**

| File | Finding |
|---|---|
| `cic-core/src/binary.rs`, `lib.rs` | No derivation. |
| `cic-render/src/resource.rs` | No derivation. |
| `cic-render/src/terrain_virtual.rs` | No derivation. |
| 8 of the 13 seeded WGSL shaders | No derivation. |

**Seeded after removing a derived region**

| File | What was removed |
|---|---|
| `cic-camera/src/lib.rs` | `RtsCameraProfile::GENERALS_DEFAULT` — its pitch, yaw, height and rate limits were the source's `GameData` camera fields. The camera *model* is original and was kept; only the constant table was replaced, and the three tests that asserted those constants were rewritten to assert invariants instead. |
| `cic-render/src/terrain_deferred.wgsl` | Its inherited camera layout and three-light `GameData` model. The lighting technique, the world-position-from-depth reconstruction, and the shadow and occlusion floor interaction are original and were kept with their reasoning. |
| `cic-render/src/terrain_ao.wgsl` | Same camera layout. The occlusion integral is from Jimenez et al., *Practical Realtime Strategies for Accurate Indirect Occlusion* (2016) — a published technique, cited in the file. |
| `boundary_viewer.wgsl`, `road_viewer.wgsl`, `terrain_viewer.wgsl` | The inherited camera struct, so no shader in the tree carries it. These three are staged for passes not yet built. |

That accounts for all 13 shaders present at the seed: 8 clean and 5 with a region removed. The tree now
holds 16, the difference being `terrain_forward.wgsl`, `terrain_gbuffer.wgsl` and `model_gbuffer.wgsl`,
which were written afterwards against the native formats and have no predecessor.

**Deliberately not seeded — must be written from scratch**

These two paths do not exist in the tree. They are listed by the name their replacement should take.

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

Where a dependency offers Apache-2.0 among its options, this project takes it, matching the engine's own
licence rather than collecting a second set of obligations for no benefit.

Two obligations follow, and apply at release rather than now:

- **Attribution.** Permissive licences require their notices to accompany a binary. See
  [NOTICES.md](NOTICES.md), which is generated and must be regenerated when dependencies change. CI
  regenerates it and fails on a diff, so it cannot drift.
- **Re-check on a dependency bump.** A new dependency is a new licence. The generator makes this a diff
  rather than an audit.

### `NOTICE` and `NOTICES.md` are different files

Similar names, opposite directions, and worth keeping straight:

- **`NOTICE`** is *this project's* attribution, in the form Apache-2.0 §4(d) gives weight to. Anyone
  redistributing the engine must carry its contents.
- **`NOTICES.md`** is the *third-party* listing: what this project owes everyone else. Generated.

## Two obligations unrelated to code

- No Electronic Arts trademark or publicity rights are claimed. This project is independent and not
  affiliated with, endorsed by, or sponsored by any game publisher.
- No retail game assets are distributed with this repository, and none may be.
