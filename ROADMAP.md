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

**Status:** In progress; format compatibility and external animated-model preview gates complete.

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
The format surface is complete for renderer ingestion: time-coded and adaptive-delta compressed
animation decode under bounded expansion and use the existing glTF clip path. All fixed-function
passes, stages, mapper data, animated-texture descriptors, and shader bytes are retained in
versioned GLB/glTF metadata; the visible core-glTF approximation remains explicitly pass
zero/stage zero. Installed compressed infantry and two-pass building exports passed local
verification without retaining retail data. Additive `ONE + ONE` light materials keep their
unchanged source RGBA images and use separate alpha-coverage preview images; installed airstrip
lights verified that black sprite backgrounds no longer become opaque rectangles.

### R2 next gate: renderer ingestion and animated viewer

**Scope:** Introduce a renderer crate that consumes immutable `cic-formats` model values, renders
the selected HLOD with hierarchy/skinning/animation, and begins fixed-function pass/stage
equivalence behind an explicit preview policy.

**Exclusions:** MAP terrain, gameplay/simulation ownership, asset editing, retail fixtures, and
claims of complete fixed-function equivalence before image comparisons exist.

**Inputs:** Existing original composed W3D fixtures and user-owned installed resources through the
VFS; no renderer-side parsing.

**Outputs:** An interactive animated viewer plus deterministic diagnostic captures from synthetic
fixtures.

**Owner:** A new renderer crate depending on `cic-formats`/`cic-vfs`; simulation and core remain
renderer-independent.

**Acceptance tests:** Headless synthetic-frame checks, hierarchy/skin pose comparisons, stable
material-pass command ordering, malformed-resource rejection before renderer ingestion, and a local
installed-resource smoke capture.

**Determinism:** Stable mesh/pass/stage submission order, explicit animation time input, no host
filesystem order, and no wall-clock values in diagnostic artifacts.

**Documentation:** A renderer-boundary ADR, compatibility updates, and capture instructions that do
not distribute retail content.

**Completion artifact:** Checked-in synthetic screenshot/capture hashes plus a locally verified
animated installed-model capture report.

**Progress:** ADR 0004 selects `wgpu` 30 with native Vulkan, Metal, and Direct3D 12. The new
`cic-render` crate stages validated W3D geometry in stable file order and completed a surface-free
64x64 RGBA8 triangle capture at an explicit pose. The checked-in SHA-256 matched a local RTX 4080
SUPER run. `cic-inspect w3d-render` now composes models from synthetic or installed BIG overlays,
applies rigid/one-bone bind transforms, and emits a depth-tested geometry capture. An installed
building smoke capture succeeded. `cic-inspect w3d-view` now presents a 960x720 auto-fitted,
45-degree elevated, Z-up rotating model, samples raw or compressed hierarchy animation at explicit
integer frames, and switches clips with Left/Right. Installed building and 39-clip infantry window
smokes passed, including bounded legacy helper-bone hiding. Stable material-pass/texture commands
and a deterministic explicit animated-pose capture remain open.

## R3: MAP terrain inspection and viewer

Implement versioned chunk inventory, terrain, objects, lighting, and diagnostics. Preserve
unknown chunks and keep semantic decoders independently versioned.

## R4: Deterministic simulation kernel

Introduce fixed 30 Hz ticks, stable IDs, versioned seeded RNG streams, ordered scheduling,
command recording, replay, and subsystem state hashes before gameplay modules.

## R5: Navigation analysis and gameplay slice

Derive terrain and locomotor-aware regions, portals, choke points, and dynamic obstacles,
then complete one build-harvest-combat loop using normal player commands.
