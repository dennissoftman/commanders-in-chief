# Determinism Invariants

- Virtual paths use `/`, collapse empty and `.` components, reject `..`, and fold ASCII
  letters to lowercase.
- Backslashes from retail archives or Windows inputs are normalized at the VFS boundary;
  public paths, manifests, hashes, and cache keys always contain `/` separators.
- Mount order is explicit and monotonic; later providers override earlier providers.
- Manifests and diagnostic collections are sorted by normalized virtual path.
- Physical directory enumeration order never affects output.
- Stable output contains no wall-clock timestamps or machine-specific absolute paths.
- Future simulation state uses fixed ticks, stable IDs, explicit RNG streams, and
  versioned hashes.
