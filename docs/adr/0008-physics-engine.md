# ADR 0008: Physics — cosmetic, and Rapier rather than Jolt

- Status: accepted. Nothing is implemented and no milestone charter schedules physics; what is settled is
  where it goes when it arrives, which is what M5 needed to know before defining its state layout.

## Context

[Jolt Physics](https://github.com/jrouwe/JoltPhysics) was the starting intent, and it is a strong choice on
its merits: MIT-licensed, Copyright 2021 Jorrit Rouwé, shipped in titles far larger than this one, and with
a documented cross-platform determinism mode — which is more than most of this ecosystem offers.

It arrives into a tree with three standing commitments it interacts with, and none of the interactions is
obvious:

- **[ADR 0007](0007-simulation-arithmetic.md)** pins simulation arithmetic to `f64` and a restricted
  operation set, so a lockstep run is bit-identical on every machine.
- **`unsafe_code = "forbid"`** at workspace scope. Jolt is C++, so reaching it means a foreign-function
  interface, which means `unsafe`.
- **`NOTICES.md` is generated from `cargo metadata`**, so it enumerates Rust crates and nothing else.

## The question that decided everything else

**Is physics authoritative, or is it cosmetic?** A gameplay decision rather than a technical one, and the
one this ADR was opened to force.

**Authoritative** would mean a physics result can decide something the simulation records — a projectile's
impact point, whether a collapsing building crushes a unit. The physics step would be part of the tick, the
tick order, and the per-subsystem hash, and every constraint in ADR 0007 would apply to it.

**Cosmetic** means physics decides nothing: debris, wreckage settling, a turret ring spinning as it falls.
Things a viewer sees and the simulation does not know about.

It is **cosmetic**, on the grounds that an RTS should not be dour about its realism.

## Decision

1. **Physics is cosmetic.** It runs in presentation, is stepped from the frame loop rather than the tick,
   and no value it produces is ever written into simulation state or consumed by a seeded stream.
   [ADR 0007's decision 9](0007-simulation-arithmetic.md) already says presentation is unrestricted, so this
   needs no new licence — it takes one that exists.
2. **An authoritative effect is implemented in the kernel instead**, in ADR 0007's restricted arithmetic,
   and the engine is *told* the answer rather than asked for it. A shell's trajectory that decides a hit is
   integrated by the simulation; the *explosion* is the engine's. This is the same division already drawn
   between a simulated unit and its drawn model.
3. **And physics is free to exaggerate**, which is the part of decision 1 that is a gain rather than a
   concession. Once a result decides nothing it no longer has to be *right*: a wreck may tumble further than
   its mass allows, a hit may throw debris harder than the round carried, and neither is a bug. A physics
   engine held to authoritative standards has to be plausible; one held to none can be **legible**, which is
   the more useful property at the distance an RTS is played from.
4. **[Rapier](https://rapier.rs/) rather than Jolt**, *because* of decision 1 — see the rationale, which is
   the whole of the argument. Pure Rust, so no foreign-function interface.
5. **`unsafe_code = "forbid"` stays exactly as it is.** No relaxation, no per-crate exemption, nothing to
   amend. This was the largest cost the original intent carried, and decision 4 removes it rather than
   managing it.
6. **No cross-platform determinism mode is enabled**, in either engine. Under decision 1 there is nothing to
   keep in step, and the mode costs performance to buy a property nothing needs. Turning it on is an
   amendment to decision 1 rather than a configuration change.

## Rationale

**Why cosmetic is right for this game specifically.** An RTS does not resolve gameplay through rigid bodies.
Units move by steering over navigable ground, combat resolves through ranges and rolls from seeded streams,
and construction and economy are bookkeeping. The physics a strategy game wants is almost entirely
*spectacle*, and spectacle is precisely the thing that may differ between two clients without either being
wrong. Choosing authoritative physics would buy gameplay nothing and pay for it with the hardest determinism
problem in the project.

**Why this had to be decided before M5 rather than when physics is written.** M5 defines the state layout,
the tick order, and the per-subsystem hashes. If physics is authoritative it is a subsystem in all three, and
retrofitting a subsystem into a hash contract means changing the contract. Because it is cosmetic, M5 does
not have to know it exists. That asymmetry is the reason this file was written before any physics was.

**Why the cosmetic decision settled the engine, rather than a separate preference doing it.** Jolt's clearest
advantage for this project was its determinism story: a build option tested across four compilers, which
works by exactly the mechanism ADR 0007 arrived at independently. Under decision 1 that advantage is worth
**nothing** — there is no property to guarantee, so the guarantee is not a feature. What remains on each side
is:

- Jolt is the more capable and more proven engine, and it costs an `unsafe` relaxation, a C++ toolchain on
  every developer machine and on the runner, and a change to how notices are generated.
- Rapier is pure Rust and costs none of those.

Debris and settling wreckage are not where a solver's capability is the binding constraint. So the axis Jolt
led on stopped mattering, and the axis Rapier leads on is the only one left — which makes this a decision
taken by the earlier decision rather than by taste. Recording *that* is the point of this section: if physics
ever becomes authoritative the argument inverts, and Jolt's determinism mode is the reason to revisit.

**Why `unsafe` was never a formality.** `unsafe_code = "forbid"` is one of two lints the workspace sets at
the root, and `CONTRIBUTING.md` names it as needing an accepted ADR to change. The mechanics are worth
recording even though decision 5 avoids them: `forbid` **cannot** be overridden by an inner `allow`, so
there is no per-crate escape as the workspace stands. Relaxing it would have meant changing the root to
`deny` — which *can* be overridden, deliberately, crate by crate — or having one crate not inherit
`[lints] workspace = true`. Either edit reaches further than a physics binding should.

## What Jolt's determinism mode does, and why it is still worth knowing

Recorded because it is the reason to reopen this ADR, and because it corroborates another one.

`JPH_CROSS_PLATFORM_DETERMINISTIC` is described by its own documentation as turning "on behavior to attempt
cross platform determinism. If this is set, JPH_USE_FMADD is ignored." Reported conditions and costs, from
the project's documentation: the same *source* compiled on every platform with the same defines, about 8%
slower, tested across MSVC 2022, clang, gcc and emscripten.

**It works by the same mechanism ADR 0007 chose**, which is independent corroboration rather than agreement
by coincidence. Jolt is deterministic across platforms because its arithmetic is *its own*, compiled from one
source everywhere rather than delegated to whatever the platform provides, and because fused multiply-add is
switched off since its availability varies. Those are ADR 0007's decisions 3 and 6, reached separately.

Two exclusions matter more than the flag, and both are why an authoritative reading would have been hard
rather than merely costly:

- **"The broadphase and multithreading are not deterministic."** The broad phase can be modified from several
  threads, so a `BroadPhaseQuery` result depends on scheduling. Under an authoritative reading no gameplay
  question could ever be asked of the most natural way to ask *what is near this*.
- **The word is "attempt"**, and there is a `JPH_ENABLE_DETERMINISM_LOG` for debugging determinism issues when
  the flag is on. A facility for debugging a class of problem is evidence the class occurs.

**And it is `f32` where ADR 0007 is `f64`.** `JPH_DOUBLE_PRECISION` exists but is a separate build-wide
choice, and mixing widths across a boundary is the sort of silent conversion this project has been bitten by
before. Rapier is `f32` by default too, with an `f64` build; under decision 1 there is no boundary to mix
across, because nothing comes back.

Rapier documents cross-platform determinism as an option of its own, on machines compliant with IEEE
754-2008, and shares an ecosystem with `simba`, which is where fixed-point scalars would come from if ADR
0007 were ever reversed. So decision 4 does not close the authoritative door; it just does not open it.

## Alternatives

**Jolt**, above, in full, because it was the starting intent and is the better engine on capability. It is
the alternative to return to if decision 1 is ever amended.

**No physics engine at all.** Under decision 1 the requirement is tumbling debris and settling wreckage,
which is a small amount of purpose-written integration rather than a solver. Worth stating because it is the
cheapest option and genuinely sufficient for what decision 1 asks for — and because if the requirement turns
out to be only debris, adopting *either* engine was the wrong trade. Decision 4 should be read as "when a
solver is wanted, this one", not as a commitment to want one.

## Consequences

- **M5 proceeds as though physics does not exist**, which decision 1 makes correct rather than merely
  convenient. Nothing about the state layout, the tick order or the per-subsystem hashes has to reserve room
  for it.
- **Tuning physics is not a balance change.** Because nothing it produces reaches simulation state, the
  numbers governing how dramatic a collapse looks can be changed by whoever is making it look good, without
  a replay breaking or a hash moving. That puts spectacle on the far side of the line from anything needing
  review for fairness.
- **Physics belongs to presentation in the dependency graph**, so it must not become something the simulation
  crate depends on. Worth stating because a physics engine is the most tempting place to break that
  direction.
- **The workspace's lints are untouched, and the build still needs no native toolchain.** Both follow from
  decision 4 and both are worth more than they look: they are the difference between `cargo build` and
  `cargo build` plus a C++ compiler pinned on three platforms.
- **The notices generator still cannot see a native dependency.** `tools/generate-notices.py` reads
  `cargo metadata`, so a vendored C++ library's licence would simply not appear. Decision 4 means nothing is
  missing *today* and nothing is expected to be — but the gap is latent rather than closed, and it is worth
  fixing before the first native dependency arrives rather than after.
- `docs/milestones/m6-gameplay.md` mentioned physics nowhere, so the absence read as an omission. It now
  states the decision instead.
