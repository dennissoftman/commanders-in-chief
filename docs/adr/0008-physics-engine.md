# ADR 0008: Physics — Jolt, and why it sits outside the deterministic kernel

- Status: **proposed.** Nothing is implemented and no milestone charter mentions physics yet. This records
  the intent, what it would cost, and the one question that has to be answered before any of it is built —
  because that question constrains M5, which starts next.

## Context

The intent is to use [Jolt Physics](https://github.com/jrouwe/JoltPhysics) when physics is implemented. It
is a strong choice on its merits: MIT-licensed, Copyright 2021 Jorrit Rouwé, and shipped in titles far
larger than this one.

It also arrives into a tree with three standing commitments that it interacts with, and none of the
interactions is obvious:

- **[ADR 0007](0007-simulation-arithmetic.md)** pins simulation arithmetic to `f64` and a restricted
  operation set, so that a lockstep run is bit-identical on every machine.
- **`unsafe_code = "forbid"`** at workspace scope in `Cargo.toml`. Jolt is C++, so reaching it means a
  foreign-function interface, which means `unsafe`.
- **`NOTICES.md` is generated from `cargo metadata`**, so it enumerates Rust crates and nothing else.

## The question that has to be answered first

**Is physics authoritative, or is it cosmetic?** Everything else follows from it, and it is a gameplay
decision rather than a technical one.

**Authoritative** means a physics result can decide something the simulation records — a projectile's
impact point, whether a collapsing building crushes a unit, where a vehicle ends up after a slope. Then
the physics step is part of the tick, part of the tick order, and part of the per-subsystem hash, and every
constraint in ADR 0007 applies to it.

**Cosmetic** means physics decides nothing. Debris, wreckage settling, a turret ring spinning as it falls,
a ragdoll: things a viewer sees and the simulation does not know about. Then physics is presentation, and
[ADR 0007's decision 9](0007-simulation-arithmetic.md) already says presentation is unrestricted.

## Decision (proposed)

1. **Physics is cosmetic.** Jolt runs in presentation, is stepped from the frame loop rather than the tick,
   and no value it produces is ever written into simulation state or consumed by a seeded stream.
2. **An authoritative effect is implemented in the kernel instead**, in ADR 0007's restricted arithmetic,
   and Jolt is told the answer rather than asked for it. A shell's trajectory that decides a hit is
   integrated by the simulation; the *explosion* is Jolt's. This is the same division the engine already
   draws between a simulated unit and its drawn model.
3. **`CROSS_PLATFORM_DETERMINISTIC` is therefore off**, which keeps the roughly 8% it costs. It goes on
   only if decision 1 is ever reversed, and reversing decision 1 is an amendment to this ADR rather than a
   configuration change.
4. **The `unsafe` policy is relaxed narrowly or not at all.** `forbid` cannot be overridden by an inner
   `allow`, so there is no per-crate escape from the workspace lint as it stands. The two available edits
   are to change the workspace level to `deny` — which *can* be overridden, crate by crate, deliberately —
   or to have the one binding crate not inherit `[lints] workspace = true`. Either way the relaxation
   reaches exactly one crate, that crate contains no gameplay logic, and every `unsafe` block in it carries
   a comment stating the invariant it is relying on.
5. **`NOTICES.md` learns about native dependencies**, because it will otherwise omit a licence the project
   is obliged to reproduce. `tools/generate-notices.py` reads `cargo metadata` and so cannot see a vendored
   C++ library at all; it needs a second, explicit list that a human maintains and a test checks is not
   empty when a native dependency is present.

## How Jolt's determinism actually works

Worth writing down even under decision 1, because it is the question that prompted this ADR and because
the answer is *better* than the surrounding ecosystem's.

Jolt has a build option for it. The define's own documentation is
`JPH_CROSS_PLATFORM_DETERMINISTIC - Turns on behavior to attempt cross platform determinism. If this is
set, JPH_USE_FMADD is ignored.` Reported conditions and costs, from the project's documentation: the same
*source* compiled on every platform with the same defines, about 8% slower, and tested across MSVC 2022,
clang, gcc and emscripten.

**It works by the same mechanism ADR 0007 chose**, which is worth noticing because it is independent
corroboration rather than agreement by coincidence. Jolt is deterministic across platforms because the
arithmetic is *its own*, compiled from one source everywhere, rather than delegated to whatever the
platform provides — and because fused multiply-add is switched off, since its availability varies. Those
are ADR 0007's decisions 3 and 6, arrived at separately.

Two things it excludes, and both matter more than the flag:

- **"The broadphase and multithreading are not deterministic."** The broad phase can be modified from
  several threads, so a `BroadPhaseQuery` result depends on scheduling. Under decision 1 that is
  irrelevant; under an authoritative reading it means no gameplay question may ever be asked of the broad
  phase, which is a sharp restriction on the most natural way to ask "what is near this".
- **The word is "attempt"**, and there is a `JPH_ENABLE_DETERMINISM_LOG` for debugging determinism issues
  when the flag is on. A facility for debugging a class of problem is evidence that the class occurs.

**And it is `f32` where ADR 0007 is `f64`.** `JPH_DOUBLE_PRECISION` exists but is a separate build-wide
choice, and mixing the two across a boundary would be the sort of silent conversion this project has
already been bitten by. Under decision 1 there is no boundary to mix across, because nothing comes back.

## Rationale

**Why cosmetic is the right default for this game specifically.** An RTS does not resolve gameplay through
rigid bodies. Units move by steering over navigable ground, combat resolves through ranges and rolls from
seeded streams, and construction and economy are bookkeeping. The physics a strategy game wants is almost
entirely *spectacle* — a destroyed tank that tumbles rather than vanishing — and spectacle is the thing
that may differ between two clients without either being wrong. Choosing authoritative physics would buy
gameplay nothing and pay for it with the hardest determinism problem in the project.

**Why this is worth deciding before M5 rather than when physics is written.** M5 defines the state layout,
the tick order, and the per-subsystem hashes. If physics is authoritative it is a subsystem in all three,
and retrofitting a subsystem into a hash contract means changing the contract. If it is cosmetic, M5 does
not have to know it exists. That asymmetry is the whole reason this file exists now.

**Why the `unsafe` question is not a formality.** `unsafe_code = "forbid"` is one of two lints the
workspace sets at the root, and the project's contribution rules name it as needing an accepted ADR to
change. The available Rust bindings — [`jolt-rust`](https://github.com/SecondHalfGames/jolt-rust), as
`joltc-sys` beneath a `rolt` wrapper — describe their own safety as provided on a best-effort basis, which
is honest and is exactly why the relaxation should reach one crate rather than the workspace.

**Why the build cost is real.** The tree currently builds with `cargo build` and no native toolchain. Jolt
needs a C++ compiler and CMake on every developer machine and on the CI runner, which is a new way for the
build to fail and a new thing to pin. That is a cost worth paying for a physics engine and not worth paying
by accident, so it belongs in this ADR rather than in a commit message.

## Alternatives

**[Rapier](https://rapier.rs/)** is the alternative worth naming rather than dismissing. It is pure Rust,
so it needs no `unsafe` relaxation, no C++ toolchain, and no change to how notices are generated — three of
the four costs above disappear. It documents cross-platform determinism as an option, on machines compliant
with IEEE 754-2008, and it shares an ecosystem with `simba`, which is where fixed-point scalars would come
from if ADR 0007 were ever reversed. Jolt is the more capable and more proven engine; Rapier is the one that
fits this tree's existing constraints without amending any of them. If the decision above is revisited,
this is the comparison to revisit.

**No physics engine at all.** Under decision 1 the requirement is tumbling debris and settling wreckage,
which is a small amount of purpose-written integration rather than a solver. Worth stating because it is the
cheapest option and it is genuinely sufficient for what decision 1 asks for — and because if the answer
turns out to be "we only ever wanted debris", adopting either engine was the wrong trade.

## Consequences

- **This ADR is `proposed`, and what would make it `accepted`** is a gameplay decision on authoritative
  versus cosmetic, taken with M6's charter in view. Until then M5 proceeds as though physics does not
  exist, which decision 1 makes correct rather than merely convenient.
- Physics belongs to **presentation** in the dependency graph, so it must not become something the
  simulation crate depends on. That is the same direction as every other boundary here and is worth
  stating because a physics engine is the most tempting place to break it.
- `docs/milestones/m6-gameplay.md` mentions physics nowhere. If decision 1 stands, it should say so
  explicitly rather than leaving the absence to be read as an omission.
- **The notices gap is a defect today, not only under this ADR.** Nothing in the tree links a native
  library yet, so nothing is currently missing — but the generator cannot report one, and that is worth
  fixing before the first one arrives rather than after.
