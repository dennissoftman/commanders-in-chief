# Binary Parsing Invariants

- Every read is checked against the current bounded region before indexing.
- Offsets and lengths use checked arithmetic.
- Sub-readers cannot escape the parent reader's byte slice.
- File-supplied counts, strings, nesting, and allocations have explicit limits.
- Malformed input returns a structured error containing source and byte offset.
- Parsers do not panic, allocate from unchecked counts, or partially mutate global state.
- Negative fixtures cover truncation, invalid offsets, invalid encoding, and limit excess.

