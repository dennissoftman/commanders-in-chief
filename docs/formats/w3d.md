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
- No geometry, material, hierarchy, or animation semantics are decoded in this gate.
- Exact boundary closure is required because the format has no independent file magic.

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
