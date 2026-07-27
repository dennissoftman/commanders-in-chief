# Map package (`.cicmap`)

One zip archive holding everything one map needs.

## Layout

```text
alpine.cicmap
  map.json                 the scenario -- store uncompressed so diff tools reach it
  terrain/alpine.cict      the terrain container
  models/*.glb             map-specific models, if any
  thumbnail.png            lobby preview, optional
```

`map.json` is required and must be at that exact path. The terrain path is whatever `map.json`'s
`terrain.path` names, so a package may organise its terrain however it likes.

## Why zip

A map is not one kind of data: it is a small diffable description, a large numeric grid, and some
number of binary assets. A single bespoke container would have to reinvent a directory, per-member
compression, and a tool ecosystem. Zip has all three, and a designer can open a map in any file manager
to see what is inside it.

## Loading

A package mounts through the resource layer like any other provider, which means:

- Mount order applies. A package mounted after base content overrides it by the same last-mounted-wins
  rule as everything else.
- A member named `../../etc/passwd` is refused at mount time, by path normalization, rather than by
  anything package-specific.
- Members are indexed but not read. The scenario and terrain are read on open; a model is read when
  something asks for it.

## Cross-checks

The package layer validates what neither format can alone: **every authored position must lie inside the
terrain's world extent.** The scenario knows where things are, the terrain knows how large the world is,
and only the package sees both. Player starts, object placements, and waypoints are all checked, and the
boundary is inclusive — a position exactly on the edge is valid.

Without this, a unit authored outside the map spawns in the void at runtime instead of failing at load.

## Supported compression

Members may be `stored` or `deflate`. Encrypted members and Zip64 archives are refused rather than
mis-parsed — see [M1](../milestones/m1-resources.md) for why.

Store `map.json` uncompressed. It costs a few kilobytes and means a diff tool or a text editor can read
the scenario straight out of the archive, which is the whole reason the scenario is text.

## Limits

Opening a package takes explicit bounds for the archive, the terrain container, and the maximum bytes
read for the scenario and terrain members. Declared uncompressed sizes are bounded at index time, so a
package claiming a terabyte of expansion is refused before anything is allocated.
