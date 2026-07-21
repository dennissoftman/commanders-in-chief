# Current Objective

## Objective

Begin evidence-backed W3D static-mesh decoding on top of the verified chunk inventory.

## Implemented foundation

- R0 completion suite passed in GitHub CI run `29840005186`.
- Rust workspace and CI policy.
- Bounded, cursor-based binary reads with structured errors.
- Normalized, ASCII case-insensitive virtual paths.
- Deterministic last-mounted-wins overlays with full provider history.
- Loose-directory manifest CLI and synthetic tests.
- Evidence-backed `BIGF`/`BIG4` indexing with explicit limits and synthetic fixture.
- BIG duplicate-name history with deterministic last-entry-wins resolution.
- Mixed directory/BIG manifests through `cic-inspect`.
- Evidence-backed CSF version 3 decoding with raw names, complemented UTF-16, optional
  wave names, zero-string labels, and all variants preserved.
- Deterministic `cic-inspect csf` reports through loose-directory or BIG mounts.
- Original CSF fixture and synthetic BIG-to-CSF CLI completion artifact.
- Bounded W3D chunk-tree inventory with opaque unknown payload preservation, stable
  slash-separated tree paths, and 73 known chunk names.
- Deterministic `cic-inspect w3d` reports through loose-directory or BIG mounts.
- Original nested W3D fixture and synthetic BIG-to-W3D CLI completion artifact.
- Local formatting, strict Clippy, and all 31 runtime tests pass.
- All 18 installed Steam Generals BIG archives have matching declared sizes and bounded
  verified directory trailers; `INI.big` resolves 92 deterministic manifest entries.
- The installed Steam Generals CSF parses exactly to its 282,246-byte member boundary and
  reports version 3, 2,806 labels, and 2,805 strings.
- A 113,980-byte installed Steam Generals W3D parses exactly into 525 stable inventory
  records; 12 sampled W3Ds use the documented container flag.

## Known blockers

- `BIG4` remains implemented from corroborating source but unverified against retail data.
- The checked-in CSF fuzz target compile-checks with `libfuzzer-sys`; a bounded runtime
  smoke test remains pending because the `cargo-fuzz` runner is not installed.
- Mesh header versions, fixed structure sizes, vertex/triangle validation, and material
  references require an evidence-backed semantic W3D specification.

## Next verified step

Specify `W3D_CHUNK_MESH_HEADER3`, vertices, normals, triangles, and index/count invariants;
then decode one immutable static mesh without introducing rendering dependencies.
