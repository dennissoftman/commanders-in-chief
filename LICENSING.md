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

## A licence choice that changed an engineering one: the audio backend

Worth recording because it is the first time the licence drove a design decision rather than merely being
satisfied by one, and because the reasoning generalises to anything else this engine might want to depend
on for a whole subsystem.

The requirement for audio was a *switchable* backend — FMOD, OpenAL, or another established library. Both
obvious candidates carry terms this project cannot require:

| Library | Terms | Consequence here |
|---|---|---|
| **FMOD** | Proprietary. Licensed per title, free only below a revenue threshold, attribution required in the shipped product. | Cannot be a dependency of an Apache-2.0 engine that anyone may fork and ship. |
| **OpenAL Soft** | LGPL-2.1. | Comfortable dynamically linked with a relinking path preserved; a real constraint statically, which is how a Rust binary would ordinarily want it. |

Either would be defensible as an *option*. Neither can be what the engine **requires** in order to make a
sound, because the README's licence claim has to hold for the person who clones the repository and builds
it — and "Apache-2.0, except you also need a per-title commercial licence before anything is audible" is
not that claim.

So the default implementation is a software mixer written from scratch, in
[`cic-audio`](crates/cic-audio/src/mixer.rs), depending on no audio library at all. It is about 700 lines
and it is the reason `NOTICES.md` did not change when audio landed. FMOD and OpenAL remain available behind
the same trait, as separate crates a project opts into — which also puts them outside the workspace's
`unsafe_code = "forbid"`, since either binding is FFI.

The engineering argument for the boundary is [ADR 6001](docs/adr/6001-audio-backend-boundary.md), and it
happens to reach the same place from a different direction: a sample-sink boundary would have made every
backend interchangeable and worthless, while a command boundary makes the from-scratch mixer a real
implementation rather than a fallback. The licence constraint and the design constraint agreed, which is
not always how this goes and is worth noting when it happens.

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
achieve it, and **none of the history this section describes may be**: the seed history is the evidence
that the derivation was removed, and that record is worth considerably more than a tidy log.

### One history rewrite, after the licence boundary

On 2026-07-28 the seven commits merged by pull requests #27 and #28 were rewritten once, to add the DCO
`Signed-off-by` trailers they had been merged without. They are recorded here because a rewrite of `main`
is exactly the kind of event this file exists to make legible rather than leave for someone to discover.

What it did and did not touch:

- **Every commit rewritten is newer than `3a69fa6`**, which is well after `8839332` — so no commit named in
  the table above changed, no region boundary moved, and the seed history that carries the audit evidence is
  untouched. The prohibition in the paragraph above is intact; this happened on the far side of it.
- **The trees are byte-identical.** `main`'s tree hash before and after the rewrite is
  `7bcb873c654d9a57240c8f839af63316d0879f0b` in both cases. Only commit metadata changed.
- **Both merge commits were preserved** rather than flattened, so the shape of the history still records
  which work arrived through which pull request. They carry no sign-off themselves, which is correct: GitHub
  authored them, and a DCO certification belongs to whoever wrote the contribution.
- The pre-rewrite tip is kept as the branch `backup/pre-signoff-main` at `9bae1b2`, and pull requests #27 and
  #28 still cite their original hashes. Those commits are unreachable from `main` but not destroyed.

Why it was worth doing at all: `CONTRIBUTING.md` requires a sign-off on every commit, and seven commits
without one is a gap in precisely the record the DCO exists to create. In a repository that documents a
removed derivation file by file, an auditable trail with a hole in it is worse than the cost of one
rewrite. Nothing enforces the requirement mechanically — CI runs formatting, lints, tests and the notices
check, and no DCO bot is installed — so it is a convention, and conventions want either enforcement or
honesty about their gaps.

### The rule this section exists to state

**Nothing may be copied backward across `5e824cf`.** No revert, no cherry-pick, no `git show` of a
predecessor file into a current one, from any commit in the GPL region into this tree.

That region still contains every constant removed on purpose: the camera profile, the standing-water
policy, and the scenery sway families. They are one command away for anyone who knows they are there, and
attaching the ancestry to `main` is what made them easy to reach by accident. A single pasted table
reinstates the copyleft obligation for every snapshot after it and silently voids the Apache-2.0 offer
this file records — silently, because nothing in the build would fail.

The risk was highest for whoever implemented water and scenery, since the removed code is precisely the
code they would most like to consult. Neither consulted it.

