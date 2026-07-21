# CSF Localization Format

- Status: verified against a user-owned Steam Generals `generals.csf`
- Owning crate: `cic-formats`
- Last updated: 2026-07-21

## Evidence

The Generals loader at TheSuperHackers/GeneralsGameCode revision
`9f7abb866f5afd446db14149979e744c7216baaf` establishes the header fields, record tags,
little-endian integers, complemented UTF-16 code units, optional wave-name records, and
version-dependent language field. Exact source and license details are in
`docs/provenance/csf.md`.

Runtime inspection of the user-owned Steam Generals `English.big` established a complete
282,246-byte version 3 CSF containing 2,806 labels and 2,805 strings. It parses exactly to
the member boundary, contains one valid zero-string label, and has no case-insensitive
duplicate labels. No retail bytes or strings are stored in this repository.

## Header

All numeric fields are unsigned 32-bit little-endian values. The source loader uses signed
`Int` fields, but negative counts and lengths cannot describe bounded input and are rejected
by the implementation as values above its limits.

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | File tag: bytes `20 46 53 43` (ASCII ` FSC`) |
| 4 | 4 | Version; retail Generals uses 3 |
| 8 | 4 | Label-record count |
| 12 | 4 | Total string-record count |
| 16 | 4 | Reserved/skip field; observed as zero |
| 20 | 4 | Language identifier for version 2 and later |

The apparently reversed tags are a consequence of multi-character C++ integer constants
being read as native little-endian integers. They are compared as bytes here to avoid
host-endian behavior.

## Label and string records

Exactly the header's label count follows the header. A label record is:

| Size | Field |
|---:|---|
| 4 | Label tag: bytes `20 4C 42 4C` (ASCII ` LBL`) |
| 4 | Number of string variants belonging to this label |
| 4 | Label byte length |
| variable | Label bytes, with no terminator |
| variable | Declared string records |

A zero variant count is valid. The verified retail file contains one such label.

Each string record is one of:

| Size | Field |
|---:|---|
| 4 | Plain tag `20 52 54 53` (ASCII ` RTS`) or wave tag `57 52 54 53` (ASCII `WRTS`) |
| 4 | Text length in 16-bit code units |
| length * 2 | Little-endian UTF-16 units, each stored with every bit complemented |
| 4 | Wave-name byte length, only for `WRTS` |
| variable | Wave-name bytes, with no terminator, only for `WRTS` |

The decoded code unit is `stored_unit XOR 0xFFFF`. Surrogate pairs are decoded according
to UTF-16; unpaired surrogates are rejected. Label and wave names remain raw byte strings
in the format IR because the source loader does not establish a character encoding for
them. Diagnostic output escapes non-ASCII bytes losslessly.

## Compatibility behavior

- File order, zero-string labels, and every string variant are preserved.
- Duplicate label bytes are preserved rather than collapsed. Lookup precedence remains a
  later semantic-layer decision because the original loader's sort/search does not define
  which duplicate wins.
- The total parsed string count must equal the header declaration.
- The input must end immediately after the declared label records.
- Version and language values are retained. Version 1 treats language as the loader's
  default US language; version 2 and later use the header language identifier.
- Text whitespace is preserved by the decoder. The original client's presentation-time
  whitespace cleanup belongs in a compatibility policy above the file parser.

## Current safety limits

- File: 64 MiB.
- Labels: 100,000.
- Total strings: 1,000,000.
- String variants per label: 65,536.
- Label name: 4,096 bytes.
- Text: 1,048,576 UTF-16 code units per string.
- Wave name: 4,096 bytes.

All lengths and counts are checked before allocation or reading. Truncation, invalid tags,
invalid UTF-16, count mismatches, trailing bytes, and limit excess return structured errors.

## Synthetic fixture

`crates/cic-formats/tests/fixtures/minimal.csf.hex` is an original three-label fixture. It
contains a plain string, a string with a wave name, and a zero-string label. It contains no
retail text or other copyrighted game content.
