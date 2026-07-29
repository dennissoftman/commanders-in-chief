# M5: Simulation

A deterministic fixed-tick simulation kernel: the thing everything about gameplay and multiplayer
rests on.

**Status:** Planned. The one decision that had to be made before any of it — how floating point is pinned
where it reaches simulation state — is settled in [ADR 0007](../adr/0007-simulation-arithmetic.md).

## Charter

- Fixed ticks at a defined rate, independent of frame rate. Presentation may interpolate between ticks;
  it may never advance them.
- Stable object identifiers from a deterministic counter — never an allocation address, never insertion
  order in a hashed container.
- Named, versioned, seeded random streams. Drawing from a stream is part of the state transition, so no
  stream may be consumed by presentation, logging, or diagnostics.
- Ordered scheduling: for one tick, every subsystem runs in a defined order, and that order is part of
  the format's contract.
- Command recording: the tick-stamped input stream that produced a run.
- Replay: the same commands against the same initial state reproduce the run exactly.
- Per-tick, per-subsystem state hashes, so a divergence reports *which* subsystem drifted and *when*.
- Snapshots the interface can read without being able to mutate.

## Exit condition

A recorded command stream replayed against the same initial state reproduces identical per-tick state
hashes, verified in CI rather than by hand. Player and team activation, spawn assignment, and object
construction from a scenario all work, with a scenario's declared starts producing the units it claims.

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
trigonometry to be written. `f64` over `f32` is for accumulation headroom and **not** for determinism —
the two widths are equally deterministic, and reaching for a wider type to fix a divergence treats a
reproducibility problem as an accuracy problem.

## Explicitly not done

- No gameplay. This milestone delivers the kernel that gameplay runs inside; units, orders, and combat
  are M6. The separation is deliberate: a kernel proven deterministic against a trivial subsystem is a
  much better foundation than one debugged alongside the gameplay it runs.
- No networking. Lockstep is M7 and needs this milestone's hashes to exist first.
- No save and load of a running match. It needs a stable state layout, which follows M6.
