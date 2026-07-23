# R0: Repository and resource-probe foundation

**Status:** Complete. GitHub CI run `29840005186` passed the completion suite.

**Scope:** GPL/provenance policy, Rust workspace, bounded reader, normalized VFS paths,
loose-directory mounts, deterministic manifest CLI, tests, and CI.

**Exclusions:** Archive formats, retail assets, rendering, simulation, AI, networking.

**Inputs:** Synthetic byte arrays and temporary directory trees.

**Outputs:** Structured parse errors and stable tab-separated resource manifests.

**Owner:** `cic-core`, `cic-vfs`, and `cic-tools`.

**Acceptance tests:** Truncation and invalid-seek tests; path normalization and traversal
rejection; overlay precedence and history; identical manifests for identical inputs.

**Determinism:** Sorted virtual paths, explicit mount order, no host enumeration order in
output.

**Documentation:** Repository spine, ADR 0001, binary parsing and determinism invariants.

**Completion artifact:** Passing CLI integration test with two synthetic overlay trees.

### Resource provider and mod-profile refinement (complete)

**Scope:** Replace eager disk payload retention with lazy bounded resource reads and add ordered,
declarative custom-base/mod mount plans without making built-in retail filenames engine
requirements.

**Exclusions:** Package/dependency management, Workshop integration, hot reload, mod authoring,
signing, scripting, and automatic interpretation of third-party mod conventions.

**Inputs:** Synthetic loose trees, arbitrarily named synthetic BIG files, bounded mount-profile
text, explicit built-in profiles, and repeated mod paths.

**Outputs:** Stable indexed manifests, caller-bounded owned resource reads, custom total-conversion
plans, and deterministic base-then-mod provider provenance.

**Owner:** `cic-vfs` for lazy providers and `cic-tools` for profile parsing and CLI composition.

**Acceptance tests:** Disk providers remain indexable after payload deletion and fail only on lazy
read; payload and directory-index limits reject before excess allocation; malformed/oversized
profiles fail structurally; arbitrary archive names and a loose mod produce the expected winning
manifest.

**Determinism:** Mount order is explicit, optional providers retain declaration order, built-in
host filenames resolve by ASCII case with ambiguity rejection, and no filesystem enumeration order
selects a winner.

**Documentation:** ADR 0008, architecture boundaries, README profile syntax, compatibility matrix,
and changelog.

**Completion artifact:** Synthetic custom profile plus repeatable mod CLI integration test and lazy
directory/BIG provider unit tests.

## Completion evidence

- R0 completion suite passed in GitHub CI run `29840005186`.
- Rust workspace and CI policy.
- Bounded, cursor-based binary reads with structured errors.
- Normalized, ASCII case-insensitive virtual paths.
- Deterministic last-mounted-wins overlays with full provider history.
- Disk-backed directory mounts retain file metadata and BIG mounts retain only bounded directory
  indices; winning payloads are read lazily under parser-selected allocation limits.
- Bounded declarative mount profiles support arbitrarily named custom bases, optional providers,
  total conversions, and repeatable ordered mod layers without retail archive sentinels.
- Loose-directory manifest CLI and synthetic tests.
