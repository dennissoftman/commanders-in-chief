# Determinism invariants

Determinism is not a nicety here. Lockstep multiplayer, replays, and desync diagnosis all reduce to
the same requirement: the same inputs must produce the same state on every machine, every run.

## Resource resolution

- Virtual paths use `/`, collapse empty and `.` components, reject `..`, and fold ASCII letters to
  lowercase.
- Separators are normalized at the resource-layer boundary. Paths, manifests, hashes, and cache keys
  downstream of it always contain `/`.
- Mount order is explicit and monotonic; later providers override earlier ones. Nothing infers
  precedence from a filename, a timestamp, or a directory listing.
- Physical directory enumeration order never affects output.
- Manifests and diagnostic collections are sorted by normalized virtual path.

## Stable output

- Stable output contains no wall-clock timestamps, no machine-specific absolute paths, and no
  addresses.
- Iteration over associative collections uses ordered maps, never hash maps, wherever the order can
  reach output or affect control flow.

## Simulation

- The simulation advances in fixed ticks, independent of frame rate. Presentation may interpolate
  between ticks; it may never advance them.
- Every object carries a stable identifier assigned by a deterministic counter, not by allocation
  address or insertion into a hashed container.
- Randomness comes from explicit, named, seeded streams. Drawing from a stream is part of the
  simulation's state transition, so a stream must never be consumed by presentation, logging, or
  diagnostics.
- Subsystem state hashes are computed per tick and versioned, so a desync reports *which* subsystem
  diverged and on which tick rather than only that one did.
- Floating-point behaviour is pinned where it reaches simulation state. Anything that cannot be
  pinned across platforms stays in presentation. **How** it is pinned is
  [ADR 0007](../adr/0007-simulation-arithmetic.md): simulation state is `f64`, only correctly-rounded
  operations may touch it, and the transcendentals are the project's own rather than the platform's —
  because what differs between platforms is the library, not the arithmetic.

## Testing

A determinism claim is only as good as its test. The standard here is that a recorded command stream
replayed against the same initial state reproduces the same per-tick state hashes — checked in CI,
not by hand.