**Both are now written, and there is no outstanding case.** The rule above does not lapse with them: the
region still contains every removed constant, and a later change wanting a camera profile or a
standing-water policy would find them exactly as reachable as before.

## Audit result

Every file in this tree is project-authored and carries no `GeneralsGameCode` derivation. Verified by
sweep: no SPDX headers, no copyright notices, and no reference to the predecessor's source anywhere in
code.

No source file asserts a licence of its own — the licence is declared once, in the workspace manifest.
That is deliberate rather than an omission: it makes *any* licence header appearing in this tree an
inherited one, which is precisely the copy-paste worth catching. A test in `cic-render`
(`no_chunk_carries_an_inherited_licence_header`) fails if one is ever pasted back into a shader.

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

That accounts for all 13 shaders present at the seed: 8 clean and 5 with a region removed.

**The set has since been restructured, and the tally is worth restating because this table is the audit.**
Shaders are now assembled from composable chunks under `cic-render/src/shaders/`, and the tree holds 16 of
them. Six of the 8 *clean* seeded shaders were deleted as superseded dead code — `shader.wgsl`,
`terrain.wgsl`, `model.wgsl`, `terrain_shadow.wgsl`, `scene_shadow.wgsl` and `viewer.wgsl` — leaving `ui`
and `terrain_virtual` from that group. **All 5 of the region-removed shaders survive**, which is the half
that matters here: `terrain_ao`, `boundary_viewer`, `road_viewer` and `terrain_viewer` unchanged, and
`terrain_deferred.wgsl` split into the `scene`, `shadow`, `atmosphere`, `lighting` and `composite` chunks.
Splitting a file moves the region-removed code between files without reintroducing anything, and the
per-chunk licence-header test now runs over all 16.

The remaining 5 — `terrain_forward`, `terrain_gbuffer`, `model_gbuffer`, `water` and `antialias` — were
written after the seed against the native formats and have no predecessor.

**One of the two clean survivors is worth a note, because "cleared of derivation" is not the same as
"usable".** `terrain_virtual.wgsl` composes terrain pages from a tile atlas: per-cell material slots, blend
masks with orientation and diagonal codes, a 32-pixel edge-tile sheet, a macro lattice. This project's
terrain is a heightfield plus per-layer weight textures and has none of those things, which is why the file
is still staged and why wiring it means rewriting it against the native model rather than connecting it up.
The audit finding stands — there is no derivation in it — and it was also a shader written for a terrain this
engine does not have.

**It has since been replaced by one written against the native model**, composing a page from the
heightfield's per-layer weight textures and layer albedo, in the same blend the G-buffer already used. That
is a rewrite rather than a salvage: the tile atlas, the blend masks, the orientation and diagonal codes, the
edge-tile sheet and the macro lattice are all gone, because none of them describes anything this engine
builds. Nothing was carried across, and there was nothing worth carrying — a file whose every input is a
resource that does not exist has no reusable part.

**The shader set has grown again, and the tally is restated because this table is the audit.** It now holds
19 chunks: the 16 above plus `scenery` for the sway model, `motion` for the screen-space motion vector, and
`taa` for the temporal resolve. All three were written after the seed, from scratch, and have no
predecessor — the sway is documented in full above, and the other two implement techniques with no file to
copy from. The per-chunk licence-header test runs over all 19, and a second test now also fails on a
half-pixel framebuffer offset, which is a correctness tripwire rather than a provenance one.

**`antialias.wgsl` is worth a sentence of its own, because "FXAA" names a file as well as a technique.**
Timothy Lottes' `fxaa3_11.h` is the reference implementation everybody reaches for, it carries NVIDIA's
own licence terms, and it was **not** consulted — nor was any derivative or port of it. What is in the
tree is an independent construction: a luma gate with an absolute and a relative term, a Sobel pair for
edge orientation, and a blend weight built from the second difference across the edge, each derived in the
file with its reasoning beside it. It is FXAA in the sense that it is a single luminance-directed post
pass, which is a category rather than a codebase. Vendoring the real thing would be a deliberate change
with its own `NOTICE` entry; the same reasoning already ruled SMAA out of
[ADR 0005](docs/adr/0005-antialiasing-strategy.md), whose two precomputed lookup textures are data blobs
under someone else's terms.

**Deliberately not seeded — must be written from scratch**

These paths do not exist in the tree. They are listed by the name their replacement should take.

