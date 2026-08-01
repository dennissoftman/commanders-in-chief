# Template set (`templates.json`)

What a `template:` identifier resolves to: the data format defining what a unit, structure, prop, or
faction *is*. JSON, schema version 1.

[M6](../milestones/m6-gameplay.md) deferred this format from M2 on purpose — *written once its
consumers are known* — and it is written now because the first consumers exist: scenario activation
resolves every placement and every player's faction against a set, and a drawing host looks up which
model a placed object wears.

## Example

```json
{
  "format_version": 1,
  "templates": [
    { "id": "prop/pine", "kind": "prop", "model": "models/pine.glb" },
    {
      "id": "structure/depot", "kind": "structure", "model": "models/depot.glb",
      "name": "template.depot", "footprint": { "cells": [5, 5] }
    },
    {
      "id": "structure/bridge", "kind": "structure", "model": "models/bridge.glb",
      "passage": { "cells": [2, 9], "class": 2 }
    },
    { "id": "unit/scout", "kind": "unit", "model": "models/scout.glb", "speed": 26.0, "radius": 3.4 },
    { "id": "faction/vanguard", "kind": "faction", "name": "faction.vanguard" }
  ]
}
```

## Fields

| Field | Required | Default | Notes |
|---|---|---|---|
| `format_version` | yes | — | Must be 1. |
| `templates` | yes | — | `id` must be unique and non-blank. |
| `templates[].kind` | yes | — | One of `unit`, `structure`, `prop`, `faction`. |
| `templates[].model` | for placeable kinds | absent | Package-relative `.glb` path. Required for `unit`, `structure`, `prop`; refused for `faction`, which has no pose to draw at. |
| `templates[].name` | no | absent | String-table key for the display name. |
| `templates[].speed` | for `unit` | absent | World units per second, finite and positive. Required for a `unit` — one that cannot move is a structure wearing the wrong kind — and refused for every other kind, which has no movement for it to mean anything to. |
| `templates[].radius` | for `unit` | absent | World units, finite and positive. How much room the unit keeps around itself, which is what stops units standing in each other. Required for a `unit` and refused for every other kind, on the same rule as `speed`. |
| `templates[].footprint` | no | absent | `{ "cells": [x, y] }` — the ground this object *denies*, in whole pathfinding cells. Both extents non-zero. Allowed on `structure` and `prop` only. |
| `templates[].passage` | no | absent | `{ "cells": [x, y], "class": n }` — the ground this object *grants*, and what crossing it costs. Both extents non-zero, class non-zero. Allowed on `structure` and `prop` only. |

## Footprint and passage

[ADR 3001](../adr/3001-pathfinding.md) decision 4. A structure denies the ground it stands on; a
bridge grants passage over water it spans. Both are rectangles of whole cells measured along the
template's own axes, and both are optional — a template may declare either, both, or neither.

- **Precedence is derivation, then passage, then occlusion.** Passage replaces what the terrain
  derived, so a bridge crosses a river; a footprint beats everything, so a depot raised at a
  bridgehead denies the bridgehead — including its own template's passage, which is how a gatehouse
  works.
- **The cost class ladder** is `cic_sim::ground`'s: `1` metalled, `2` graded, `3` plain, `4` mud,
  `5` rubble, and `0` impassable. A `passage` of class `0` is refused, because something that denies
  ground declares a `footprint`; that is what the word means, and the two are not interchangeable.
- **Where the rectangle lands.** The placement's position picks the cell it falls in, and the
  rectangle is centred on that cell — an even extent taking the extra cell on the high side, because
  a rectangle with an even side has no cell at its centre and something has to break the tie. A
  rectangle that runs off the map is clipped where it leaves, not slid inward.
- **Rotation is quantized to quarter turns, and only for the stamp.** A placement's `rotation` is
  rounded to the nearest right angle, which swaps the two extents for a quarter or three-quarter
  turn and does nothing for a half. What is *drawn* rotates freely: rasterizing a rectangle at 37°
  deterministically is solvable and buys nothing for a genre whose buildings have snapped to grids
  since it existed.
- **A `unit` may not declare either.** A mover's own occupancy is its `radius` below, not a grid
  stamp — a footprint that moved would have to be lifted and re-laid every tick. A `faction` may not
  either, having no ground to stand on.

## Radius

[ADR 3001](../adr/3001-pathfinding.md) decision 10, and the other half of the same idea: a standing
object occupies whole **cells** and a moving one occupies a **circle**. A unit's radius is what keeps
units out of each other — after everybody has stepped, overlapping circles push apart, and a push the
ground refuses slides along whichever axis is free.

Required for a `unit` and refused for everything else, on the identical rule as `speed`, and for the
same reason: a template that declares the wrong kind of occupancy has said something it cannot mean,
and that is worth a loud error at load rather than a unit that never gets out of anybody's way. A
radius of zero is refused too — it parses, and what it would actually describe is a unit nothing can
ever push.

## Deliberately minimal, and how it grows

Health, cost, weapons: none are here yet, and that is the point rather than an oversight. A
field nothing consumes is a field nothing tests, which is the same argument that deferred the whole
format from M2. Each arrives with the M6 mechanic that reads it — `speed` was the first to do so,
arriving with movement, then `footprint` and `passage` with the grid stamps of
[ADR 3001](../adr/3001-pathfinding.md) decision 4, then `radius` with decision 10's local
avoidance. Adding an optional field later does not break existing files; changing what an existing
field means takes a version bump.

## One document, overridden wholesale

The set lives at a well-known path, so the resource layer's ordered mounts apply to it as to any other
file: a map package or a mod providing its own `templates.json` replaces the one mounted beneath it
entirely. Per-template merging across mounts is a modding decision for later, taken deliberately rather
than fallen into — wholesale replacement is at least never surprising.

## Where references are checked

The format validates itself (version, unique ids, model presence by kind). What it cannot check is
whether a *scenario's* references resolve, because the scenario and the set may come from different
mounts — so that check lives in **activation**, the last line before a name becomes kernel state, the
same reasoning that puts scenario-versus-terrain bounds checking in the package loader.

## Unknown fields are rejected

Deliberately, as everywhere in this project: a template set is hand-edited, and a typo in a key should
be a loud error at load rather than a silently-defaulted value that surfaces as a balance bug.
