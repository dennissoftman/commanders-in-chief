# M5: Simulation

A deterministic fixed-tick simulation kernel: the thing everything about gameplay and multiplayer
rests on.

**Status:** Planned.

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
why it is settled here rather than discovered in M7 when a desync appears.

## Explicitly not done

- No gameplay. This milestone delivers the kernel that gameplay runs inside; units, orders, and combat
  are M6. The separation is deliberate: a kernel proven deterministic against a trivial subsystem is a
  much better foundation than one debugged alongside the gameplay it runs.
- No networking. Lockstep is M7 and needs this milestone's hashes to exist first.
- No save and load of a running match. It needs a stable state layout, which follows M6.
