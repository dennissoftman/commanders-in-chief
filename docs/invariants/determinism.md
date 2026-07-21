# Determinism Invariants

- Virtual paths use `/`, collapse empty and `.` components, reject `..`, and fold ASCII
  letters to lowercase.
- Mount order is explicit and monotonic; later providers override earlier providers.
- Manifests and diagnostic collections are sorted by normalized virtual path.
- Physical directory enumeration order never affects output.
- Stable output contains no wall-clock timestamps or machine-specific absolute paths.
- Future simulation state uses fixed ticks, stable IDs, explicit RNG streams, and
  versioned hashes.

