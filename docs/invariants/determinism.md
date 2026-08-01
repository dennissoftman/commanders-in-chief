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
- **A subsystem reads its peers and mutates only itself**, and registration order decides what a read
  sees: a peer registered earlier has already advanced this tick, one registered later has not. Both
  halves matter. Cross-subsystem writes would make "who changed this" depend on execution order in a
  way no per-subsystem hash could attribute, which is the guarantee above. And a read whose answer
  depended on registration order *without that order being part of the contract* is a simulation
  whose result depends on how a host happened to assemble it. The kernel enforces the first half
  structurally — it splits its own list around the subsystem it is running, so that one holds `&mut`
  to itself and `&` to everything else, and the forbidden mutation cannot be written.
- Floating-point behaviour is pinned where it reaches simulation state. Anything that cannot be
  pinned across platforms stays in presentation. **How** it is pinned is
  [ADR 0007](../adr/0007-simulation-arithmetic.md): simulation state is `f64`, only correctly-rounded
  operations may touch it, and the transcendentals are the project's own rather than the platform's —
  because what differs between platforms is the library, not the arithmetic.

## What is deliberately outside this

Presentation. A frame may compute anything it likes, in any precision, with any library, because nothing it
produces reaches simulation state — which is what the rule above about randomness is protecting. A physics
engine is the clearest case and the most tempting one to get wrong:
[ADR 0008](../adr/0008-physics-engine.md) keeps it there.

## Testing

A determinism claim is only as good as its test. The standard here is that a recorded command stream
replayed against the same initial state reproduces the same per-tick state hashes — checked in CI,
not by hand.
