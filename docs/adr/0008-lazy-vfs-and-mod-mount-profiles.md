# ADR 0008: Lazy VFS providers and mod mount profiles

- Status: accepted
- Date: 2026-07-22

## Context

The VFS already normalized game paths and applied explicit last-mounted-wins overlays, but disk
providers eagerly retained every loose file and every complete BIG archive in memory. Automatic
installed-resource profiles also selected a fixed retail archive subset. Those behaviors do not
scale to large installations or mod stacks, and a built-in retail preset must not become an engine
requirement for custom games or total conversions.

## Decision

Disk-backed VFS providers index metadata only. Directory providers retain the physical path and
indexed length of each regular file. BIG providers read and validate only the bounded header and
directory prefix, then retain the archive path, indexed archive length, member offset, and member
length. Manifest generation and path resolution never read payload bytes.

`ResourceEntry::read(maximum_bytes)` is the only disk payload boundary. It applies the caller's
explicit allocation limit, verifies that the backing file length still matches its indexed length,
seeks to the indexed range, and returns one owned byte vector. The VFS does not retain an implicit
payload cache. In-memory providers remain resident by definition, and in-memory BIG mounts retain
one shared archive allocation rather than copying every member.

Custom bases and total conversions use a bounded UTF-8 line-oriented mount profile. Version 1
requires a leading `version=1` record followed by ordered `mount=<path>` or
`optional=<path>` records. Relative paths resolve against the profile directory. The CLI accepts
one `--profile` and repeatable `--mod` providers; mod providers are appended after the selected
base in command-line order. Existing positional mounts remain an explicit complete base when no
custom profile is supplied, and become additional ordered providers when a profile is supplied.

Generals and Zero Hour archive names remain isolated built-in compatibility presets. Custom
profiles do not validate retail sentinels or require known filenames. Built-in filenames are
matched by ASCII case while retaining the actual physical path; ambiguous case variants are an
error. Virtual game paths continue to use ASCII case folding, while physical user paths are never
lowercased.

The VFS deliberately exposes two views of a normalized path. `resolve` and `iter_resolved` return
the last-mounted winner for opaque replacement resources. `history` returns all provider entries
from earliest to latest mount for cumulative definition formats. The VFS does not guess which
behavior a format needs; the bounded tools/format consumer must choose explicitly from established
source semantics. Built-in Zero Hour profiles always populate that history as Generals first, Zero
Hour second, then mods. Partial INI-style overlays must parse the history in that order instead of
parsing only the winning entry.

Profile input is bounded to 1 MiB, 4,096 providers, and 4,096 UTF-8 bytes per declared path by
default. Loose-directory indices are bounded to 1,000,000 files, 256 nested directories, and
4,096 bytes per virtual path. BIG directory indexing is independently bounded to 64 MiB in
addition to existing archive, entry-count, name, and trailer limits.

## Consequences

- Mounting scales with directory and archive-index metadata rather than total payload size.
- Parsers and image decoders select their own read bounds before allocation.
- Arbitrarily named archives and loose directories can form deterministic custom bases and mod
  overlays without weakening the installed retail presets.
- Resource bytes are reread on repeated requests unless a higher layer deliberately owns a cache.
- Mod dependency solving, package management, hot reload, signing, and authoring tools remain later
  product work.
- New cumulative-definition consumers require an ordered-history regression test; replacement
  consumers retain deterministic last-mounted-wins lookup.
