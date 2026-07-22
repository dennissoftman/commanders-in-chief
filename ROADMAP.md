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

**Status:** Complete.

**Scope:** Bounded recursive chunk inventory followed by separately gated static geometry,
materials, hierarchies, animation, and an animated viewer.

**Exclusions:** MAP terrain, gameplay simulation, general asset editing, and retail asset
distribution.

**Inputs:** Original synthetic W3D streams and user-owned W3D resources through the VFS.

**Outputs:** Stable unknown-preserving chunk reports, immutable decoded asset values, portable glTF
sanity-check artifacts, deterministic renderer captures, and an interactive animated viewer.

**Owner:** `cic-formats` for decoding, `cic-render` for staging/presentation/resources, and
`cic-tools` for VFS-backed inspection and launch commands.

**Acceptance tests:** Exact nested boundary closure, truncation and depth/count/size limits,
unknown payload preservation, semantic count/index checks, split-resource BIG-backed CLI
integration, retail smoke verification, and external importer validation.

**Determinism:** File-order chunk trees, slash-separated numeric paths, stable identifier
names, and no renderer or host-order dependency in reports.

**Documentation:** `docs/formats/w3d.md`, provenance, compatibility matrix, and renderer-boundary
ADR 0004.

**Completion artifact:** Original nested and composed textured/animated fixtures, stable chunk and
exact-bit geometry reports, a Blender-importable synthetic GLB, headless renderer hashes/captures,
and installed-resource window smokes that retain no retail content.

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

### R2 renderer ingestion and animated viewer gate (complete)

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
smokes passed, including bounded legacy helper-bone hiding. Clip framing is now fixed at selection
time rather than recomputed per animation tick. Pass-zero/stage-zero materials resolve textures
through the VFS, expand per-face UV seams, preserve source alpha, select opaque/alpha/additive GPU
pipelines, and reuse content-deduplicated texture images and material bind groups. An installed
airstrip initially used 15 effective materials and 13 unique textures without black sprite
backgrounds; the 39-clip infantry used four materials and four textures. The completed renderer
expands all passes/stages in stable order, uses a documented multiply policy for later stages,
samples temporal mapper arguments from explicit seconds, and exposes the same path to headless
capture. A synthetic two-pass/two-stage capture at animation frame 1 and mapper time 0.5 seconds
matches checked RGBA SHA-256
`b1f43b981348e99b89c5dcd15b64279cb1b9990df3996ae4b35e4939d8301672`. Final installed captures
rendered the airstrip as 27 draws/17 materials/14 textures and infantry frame 1 as four
draws/materials/textures without retaining retail content. Exact legacy fixed-function equivalence
remains explicitly excluded until broader image comparisons exist.

## R3: MAP terrain inspection and viewer

**Status:** In progress.

**Scope:** Add a bounded, unknown-preserving MAP chunk inventory, then separately gate versioned
terrain heights, blend/shore data, object placement, lighting, and a terrain viewer.

**Exclusions:** Gameplay simulation, pathfinding, scripts, asset editing, retail fixtures, and
claims that unobserved MAP versions share layouts.

**Inputs:** Original synthetic MAP byte streams plus user-owned installed and custom maps through
the existing VFS.

**Outputs:** Stable chunk/semantic reports, immutable renderer-neutral terrain values, deterministic
synthetic terrain captures, and an opt-in interactive viewer.

**Owner:** `cic-formats` for bounded decoding, `cic-render` for terrain staging/presentation, and
`cic-tools` for VFS-backed reports and viewer commands.

**Acceptance tests:** Exact chunk closure, version dispatch, truncation/count/offset/allocation
limits, unknown preservation, deterministic report ordering, original negative fixtures, headless
capture hashes, and local installed/custom-map smokes that retain no retail data.

**Determinism:** File/chunk/object order is explicit and stable; terrain diagnostics use explicit
camera/time inputs and never depend on filesystem order, locale, randomized maps, or wall clock.

**Documentation:** `docs/formats/map.md`, provenance records, compatibility matrix, renderer ADR
updates where the existing boundary changes, and user-owned capture instructions.

**Completion artifact:** Original versioned terrain fixture(s), checked stable reports and capture
hashes, plus a local user-owned MAP verification record without copied game content.

**Progress:** The initial source-backed gate inventories the `CkMp` symbol table and top-level
chunks with exact closure and opaque payload preservation. A separate semantic decoder accepts
`HeightMapData` versions 1 through 4, validates dimensions, border, boundaries, and exact row-major
sample cardinality, and retains the stored version-1 grid pending an explicit compatibility policy.
`cic-inspect map` and `map-height --report` produce stable VFS-backed reports from an original
synthetic MAP inside a synthetic BIG, while `map-height` writes exact samples as a basename-derived
deterministic grayscale PNG by default.
`BlendTileData` versions 6 and 7 now decode bounded tile-index planes, height-derived or stored
legacy cliff flags, terrain and edge texture classes, blend selectors, and cliff records; an
original negative fixture and stable
`map-blend` report cover the gate. Source-backed bounded `EAR\0` RefPack decompression closed one
user-owned installed MAP, whose height and blend cell counts both validated at 152,000. A bounded
Terrain INI gate now maps semantic classes to VFS terrain sheets with explicit default inheritance.
`cic-render` stages source-scaled geometry, packed tile quadrants, base/primary/extra procedural
layers, mip rounding, stored and legacy-adjusted cliff UVs, height-selected cliff diagonals, and a
separately indexed custom-edge atlas pass. `map-render` exposes explicit legacy/modern policies and
emits an sRGB headless PNG with stable diagnostics. `map-view` shares that resource/staging path and
adds perspective free-flight camera controls. Original synthetic tests cover cliff adjustment and
custom-edge geometry/texturing; an installed 151,221-cell Generals visual and viewer smoke resolved
all 14 classes and retained no retail capture. The viewer now streams independently cancellable,
depth-capped 16/32-pixel screen-space tiers over the stable 8-pixel background, retains old GPU
patches during replacement, and applies an explicit viewer-only directional slope light; synthetic
region output matches a full-resolution bake byte-for-byte. Hybrid-deferred water and Modern
de-tiling are active. Blend version 8, object placement, source-authored lighting, and custom-map
verification remain open.

## R4: Deterministic simulation kernel

Introduce fixed 30 Hz ticks, stable IDs, versioned seeded RNG streams, ordered scheduling,
command recording, replay, and subsystem state hashes before gameplay modules.

## R5: Navigation analysis and gameplay slice

Derive terrain and locomotor-aware regions, portals, choke points, and dynamic obstacles,
then complete one build-harvest-combat loop using normal player commands.
