# W3D Format Provenance

## GeneralsGameCode evidence

- Repository: <https://github.com/TheSuperHackers/GeneralsGameCode>
- Revision: `9f7abb866f5afd446db14149979e744c7216baaf`
- Container files:
  - `Core/Libraries/Source/WWVegas/WWLib/chunkio.h`
  - `Core/Libraries/Source/WWVegas/WWLib/chunkio.cpp`
- Identifier file:
  - `GeneralsMD/Code/Libraries/Source/WWVegas/WW3D2/w3d_file.h`
- Static mesh readers:
  - `GeneralsMD/Code/Libraries/Source/WWVegas/WW3D2/meshmdlio.cpp`
  - `GeneralsMD/Code/Libraries/Source/WWVegas/WW3D2/meshgeometry.cpp`
  - `GeneralsMD/Code/Libraries/Source/WWVegas/WW3D2/vertmaterial.cpp`
- Hierarchy, HLOD, and animation readers:
  - `GeneralsMD/Code/Libraries/Source/WWVegas/WW3D2/htree.cpp`
  - `GeneralsMD/Code/Libraries/Source/WWVegas/WW3D2/hlod.cpp`
  - `GeneralsMD/Code/Libraries/Source/WWVegas/WW3D2/hrawanim.cpp`
  - `GeneralsMD/Code/Libraries/Source/WWVegas/WW3D2/motchan.cpp`
  - `Core/Libraries/Source/WWVegas/WW3D2/hcanim.cpp`
- Mapper reference:
  - `Core/Libraries/Source/WWVegas/WW3D2/MAPPERS.TXT`
- Permanent links:
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/Libraries/Source/WWVegas/WWLib/chunkio.h>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/Libraries/Source/WWVegas/WWLib/chunkio.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/GeneralsMD/Code/Libraries/Source/WWVegas/WW3D2/w3d_file.h>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/GeneralsMD/Code/Libraries/Source/WWVegas/WW3D2/meshmdlio.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/GeneralsMD/Code/Libraries/Source/WWVegas/WW3D2/meshgeometry.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/GeneralsMD/Code/Libraries/Source/WWVegas/WW3D2/vertmaterial.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/GeneralsMD/Code/Libraries/Source/WWVegas/WW3D2/htree.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/GeneralsMD/Code/Libraries/Source/WWVegas/WW3D2/hlod.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/GeneralsMD/Code/Libraries/Source/WWVegas/WW3D2/hrawanim.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/GeneralsMD/Code/Libraries/Source/WWVegas/WW3D2/motchan.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/Libraries/Source/WWVegas/WW3D2/hcanim.cpp>
  - <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/Libraries/Source/WWVegas/WW3D2/MAPPERS.TXT>
- Upstream notice: Command & Conquer Generals Zero Hour; Copyright 2025 Electronic Arts
  Inc.; historical notices identify Westwood Studios.
- License: GNU GPL version 3 or later with the Electronic Arts Section 7 additional terms
  in the upstream repository's `LICENSE.md`.

The source establishes native 32-bit chunk type/size words, a payload-only 31-bit length,
the high-bit child-container flag, nested boundary accounting, W3D identifiers, the
116-byte Header3 layout, 12-byte vectors, 32-byte triangles, header-driven record counts,
the 16-byte material inventory, 32-byte vertex materials, material-pass ID cardinality,
and four-byte DCG colors.

The same revision establishes the hierarchy and pivot layouts, parent-before-child world
transform composition, last-array HLOD selection, one-bone vertex influences, classic raw
animation headers and channels, post-multiplied translation/quaternion application, and direct
bone-transform deformation of bone-local skin vertices.

The compressed-animation header and loaders establish time-coded and adaptive-delta flavors,
their fixed channel headers, binary movement flag, sparse interpolation, four-bit signed deltas,
nine-byte delta packets, and 256-entry filter construction. The vertex-material attributes and
mapper reference establish two mapper selectors, their argument-string chunks, and the named
fixed-function mapping modes. The material-pass declarations also establish the `DIG` and `SCG`
four-byte RGB arrays and texture animation type, count, and rate fields.

## Runtime verification

On 2026-07-21, first chunk headers from 12 W3D members in user-owned Steam Generals BIG
archives were inspected in place. Hierarchy, animation, and mesh identifiers were present;
all 12 first chunks set the child-container bit, and several files had additional
top-level chunks after the first declared payload. A complete 113,980-byte W3D was then
parsed to exact closure as 525 chunk records. No retail bytes were copied.

On the same date, the semantic decoder read one normal-geometry version 4.2 mesh from that
user-owned member. Its header declared 24 vertices and 12 triangles; the vertex, normal,
and triangle payload lengths matched exactly and all 36 triangle indices were below 24.
No retail bytes, names, or float values were retained in the repository.

The same installed mesh verified two material passes and two vertex materials, with all
first-pass assignments in range. A second installed static mesh also decoded its material
inventory and assignments through the BIG-backed VFS. Their texture-driven diffuse values
are white, which is preserved accurately; no retail names or material values are retained.

On 2026-07-21, an installed split infantry export verified that raw animation channels use
model-scale-outlier translations to hide carried attachments. Literal glTF mapping expanded
animated bounds by orders of magnitude. The tools-layer nonsingular near-zero-scale preview policy
was authored for this project and verified to retain all clips while removing those remote output
positions. No retail bytes, names, or numeric channel values were retained.

On 2026-07-21, a bounded top-level scan found compressed-animation chunks in 17 user-owned
Generals W3Ds and at least three Zero Hour W3Ds. An installed infantry export decoded its
compressed companions into a self-contained GLB with 20 actions, including the compressed idle
clip. A separate installed building export preserved two-pass material metadata and a non-UV
environment mapper on two meshes. No retail bytes, names, mapper strings, or numeric channel
values were retained in the repository.

On 2026-07-21, installed `abarfrccmd.w3d` airstrip-light materials were verified against their
user-owned resolved DDS images. The decoded images were fully opaque, while their retained W3D
shader selectors were source `ONE` and destination `ONE`. A project-authored core-glTF preview
policy generated separate alpha-coverage images; the source images remained unchanged. No retail
bytes or images were retained in the repository.

On 2026-07-21, the interactive renderer resolved the same user-owned airstrip through the VFS and
rendered its pass-zero/stage-zero materials with source-alpha or additive GPU blending. Its 15
effective materials reused 13 unique decoded textures, and the additive lights displayed without
opaque black rectangles. A second installed smoke rendered four materials and four textures while
switching among 39 infantry clips with fixed per-clip framing. No retail bytes, images, names, or
captures were retained in the repository.

## Implementation record

The Rust implementations in `crates/cic-formats/src/w3d.rs`, `w3d_mesh.rs`,
`w3d_material.rs`, and `w3d_scene.rs`, plus the preview mapping in
`crates/cic-tools/src/gltf.rs` and renderer staging in `crates/cic-render/src/model.rs`,
were authored for this project from the facts in `docs/formats/w3d.md` and the runtime
verification above.
No C++ source code was copied, translated line by line, or imported. The immutable tree,
mesh and material values, structured errors, limits, exact-size checks, index validation,
color resolution, absolute offsets, and unknown-payload preservation policy are native to
this repository.
