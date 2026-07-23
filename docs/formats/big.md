# BIG Archive Format

- Status: verified against a user-owned Steam Generals `INI.big`
- Owning crate: `cic-vfs`
- Last updated: 2026-07-21

## Evidence

The Generals loader at TheSuperHackers/GeneralsGameCode revision
`9f7abb866f5afd446db14149979e744c7216baaf` establishes the `BIGF` signature,
16-byte directory start, big-endian file count, and big-endian member offset and size.

OpenSAGE revision `588ac477367a0022adf29f20a084e8873014e6ce` corroborates the general layout,
assigns the fourth header word as the first-file offset, supports `BIG4`, and records that
duplicate member names occur in real archives. Its assumption that archive size is
big-endian does not match the verified Generals retail file described below.

EternalBig revision `cdcabab6ed2cbbcbcf453baf6c16f619736b540f` independently records the
mixed-endian header and names `0x4C323331` as an archive-end marker. That value encodes
ASCII `L231`; its adjacent `L321` source comments are transposition typos. The relevant
reader was first committed on 2022-09-15, before the 2025 EA source release.

Runtime verification against 18 Steam-distributed Generals BIG archives established the
mixed-endian header with no declared-size mismatches. For example, `INI.big` bytes
`B7 14 74 00` at offset 4 decode little-endian to `7,607,479`, exactly matching its
physical file length. Big-endian decoding produces `3,071,570,944`. No retail bytes or
member content were added to the repository.

Exact source links and licensing notes are in `docs/provenance/big.md`.

## Byte layout

Integers are unsigned 32-bit values. Archive size is little-endian; all remaining numeric
fields are big-endian.

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | ASCII signature: `BIGF` or `BIG4` |
| 4 | 4 | Complete archive size, little-endian |
| 8 | 4 | File-table entry count, big-endian |
| 12 | 4 | First payload byte / end of file table, big-endian |
| 16 | variable | File-table entries |
| first payload byte | variable | Member payloads |

Each file-table entry is:

| Size | Field |
|---:|---|
| 4 | Absolute member offset from archive start, big-endian |
| 4 | Member byte length, big-endian |
| variable | Zero-terminated path bytes |

The file table contains exactly the declared number of entries. An optional directory
trailer may occur between the final entry and the declared first payload byte.

Observed Generals trailer variants are:

| Length | Bytes | Observation count |
|---:|---|---:|
| 0 | absent | 3 of 18 archives |
| 8 | ASCII `L225`, then `00 00 00 00` | 10 of 18 archives |
| 8 | ASCII `L231`, then `00 00 00 00` | 5 of 18 archives |

The marker meanings are not yet established. The parser bounds and preserves trailer
bytes without rejecting unknown markers, so diagnostics can classify them later without
losing compatibility.

## Path and duplicate rules

- Entry names are decoded as UTF-8; verified retail names are expected to be ASCII.
- `/` and `\\` are equivalent separators.
- ASCII case is folded to lowercase.
- Empty paths and `..` components are rejected.
- File-table order is significant.
- Duplicate normalized paths are retained in provenance history; the last entry wins.
- Archive mount order remains global VFS order; a later mount overrides an earlier one.

## Current safety limits

- Archive: 2 GiB.
- Entries: 1,000,000.
- Entry name: 4,096 bytes before the zero terminator.
- Opaque directory trailer: 64 KiB.
- Declared archive size must equal the supplied byte-region length.
- The first payload offset must lie between byte 16 and the archive end.
- Every member range must lie entirely at or after the first payload byte and inside the
  archive.
- Offset-plus-size arithmetic is checked before member data is exposed.

## Explicit exclusions and open questions

- Two-byte `??` placeholder archives accepted by OpenSAGE are not accepted yet because
  the Generals loader does not establish that behavior.
- Non-UTF-8 entry names are rejected until an observed encoding policy is documented.
- Archive padding and mismatched declared sizes are rejected until retail evidence
  requires a compatibility policy.
- Directory trailer marker meanings remain unknown. Absence, `L225`, and `L231` are
  verified variants; other bounded values remain preserved as opaque data.
- BIG data remains memory-backed in this slice; streaming providers are deferred.

## Synthetic fixture

`crates/cic-vfs/tests/fixtures/minimal.big.hex` describes a 69-byte original fixture with
two backslash-separated member names and an `L231` plus zero-word trailer. Tests verify
that the public paths use `/` separators without committing copyrighted game content.

`crates/cic-vfs/tests/fixtures/minimal.big4.hex` is the byte-identical fixture with the
`BIG4` signature substituted for `BIGF`, verifying that both header variants dispatch
through the same mixed-endian parse and pass identical valid-parse and
truncated-every-prefix acceptance tests. It does not independently confirm BIG4's field
order or endianness against retail data; see the open item in
`docs/milestones/r1-big-csf.md`.
