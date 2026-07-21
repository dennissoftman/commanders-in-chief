# W3D Chunk Container Format

- Status: verified against user-owned Steam Generals W3D assets
- Owning crate: `cic-formats`
- Last updated: 2026-07-21

## Evidence

The Westwood chunk reader in TheSuperHackers/GeneralsGameCode revision
`9f7abb866f5afd446db14149979e744c7216baaf` establishes the eight-byte chunk header,
payload-length meaning, and high-bit child-container flag. The W3D header at the same
revision establishes asset chunk identifiers. Exact source and licensing details are in
`docs/provenance/w3d.md`.

Runtime sampling of 12 user-owned Steam Generals W3D members found hierarchy (`0x100`),
animation (`0x200`), and mesh (`0x000`) first chunks. Every sampled first chunk used the
container bit. Several files contain multiple top-level chunks. One 113,980-byte member
was fully inventoried into 525 chunks with exact recursive and file boundary closure. No
retail bytes or asset names are stored in repository fixtures.

The pinned `w3d_file.h` defines the version-3 mesh header, vector, and triangle records.
The matching `meshmdlio.cpp` and `meshgeometry.cpp` readers establish that the header must
be the first mesh child and that its declared counts drive vertex, normal, and triangle
reads. Local verification decoded one static version 4.2 mesh from the same user-owned
113,980-byte member: 24 vertices, 24 normals, and 12 triangles closed their chunks exactly,
and every triangle index was in range.

## Stream layout

A W3D file is a sequence of chunks and has no separate whole-file magic or header. All
fields are unsigned 32-bit little-endian values.

| Size | Field |
|---:|---|
| 4 | Numeric chunk identifier |
| 4 | Payload length and flags |
| variable | Payload bytes |

The low 31 bits of the second word are payload length, excluding the eight-byte header.
Bit 31 means that the payload is itself a sequence of chunks. If bit 31 is clear, payload
bytes are opaque data. Container payloads must close exactly at their declared boundary.

Representative top-level W3D identifiers are:

| Identifier | Meaning |
|---:|---|
| `0x00000000` | Mesh |
| `0x00000100` | Hierarchy |
| `0x00000200` | Animation |
| `0x00000280` | Compressed animation |
| `0x00000700` | Hierarchical LOD object |
| `0x00000740` | Collision box render object |

Identifiers do not determine whether a chunk is nested; the size word's high bit is the
authoritative container flag. The inspector currently labels 73 mesh, material, hierarchy,
animation, tree, and top-level identifiers from the pinned GPL header.

## Inventory policy

- Top-level and child order are preserved exactly.
- Every chunk records its numeric ID, absolute header offset, and payload length.
- Unknown data chunks preserve all raw payload bytes.
- Unknown container chunks preserve their complete child trees.
- The inventory remains lossless even when a separate semantic decoder recognizes geometry.
- Material, hierarchy, and animation semantics remain opaque in this gate.
- Exact boundary closure is required because the format has no independent file magic.

## Static mesh geometry

`W3D_CHUNK_MESH` is a child container. The static decoder requires exactly one data leaf
for each of the header, vertices, vertex normals, and triangles; unknown siblings remain
preserved by the inventory. `W3D_CHUNK_MESH_HEADER3` must be the first child.

The 116-byte Header3 layout is:

| Size | Field |
|---:|---|
| 4 | packed major/minor version |
| 4 | mesh attributes |
| 16 | raw fixed-width mesh name |
| 16 | raw fixed-width container name |
| 4 each | triangle, vertex, material, and damage-stage counts |
| 4 | signed sort level |
| 4 each | prelit version and one future count |
| 4 each | vertex-channel and face-channel bits |
| 12 each | bounding-box minimum, maximum, and sphere center |
| 4 | sphere radius |

Each vertex or normal is three little-endian IEEE-754 32-bit components (12 bytes). Each
32-byte triangle contains three 32-bit vertex indices, 32-bit attributes, a 12-byte plane
normal, and a 32-bit plane distance.

The implemented static subset accepts Header3 versions 3.0 through the pinned current
version 4.2. It requires the location and basic-face channel bits, rejects nonzero geometry
type bits and bone-ID channels, and enforces these invariants before returning immutable
values:

- declared vertex and triangle counts are limited before allocation;
- vertex and normal payloads are exactly `NumVertices * 12` bytes;
- the triangle payload is exactly `NumTris * 32` bytes;
- each required semantic chunk occurs exactly once as a data leaf;
- every triangle vertex index is less than `NumVertices`.

The default semantic limits are 4,000,000 vertices and 4,000,000 triangles per mesh.
Decoded values have no rendering, filesystem, or simulation dependencies.

`cic-inspect w3d-mesh <virtual-path> <top-level-index> <mount>...` produces a stable
geometry report. Floating-point values are rendered as exact hexadecimal bit patterns so
host locale and formatting do not affect output.

`cic-inspect w3d-obj <virtual-path> <top-level-index> <output.obj> <mount>...` writes a
deterministic Wavefront OBJ sanity-check export. It preserves object-space coordinates,
per-vertex normals, triangle order, and winding while deliberately omitting UVs and
materials from the not-yet-decoded material gate.

## Current safety limits

- File: 256 MiB.
- Total chunks across the tree: 1,000,000.
- Zero-based nesting depth: 64.
- Payload lengths are limited to the bounded file region.
- All offset additions and count increments are checked.

## Synthetic fixture

`crates/cic-formats/tests/fixtures/minimal.w3d.hex` is an original 49-byte stream with a
mesh container, nested unknown chunks, an unknown top-level leaf, and opaque test bytes.
It contains no retail art or derived asset data.

`crates/cic-formats/tests/fixtures/static-mesh.w3d.hex` is an original 260-byte Header3
version 4.2 mesh containing one triangle, three vertices, and three normals. Unit and
BIG-backed CLI tests cover exact decoding plus count, size, channel, type, duplication,
allocation-limit, and index failures.
