# Roadmap

Progress is measured by compatibility gates, not elapsed time.

## R0: Repository and resource-probe foundation

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

## R1: BIG and CSF resource probe

**Status:** In progress.

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

**Progress:** BIGF indexing and mounting pass the complete local suite and all 18
installed Steam Generals archives. Mixed-endian fields, slash-normalized paths, and
none/`L225`/`L231` directory trailers are verified. The bounded CSF decoder, lossless
record IR, original fixture, deterministic report, and synthetic BIG-to-CSF CLI artifact
are implemented and verified against the installed Generals CSF. A 30-second AddressSanitizer
libFuzzer smoke run completed 4,077,155 CSF inputs without a finding. BIG4 retail
verification remains open.

## R2: W3D inspection and viewer

**Status:** In progress; external animated-model preview gates complete.

**Scope:** Bounded recursive chunk inventory followed by separately gated static geometry,
materials, hierarchies, animation, and an animated viewer.

**Exclusions:** MAP terrain, gameplay simulation, general asset editing, and retail asset
distribution.

**Inputs:** Original synthetic W3D streams and user-owned W3D resources through the VFS.

**Outputs:** Stable unknown-preserving chunk reports, immutable decoded asset values, and
portable glTF sanity-check artifacts before renderer integration.

**Owner:** `cic-formats` for decoding and `cic-tools` for inspection; a renderer crate is
introduced only when the viewer gate begins.

**Acceptance tests:** Exact nested boundary closure, truncation and depth/count/size limits,
unknown payload preservation, semantic count/index checks, split-resource BIG-backed CLI
integration, retail smoke verification, and external importer validation.

**Determinism:** File-order chunk trees, slash-separated numeric paths, stable identifier
names, and no renderer or host-order dependency in reports.

**Documentation:** `docs/formats/w3d.md`, provenance, compatibility matrix, and later ADRs
for renderer boundaries.

**Completion artifact:** Original nested and composed textured/animated fixtures, stable
chunk and exact-bit geometry reports, and a Blender-importable synthetic GLB; later
renderer gates add screenshot and animation capture artifacts.

**Progress:** The recursive inventory, 73-name identifier table, original nested fixture,
and `cic-inspect w3d` report are complete. A 113,980-byte installed W3D closes exactly into
525 records. Header3 versions 3.0 through 4.2, vertices, normals, and triangles now decode
into immutable renderer-neutral values with exact count/size and vertex-index validation.
The BIG-backed `cic-inspect w3d-mesh` report is deterministic. Materials, shaders, textures,
UVs, hierarchy/HLOD composition, rigid and skinned models, and classic raw-animation clips
decode into immutable bounded values. `cic-inspect w3d-export` composes split retail W3Ds,
resolves Generals or layered Zero Hour resources, converts TGA/DDS images to sRGB PNG, and
emits a Blender-importable self-contained GLB by default or external glTF with `--gltf`.
Compressed animation and richer multi-pass material equivalence remain before the format
surface is complete.

## R3: MAP terrain inspection and viewer

Implement versioned chunk inventory, terrain, objects, lighting, and diagnostics. Preserve
unknown chunks and keep semantic decoders independently versioned.

## R4: Deterministic simulation kernel

Introduce fixed 30 Hz ticks, stable IDs, versioned seeded RNG streams, ordered scheduling,
command recording, replay, and subsystem state hashes before gameplay modules.

## R5: Navigation analysis and gameplay slice

Derive terrain and locomotor-aware regions, portals, choke points, and dynamic obstacles,
then complete one build-harvest-combat loop using normal player commands.
