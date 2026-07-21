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

## Material colors

The first material gate decodes color-relevant values without interpreting shaders or
textures. `W3D_CHUNK_MATERIAL_INFO` is exactly four little-endian 32-bit counts: material
passes, vertex materials, shaders, and textures. Vertex materials are child containers;
their optional zero-terminated name and required 32-byte info record contain:

| Size | Field |
|---:|---|
| 4 | attributes |
| 4 each | ambient, diffuse, specular, and emissive RGB plus pad byte |
| 4 each | finite IEEE-754 shininess, opacity, and translucency |

Each `W3D_CHUNK_MATERIAL_PASS` may contain one vertex-material ID shared by the mesh or
one ID per vertex. IDs must be below the declared and decoded vertex-material count. An
optional DCG chunk contains exactly one four-byte RGBA value per vertex.

For preview output, the first pass resolves explicit DCG colors first; otherwise it maps
each vertex to its vertex material's diffuse RGB. The semantic defaults limit meshes to
64 passes, 65,536 vertex materials, 65,536 shaders, 65,536 textures, eight texture stages
per pass, 12,000,000 UVs per stage, and 255 name bytes.
Names, counts, payload sizes, and IDs are checked before allocation or lookup.

`cic-inspect w3d-mesh <virtual-path> <top-level-index> <mount>...` produces a stable
geometry report. Floating-point values are rendered as exact hexadecimal bit patterns so
host locale and formatting do not affect output.

## Shaders, textures, and preview materials

The shader table is a packed array of 16-byte fixed-function records. Texture entries
contain one required, bounded zero-terminated filename and an optional 12-byte record:
16-bit attributes, 16-bit animation type, 32-bit frame count, and a finite 32-bit frame
rate. Declared shader and texture counts must exactly match their decoded tables.

Material-pass shader IDs and texture-stage texture IDs are either one 32-bit value or one
value per triangle. Texture ID `0xFFFFFFFF` means no texture; every other shader or texture
ID must be in range. Stage UVs are finite `(U, V)` float pairs. Without a per-face index
array there must be one UV per vertex; an optional per-face array contains three checked
UV indices per triangle.

## Hierarchy, HLOD, skinning, and animation

A hierarchy header is 36 bytes: version, a fixed 16-byte name, pivot count, and center.
Each 60-byte pivot contains a fixed name, parent index, translation, Euler values, and an
`(x, y, z, w)` quaternion. Pivot zero has no parent and its serialized base transform is
ignored by the legacy runtime; every other parent must precede its child.

The HLOD header selects a named hierarchy and declares ordered detail arrays. Each array
declares its subobject count and screen size; each 36-byte subobject binds a named render
object to one hierarchy pivot. The final array is the highest-detail representation used
by preview export. Named box render objects are recognized and omitted from mesh output.

Skin geometry carries one 16-bit bone index per vertex. The decoder requires the skin
geometry flag, bone channel, and influence chunk to agree, and checks every influence
against the composed hierarchy before export. Rigid HLOD meshes are attached to their
selected pivot instead.

Classic raw-animation headers contain fixed animation and hierarchy names, a 32-bit frame
count, and frame rate. Translation and quaternion channels use checked 16-bit inclusive
frame ranges and pivot indices. Quaternion channels contain four floats per sample; other
supported channels contain one. Retail exporters can leave unused whole-float samples at
the end of a channel, matching the original loader's bounded close-chunk behavior; partial
float padding is rejected. Root-pivot animation is ignored as in the runtime.

`cic-inspect w3d-export <virtual-path> [<output.glb>] [<mount>...]` composes the final HLOD,
referenced hierarchy, skinning, and matching raw animation clips. If the retail layout
splits `_SKN`, `_SKL`, and animation W3Ds, sibling resources are discovered through the
same VFS. The default output is one self-contained GLB named after the resource basename;
an explicit output path overrides it. Passing `--gltf` before the virtual path instead
writes glTF 2.0 JSON, an external `.bin`, and PNG images, using `.gltf` for an inferred
name. A root quaternion converts W3D Z-up coordinates to glTF Y-up for Blender and standard
viewers.

All directory and BIG inputs share one VFS. Texture resolution tries the encoded name and
`art/textures/<name>`; a `.tga` reference may resolve to the retail `.dds` replacement.
Only referenced user-owned images are decoded and converted to PNG. Decoded straight-alpha
RGBA texels are preserved and tagged sRGB because they feed glTF base-color textures; no
extra gamma transform is applied. Later mounts retain the normal override policy. Missing
images produce a warned 1x1 magenta placeholder so a model remains inspectable. Additional
W3D passes/stages remain decoded but are not mapped into the first-pass glTF preview
material.

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

`crates/cic-formats/tests/fixtures/colored-mesh.w3d.hex` extends that original triangle
with one red vertex material and one material pass. Tests also synthesize an explicit
red/green/blue DCG array and cover precedence, count, ID, name, and allocation failures.

The composed completion fixture adds an original two-pivot hierarchy, highest-detail HLOD,
translation animation, fixed-function shader, texture, one stage, and three UV pairs. Its
skin, hierarchy, and animation are split into separate W3Ds in a synthetic `W3D.big`; an
independent synthetic `Textures.big` contains an original 2x2 TGA. The CLI integration
test proves cross-resource composition, a valid single-file GLB, optional external glTF,
and exact decoded RGBA preservation in sRGB PNG output.
