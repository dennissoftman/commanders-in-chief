# M0: Foundation

Establish the workspace, the invariants every later milestone is held to, and the gate that enforces
them.

**Status:** Complete.

## Charter

A foundation milestone delivers no gameplay. What it delivers is the ability to tell, mechanically,
whether later work is correct — which is why it comes first rather than being retrofitted.

- A Rust workspace on a pinned toolchain, so a build is reproducible.
- `unsafe_code = "forbid"` at workspace scope.
- Strict lints (`clippy::all` and `clippy::pedantic`) treated as errors in CI, so a warning cannot
  accumulate into background noise.
- Bounded-parsing primitives: a reader that cannot escape its region, with structured errors carrying
  the offset, the requested length, and what remained.
- The two invariant documents that later milestones are measured against:
  [binary parsing](../invariants/binary-parsing.md) and [determinism](../invariants/determinism.md).
- A licence posture recorded before the first public push.

## Exit condition

Met. The gate runs formatting, strict lints, and the full test suite; the bounded reader is covered by
negative tests for truncation, out-of-range seeks, and sub-reader escape.

## Explicitly not done

- No profiling or benchmark harness. Performance work needs something to measure, and M0 has nothing
  that runs per frame.
- No fuzz targets yet. The bounded reader is small enough that its negative tests cover it; fuzzing
  becomes worthwhile once the container decoders in M1 and M2 exist.
