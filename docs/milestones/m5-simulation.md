# M5: Simulation

A deterministic fixed-tick simulation kernel: the thing everything about gameplay and multiplayer
rests on.

**Status:** Exit condition met. The kernel mechanics landed first, every charter line below exercised
against a deliberately trivial subsystem; scenario activation followed as `cic_sim::activation`, so a
map's declared players and placements construct into hashed kernel state. The prerequisite decision —
how floating point is pinned where it reaches simulation state — was settled in [ADR
0007](../adr/0007-simulation-arithmetic.md) before any of this was written, and the kernel was written
inside it.

## Charter

- Fixed ticks at a defined rate, independent of frame rate. Presentation may interpolate between ticks;
  it may never advance them. **Done** — `Kernel::advance` is the only way state moves, one tick at a
  time; advancing needs `&mut Kernel`, which a render loop does not hold, and the `TickAccumulator`
  is the presentation-side loop every host shares rather than rewrites.
- Stable object identifiers from a deterministic counter — never an allocation address, never insertion
  order in a hashed container. **Done** — `ObjectId` from `IdAllocator`, counting from one, never
  reused within a run, with zero reserved as "no object". The counter is itself hashed state.
- Named, versioned, seeded random streams. Drawing from a stream is part of the state transition, so no
  stream may be consumed by presentation, logging, or diagnostics. **Done** — SplitMix64 streams keyed
  by name in a `BTreeMap`, seeded from the session seed, the name, and a version, so a subsystem that
  changes how it consumes its stream bumps the version and every stale replay fails its hash comparison
  immediately. Stream states and draw counts are hashed with everything else, which is what caught the
  planted extra-draw bug in the replay test on the exact tick it happened.
- Ordered scheduling: for one tick, every subsystem runs in a defined order, and that order is part of
  the format's contract. **Done** — registration order is execution order, a duplicate name is refused
  at registration, and the hash record preserves the order so a reordering is visible.
- Command recording: the tick-stamped input stream that produced a run. **Done** — `CommandLog`,
  append-only and tick-ordered, refusing an out-of-order record rather than sorting it, because a log
  that reorders its input cannot testify about what arrived. Payloads are opaque bytes: what a command
  *means* is M6's to define.
- Replay: the same commands against the same initial state reproduce the run exactly. **Done** — the
  exit-condition test in `tests/replay.rs` runs 120 ticks with commands landing mid-run, twice, and
  requires every per-tick hash to match.
- Per-tick, per-subsystem state hashes, so a divergence reports *which* subsystem drifted and *when*.
  **Done** — FNV-1a over explicitly written state (floats by bit pattern, so two machines holding
  different zeros are diverged and say so), one entry per subsystem plus the kernel's own ids and
  streams, and `first_divergence` names the entry and the tick. Commands are folded into each tick's
  combined hash, so different *inputs* are caught on the tick they differ rather than surfacing later
  as mysterious state drift.
- Snapshots the interface can read without being able to mutate. **Done** — `Kernel::subsystem` hands
  out `&dyn Subsystem`, downcast for reading; mutation needs the `&mut` only the tick path holds.

## Exit condition

A recorded command stream replayed against the same initial state reproduces identical per-tick state
hashes, verified in CI rather than by hand. Player and team activation, spawn assignment, and object
construction from a scenario all work, with a scenario's declared starts producing the units it claims.

**Met.** The replay half runs in CI against a deliberately trivial subsystem — wandering points
spawned through the id counter, drifting by stream draws, culled by command — which exercises
everything the kernel owns while proving nothing about gameplay, exactly as the design notes below
prescribe. The activation half is `cic_sim::activation`: players take seats in authored order, every
placement becomes an object with an authored-order identifier, owners resolve to seats, and the pose
crosses into simulation units exactly once — `f32` widening exactly to `f64`, degrees becoming an
integer binary fraction of a revolution per ADR 0007's "angles are integers in simulation state".
Activation is inside the determinism claim, not beside it: two kernels activated from one scenario
hash identically on every following tick, and one moved placement diverges on tick zero, attributed
to `forces`.

One reading is pinned down rather than left to drift: "the units it claims" means the scenario's
declared placements, because that is all a scenario today declares. A faction's *starting roster* —
what a player gets at their start position beyond what the map places — needs the template set, and
that is [M6](m6-gameplay.md)'s content, deliberately not reached for early.

## Design notes

This milestone is where the [determinism invariants](../invariants/determinism.md) stop being
aspirational. Two are worth restating because they are the ones most easily broken by accident:

Presentation must not consume simulation randomness. A muzzle-flash variant drawn from a simulation
stream desyncs any client that renders a different number of frames — which is every client.

Floating-point behaviour is pinned wherever it reaches simulation state. Anything that cannot be
pinned across platforms stays in presentation. This constrains how gameplay maths is written, which is
why it is settled here rather than discovered in M7 when a desync appears — and it now is, in
[ADR 0007](../adr/0007-simulation-arithmetic.md).

The short version of that ADR, because it is the constraint M6 is written under: simulation state is
`f64`; only correctly-rounded operations may touch it, which is `+ - * /`, `sqrt`, comparison and rounding;
no platform transcendental appears in simulation code, because `sin` and its family come from the operating
system's C library and differ in the last bits between them; the project supplies its own instead, pinned
by exact-value tests, with angles stored as integer turns so range reduction is exact. Fixed-point was
rejected because it charges for determinism that IEEE-754 already provides and still leaves the
trigonometry to be written — though `cordic` is where to start if that is ever revisited. The `libm` crate
was evaluated as the implementation and is used as the *oracle* instead: it is already vendored here and
its code contains no architecture-gated path, but it promises nothing about reproducibility and has been
archived, and this needs a guarantee rather than a likelihood. `f64` over `f32` is for accumulation headroom and **not** for determinism —
the two widths are equally deterministic, and reaching for a wider type to fix a divergence treats a
reproducibility problem as an accuracy problem.

## Explicitly not done

- No gameplay. This milestone delivers the kernel that gameplay runs inside; units, orders, and combat
  are M6. The separation is deliberate: a kernel proven deterministic against a trivial subsystem is a
  much better foundation than one debugged alongside the gameplay it runs.
- No networking. Lockstep is M7 and needs this milestone's hashes to exist first.
- No save and load of a running match. It needs a stable state layout, which follows M6.
- No cross-platform determinism run. The hashes are exact-bit and the arithmetic is argued from the
  standard, but the settling evidence — a recorded run replayed on another platform reproducing the
  same per-tick hashes — needs CI runners on more than one platform, and none exists yet.
  [M10](m10-scripting.md) carries the same item for scripts, and [M7](m7-network.md)'s exit condition
  is what will force it.
