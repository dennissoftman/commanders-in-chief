# Changelog

All notable user-visible changes are recorded here.

## Unreleased

### Added

- Initial GPL-3.0-only repository charter and provenance policy.
- Rust workspace with bounded binary input and deterministic virtual filesystem crates.
- `cic-inspect manifest` command for deterministic loose-directory inventories.
- Bounded `BIGF`/`BIG4` archive indexing and mounting with stable entry provenance.
- Directory and BIG overlays in `cic-inspect manifest`.
- Bounded CSF localization decoding with complemented UTF-16, optional wave names,
  zero-string labels, and lossless raw names.
- `cic-inspect csf` deterministic localization reports through mounted directories and
  BIG archives.
- Bounded, unknown-preserving W3D chunk inventories with stable nested paths and known
  identifier names.
- `cic-inspect w3d` reports W3D chunk trees through mounted directories and BIG archives.
- Immutable W3D Header3 static geometry decoding with bounded vertex/triangle counts,
  exact record-size checks, static-channel validation, and range-checked triangle indices.
- `cic-inspect w3d-mesh` exact-bit geometry reports through mounted directories and BIG
  archives.
- Bounded W3D material inventories, vertex-material colors, first-pass material IDs, and
  explicit per-vertex diffuse color arrays.
- Bounded W3D fixed-function shader records, texture names/info, per-triangle shader and
  texture assignments, and texture-coordinate arrays.
- Bounded W3D hierarchy, highest-detail HLOD, rigid/skinned mesh composition, and classic
  raw-animation channel decoding, including split skeleton/skin/animation resources.
- `cic-inspect w3d-gltf` glTF 2.0 export with hierarchy transforms, skins, animation clips,
  first-pass PBR preview materials, UV conversion, and TGA/DDS-to-PNG image conversion.
- Generals and Zero Hour resource profiles with `--zh`, one-off `--game-dir`, persisted
  installation roots, Steam library discovery, and deterministic base-then-expansion VFS
  layering.
- Missing referenced retail textures produce warned magenta placeholders so geometry and
  animation remain inspectable.
- Synthetic unit and integration tests plus CI quality gates.