| File | Why | State |
|---|---|---|
| `cic-render/src/scenery.rs` | Its sway defaults and ten sway families derive from `ScriptEngine.cpp` and `W3DTreeBuffer.cpp`. The instancing structure was original and is worth redoing; the constants must be re-authored. | **Resolved** — re-authored, not recovered. See below. |
| `cic-render/src/water_viewer.wgsl` | Standing-water texture scale, tint and alpha, and depth-feather policy derive from `W3DWater.cpp`. The bounded screen and sky reflection in the same file *was* original work. | **Resolved** — replaced, not salvaged. See below. |

**Water was re-authored rather than recovered.** It landed as `cic-render/src/water.rs` plus a water
section in `terrain_deferred.wgsl`, and the removed file was not consulted. Nothing was salvaged from it,
including the reflection that would have been permissible: the sky reflection here evaluates the same two
sky constants the lighting pass already used, so it shares that pass's gradient rather than reproducing an
earlier one.

Every figure the surface uses is authored in the new code with its reasoning stated beside it: the tints,
the depth ramp, the shoreline feather, the wave spectrum, the Fresnel reflectance at normal incidence, and
the specular exponent. Two of them were arrived at by looking at captures rather than by reasoning ahead —
the wavelength ratios, because near-harmonic ones interfered into a visible lattice, and the specular
exponent, because a mirror-like value produced no highlight at all. That history is recorded in
[the M3 milestone](docs/milestones/m3-renderer.md) as evidence of independent derivation: the values are
where they are because of what this renderer's own frames showed.

**Scenery sway was re-authored rather than recovered.** It landed as `cic-render/src/scenery.rs` plus a
`scenery` shader chunk, and the removed file was not consulted. Nothing was salvaged from it, including the
instancing structure the table above notes was original: the sway rides on the per-instance data the model
batch already carried for its colour tint, which is a structure this tree arrived at for its own reasons
before any of this work started.

The replacement is deliberately *not* a table of ten families. It is four profiles, each a distinct physical
regime — stiff trunk, slack stem, bladed, and fixed — with anything between them reachable by constructor,
and that shape is itself part of the evidence: a set of four justified regimes is not a redrawing of a set
of ten tuned entries.

Every constant is derived in the file from a stated physical argument, and the arguments are what make the
derivation checkable rather than asserted. A cantilever's first mode shape is super-linear near its base,
which fixes the weight exponent at two. The steady share of the bend must exceed the oscillating share, or
the plant leans *into* the wind for part of every cycle — which fixes the split at 0.55 and 0.45 rather than
leaving it to taste, and a test asserts the constraint rather than the values. The response saturates
because nothing stops a scenario authoring an absurd wind and a vertex shader cannot refuse one. And the
flutter sits at 5.37 times the sway rather than 5 because *this* renderer was already caught by
near-harmonic ratios, when five summed water waves at related wavelengths interfered into a visible diamond
lattice — a reason that could only come from this tree's own history, recorded in
[the M3 milestone](docs/milestones/m3-renderer.md).

**Nothing is left that wants to break the rule.** The prohibition stands for its own sake: the region still
holds the camera profile, the standing-water policy, and the sway families, and a future change reaching
for any of them would reinstate the copyleft obligation as silently as ever.

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

## The interface typeface is authored here, and that is why

The one place a permissive tree usually acquires an obligation it did not intend is a font. A typeface is
a **binary asset with a licence of its own** — most of the good free ones are under the SIL Open Font
License, which is permissive but not the licence this project declares, and which carries reserved-font-name
and bundling terms that a packager has to satisfy separately. Vendoring one would mean this repository had
two sets of redistribution obligations instead of one, for the sake of drawing menu labels.

The alternative usually reached for — load whatever the operating system provides — is worse for a
different reason. It makes the rendered result depend on which machine drew it, and this project's
rendering verification is a byte comparison against committed reference images.

So `cic-render/src/text.rs` holds a typeface authored in this tree: ninety-five glyphs as line and arc
strokes on one integer grid, with a rasteriser that gives them width by distance. It is not derived from
any font, digitised from any specimen, or traced from any outline. Its limitation is stated in the module
and worth repeating here: it covers Latin, and a character with no glyph draws as a hollow box. Adding a
loaded-font path later is a change behind one type, and whoever does it inherits the licence question this
section exists to answer.

## Two obligations unrelated to code

- No Electronic Arts trademark or publicity rights are claimed. This project is independent and not
  affiliated with, endorsed by, or sponsored by any game publisher.
- No retail game assets are distributed with this repository, and none may be.
