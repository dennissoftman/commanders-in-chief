# Architecture

## Dependency direction

```
cic-core          (no dependencies)
   ├── cic-vfs    (+ flate2)
   │      └── cic-assets   (+ gltf, serde, serde_json)
   │             ├── cic-render   (+ cic-camera, cic-ui, wgpu, png, sha2;
   │             │                   naga for shader validation in tests,
   │             │                   cic-sim in examples and tests only)
   │             ├── cic-sim
   │             └── cic-texconv  (+ png; an offline binary, not a library)
   └── cic-audio  (+ serde, serde_json)

cic-camera        (no dependencies)
cic-math          (no dependencies)
   └── cic-script
cic-ui            (+ serde, serde_json)
```

`cic-texconv` is a tool rather than a stage of the engine: it converts authored PNGs to the
block-compressed DDS textures the renderer uploads ([ADR 2001](docs/adr/2001-block-compressed-textures.md)).
It depends on `cic-assets` for the container writer and — deliberately — for the block *decoders*, which its
tests measure its encoders against. Nothing depends on it, and the runtime carries no compressor.

`cic-sim` consumes `cic-assets` for the same reason and in the same direction as the renderer does:
scenario activation reads the authored formats, and assets never know about simulation. It takes
`cic-math` the day a subsystem needs a transcendental — the direction the extraction anticipated. What
it must never take is anything that renders, plays, or reads a clock.

Four crates deliberately depend on nothing of this project's: `cic-core`, because bounded reading is a
primitive; `cic-camera`, because the same camera must drive the game, the editor, and any debug viewer
without dragging a window system into each of them; `cic-ui`, for the same reason one step further
out; and `cic-math`, because it holds the arithmetic everything inside the simulation must share — the
script VM today, the kernel next — and one answer can only sit below every crate that could otherwise
disagree about it. `cic-script` depends on exactly that crate and nothing else: a language that could
reach the engine would be one whose sandbox was a matter of what it happened not to call, and a maths
crate with no I/O is nothing a script can escape through. A layout solver that pulled in a graphics stack would make every tool that wants to position a box
depend on one, so the two facts it cannot derive — how large a piece of text is, and what the display
scale is — arrive from the caller instead.

`cic-render` depends on `cic-ui` and never the reverse, which is the same rule as for assets and holds
for the same reason. The renderer draws an interface; an interface model that knew how it was drawn would
drag a graphics stack into every editor and debug tool that wanted to lay out a box. The split is drawn
where the mistakes are rather than at the crate boundary for its own sake: which colour a focused button
takes and where a slider's knob sits are arithmetic and live in `cic-ui`, testable by asserting on a list;
what is left in `cic-render` is a glyph rasteriser, a vertex buffer and a draw call.

`cic-render` depends on `cic-assets`, and that direction is the one to keep: the renderer consumes
assets, and assets never know about rendering. Nothing in `cic-assets` mentions a GPU, a texture format,
or a pipeline, and the water surface is the clearest illustration — where a body of water *is* lives in
the renderer rather than in the terrain container, because tint and wave scale are things an artist
changes without touching a map.

`cic-audio` takes `cic-core` for its bounded WAV reader and nothing else. It has **no audio dependency of
any kind** — no device library, no decoder, no mixer — because [ADR
6001](docs/adr/6001-audio-backend-boundary.md) puts the replaceable half behind a trait and implements one
from scratch in the tree. FMOD is proprietary and OpenAL Soft is LGPL, and neither can be what a
permissively licensed engine *requires* in order to make a sound.

## Where content lives

`content/` holds authored game data that is not code: at present the four interface screens and their
string table. It is a plain directory rather than a package because the resource layer already reads
loose files and packages interchangeably — that is what `cic-vfs`'s ordered mounts are for — so a
directory during development and a `.cicmap` at release are the same code path. Nothing above the
resource layer knows which it got.

It is separate from `crates/` because the boundary is a licence boundary as well as a layering one: see
[LICENSING.md](LICENSING.md), which puts the engine under Apache-2.0 and reserves the game. What is in
`content/` today is structural rather than narrative — a layout is engineering — but the directory is
where narrative content will arrive, and the split is cheaper to keep than to introduce later.

## Layering rules

**Nothing above the resource layer opens a file.** Every asset arrives as bytes through `cic-vfs`, which
is what makes mods, packages, and loose development files interchangeable. A decoder that took a path
would work for exactly one of those three.

**Decoders take limits, they do not choose them.** The caller decides how much memory a load may use,
so the editor can be generous and a multiplayer client strict, running identical code.

