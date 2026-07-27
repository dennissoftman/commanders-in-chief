# M1: Resource layer

Give the engine one way to name and fetch a resource, whatever it physically lives in, with an
override order the player controls.

**Status:** Complete.

## Charter

Every later milestone loads something. If each one invents its own path handling, mod support becomes
impossible to reason about and case-sensitivity bugs appear only on the platforms nobody tested.

- A canonical virtual path: `/` separators, case folded, `.` collapsed, `..` refused outright.
- Mounts as an explicit, ordered list. Later mounts override earlier ones — nothing infers precedence
  from a filename or a timestamp.
- Full override history retained per path, so a cumulative definition format can read every version
  while an opaque resource takes only the winner.
- Providers: loose directories, in-memory bytes, and archive containers.
- Archive containers behind one index abstraction, so adding a format means writing one reader rather
  than touching the resource layer.
- Lazy reads: indexing an archive does not read its members, and a member is decompressed only when
  something asks for it.
- A backing file that changed after indexing is detected rather than read as garbage.

## Exit condition

Met. Directory, memory, zip, tar, and gzip-framed tar mounts all resolve through one API; the
override order, the zip-bomb refusals, the traversal refusals, and the changed-file detection are all
covered by tests.

## Design notes

Both archive readers are hand-written on the bounded reader rather than delegated to a crate. That is
a deliberate cost: it keeps hostile-input handling uniform with the rest of the engine, keeps the
dependency surface small, and means a limit is enforced at index time in the same shape everywhere.
`flate2` is used for the DEFLATE and gzip streams themselves, which is compression rather than
container parsing.

The zip reader walks the central directory, never the local file headers. Streamed writers leave the
local header's sizes zero and defer them to a data descriptor, so trusting them is the most common way
a hand-written zip reader mis-indexes real archives.

## Explicitly not done

- **Zip64** is refused rather than mis-parsed. It needs a different end-of-directory record, and
  truncating its 64-bit offsets to 32 bits would silently corrupt large archives.
- **Encrypted zip members** are refused. There is no use for them in game content, and a wrong guess
  would hand a caller ciphertext as if it were data.
- **GNU long-name tar extensions** are skipped rather than truncated. The `ustar` prefix field covers
  paths well past anything this engine needs.
- No async or streaming read path. Reads are synchronous and bounded; if load times need overlapping
  I/O, that is a change to make with a measurement in hand.
