# Roadmap

Progress is measured by compatibility gates, not elapsed time.

## R0: Repository and resource-probe foundation

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

## R1: BIG and CSF resource probe

**Scope:** Evidence-backed BIG archive mounting and complete CSF decoding with resource
provenance.

**Exclusions:** Compression not present in verified variants, localization UI, retail
fixture distribution, W3D/MAP parsing.

**Inputs:** Synthetic BIG and CSF files plus user-owned archives for local verification.

**Outputs:** Resolved VFS manifests and deterministic localization reports.

**Owner:** `cic-vfs` for BIG and new `cic-formats` for CSF.

**Acceptance tests:** Valid variants, truncation at every field, invalid counts/offsets,
duplicates, overlay conflicts, string bounds, and fuzz targets.

**Determinism:** Stable archive entry ordering, last-mounted-wins policy, stable label
ordering and diagnostics.

**Documentation:** `docs/formats/big.md`, `docs/formats/csf.md`, compatibility matrix.

**Completion artifact:** Synthetic archive containing a CSF file and a checked-in stable
manifest snapshot.

## R2: W3D inspection and viewer

Gates are separately completed for chunk inventory, static geometry, materials,
hierarchies, and animation. Rendering begins only after unknown chunks can be retained
and reported without data loss.

## R3: MAP terrain inspection and viewer

Implement versioned chunk inventory, terrain, objects, lighting, and diagnostics. Preserve
unknown chunks and keep semantic decoders independently versioned.

## R4: Deterministic simulation kernel

Introduce fixed 30 Hz ticks, stable IDs, versioned seeded RNG streams, ordered scheduling,
command recording, replay, and subsystem state hashes before gameplay modules.

## R5: Navigation analysis and gameplay slice

Derive terrain and locomotor-aware regions, portals, choke points, and dynamic obstacles,
then complete one build-harvest-combat loop using normal player commands.