**Presentation may read simulation state; it may never advance or mutate it.** This is not stylistic.
Frame rate varies per machine, so anything a frame can change is something that desyncs.

**Presentation may never draw from a simulation random stream.** The less obvious half of the rule above,
and audio is where it bites: variant selection and pitch spread need randomness on every gunshot, and
drawing from a stream is part of the simulation's state transition — so a machine whose audio consumed one
extra number has desynced. `cic-audio` therefore carries its own stream, seeded separately.

**Semantic input, not device input.** The camera takes intents, not key codes; UI callbacks are typed
events from a fixed set, not handlers named by data. Layout files and mods are data, and data must not
be able to name an action the engine did not define.

## Why the split between authored and bulk data

The asset formats look inconsistent until you notice what the line is. A scenario is small, edited by
hand, reviewed in diffs, and merged between people — so it is JSON. A heightfield is large, generated by
tools, read only by machines, and needs to reach the GPU without conversion — so it is tight binary with
`u16` samples that upload as a baseline `R16Uint` texture. Geometry is a solved problem with an ecosystem — so
it is glTF.

Choosing one format for all three would optimise for uniformity, which is not a property anyone
benefits from, at the cost of diffability, size, or tooling — all of which someone does.

## Where determinism is enforced

The resource layer, because path resolution and mount ordering feed everything downstream: ordered maps
not hash maps, explicit mount order, no dependence on directory enumeration order.

The simulation kernel, `cic-sim`, for the reasons in
[docs/invariants/determinism.md](docs/invariants/determinism.md) — and structurally where it can be:
advancing state requires a `TickContext` only `Kernel::advance` constructs, identifiers come from a
hashed counter, random streams are named and registered up front, and every subsystem's state is
folded into a per-tick hash so the invariants are *checked* per tick rather than trusted.

**The scripting language**, which inherits ADR 0007 rather than restating it: scripts run inside the
simulation, so the same restricted operation set binds them. Two mechanisms enforce it — the textual scan
decision 8 requires, and the stronger structural one that the bytecode has no instruction for a forbidden
operation. See [ADR 7001](docs/adr/7001-scripting-language.md). The transcendentals themselves live in
`cic-math`, one crate below every simulation-side consumer, so the script VM and the kernel cannot end up
holding two implementations that disagree in the last bit.

Everything between is presentation and is free to be as machine-dependent as it likes, which
[ADR 0007](docs/adr/0007-simulation-arithmetic.md) decision 9 states explicitly. `cic-audio` uses floating
point freely for exactly that reason — and is bound by the *other* half of the rule instead, which is that
it must never draw from a simulation random stream.

## Testing posture

Unit tests live beside the code they cover. Fixtures are **built**, not committed as blobs: the zip, tar,
and glTF tests construct real containers at test time. That costs a fixture builder per format and buys
tests that state the structural case they care about — a straddled block boundary, a trailing archive
comment, a declared expansion that would exhaust memory — legibly enough to review.

Rendering is the exception to "tests are enough". A green suite coexists comfortably with a visibly
broken frame, which is why M3 treats capture-based visual regression as a deliverable rather than as
follow-up work.

Audio has the same hazard and gets off much more cheaply, which is worth naming because it looks like luck
and is not. A green suite coexists just as comfortably with a mix nobody would ship. What saves it is that
the in-tree mixer is a **pure function** from voices and a listener to frames, so a property can be
asserted about the samples themselves — that a crossing sound holds constant power, that a limiter holds
eight times full scale below one at five separate frequencies, that a reverb's comb lengths are pairwise
prime. Those are assertions a picture cannot make, and they exist because the boundary in
[ADR 6001](docs/adr/6001-audio-backend-boundary.md) left the mixing on this side of it. What still needs
listening to is what needs a device, which is [M9](docs/milestones/m9-audio.md)'s one open item.

That harness now exists, and two of its properties are structural rather than incidental. The comparison
is a pure function over bytes — the library never opens a file, so the caller supplies the reference and
the file handling stays in the tests — which also means the comparison is unit-tested on machines with no
GPU at all. And **references are committed per adapter**, because two GPUs do not agree to the byte and
a tolerance loose enough to span them would accept the regressions it exists to catch.

Since the harness now runs on a CI runner as well, that second claim is measured rather than assumed —
and it holds for a narrower reason than it was given. Between an NVIDIA card and a software rasteriser,
the scenes that sample no texture agree well inside the tolerance; the two that sample one exceed it by
3.5x and 114x. Mip selection is what separates two implementations, not arithmetic in the last place, so
the per-adapter split earns its keep wherever a texture is filtered and hardly anywhere else. See
[`regression`](crates/cic-render/src/regression.rs).
