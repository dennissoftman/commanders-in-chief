# Binary parsing invariants

Every decoder in this project treats its input as hostile, whether it came from a downloaded mod, a
map someone shared, or a file the engine wrote itself and a disk corrupted.

- Every read is checked against the current bounded region before indexing.
- Offsets and lengths use checked arithmetic.
- Sub-readers cannot escape the parent reader's byte slice.
- Input-supplied counts, strings, nesting depths, and allocation sizes have explicit limits, passed
  in by the caller rather than hardcoded.
- A limit is checked **before** the allocation it bounds, not after. A container declaring a
  gigabyte is refused while only its header has been read.
- Malformed input returns a structured error naming what was found, what was expected, and where.
- Decoders do not panic, do not allocate from unchecked counts, and do not partially mutate shared
  state before failing.
- Negative tests cover truncation at several offsets, invalid offsets, invalid encoding, wrong
  magic, an unsupported version, and each limit being crossed.
- Compression is bounded on the *decompressed* size. A small payload claiming a huge expansion is
  refused before inflation runs, not after it succeeds.

## Why this is not paranoia

Two of these rules exist because of properties this engine needs rather than because of security
theatre. A decoder that cannot panic is a decoder that can run inside a simulation tick without
taking the process down mid-frame. A decoder whose limits are caller-supplied is one the editor can
run with generous bounds and a multiplayer client can run with strict ones, using the same code.

## Forward compatibility

Chunked container formats skip unknown chunks rather than refusing them, so data written by a newer
build stays readable by an older one. Versioned formats refuse an unknown *version* outright: a
version bump means the meaning of existing fields may have changed, which is not something to guess
at.
