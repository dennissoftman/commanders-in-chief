# ADR 0002: Hand-written archive readers

- Status: accepted

## Context

The resource layer needs to mount zip and tar containers. Mature crates exist for both. The engine also
has a bounded-reading discipline ([binary parsing invariants](../invariants/binary-parsing.md)) that
every other decoder follows.

## Decision

Write both container readers by hand on the bounded reader. Use `flate2` for the DEFLATE and gzip
streams themselves.

The split is between **container parsing** (hand-written) and **compression** (delegated).

## Rationale

Container parsing is where hostile input does damage: a declared size that would exhaust memory, an
offset pointing outside the archive, a member name escaping the mount root. Those are exactly the cases
the engine's invariants are written for, and having them enforced in one shape across every decoder is
worth more than the code saved.

Concretely, hand-writing means:

- Limits are checked at index time, in the same `ArchiveLimits` shape for both formats, so a caller
  handles a zip failure and a tar failure identically.
- Errors are structured and name the container kind, the entry, and what was expected.
- A member path goes through the same normalization as everything else, so traversal is refused at mount
  time rather than by a separate check.
- Unsupported features fail loudly instead of being approximated.

Compression is the opposite case: DEFLATE is a well-specified algorithm with no path-handling or
allocation-policy decisions in it, and a hand-written implementation would be slower and less tested for
no benefit.

## Consequences

- Two readers to maintain, roughly 250 and 400 lines. Accepted.
- Deliberate gaps, each failing loudly: **Zip64** (needs a different end-of-directory record;
  truncating 64-bit offsets to 32 bits would silently corrupt large archives), **encrypted zip members**
  (no use in game content, and a wrong guess hands the caller ciphertext as data), and **GNU long-name
  tar extensions** (the `ustar` prefix field covers anything this engine needs).
- The zip reader walks the central directory, never the local file headers. Streamed writers leave the
  local header's sizes zero and defer them to a data descriptor, so trusting them is the single most
  common way a hand-written zip reader mis-indexes real archives. Only the local header's name and extra
  *lengths* are read, because those are what physically precede the payload.
- Adding a third container format means writing one reader against `ArchiveIndex`, with no change to the
  resource layer.
