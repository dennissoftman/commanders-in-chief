# Scenario (`map.json`)

The authored half of a map: who plays, where they start, what is placed, and what the named positions
are. JSON, schema version 1.

## Example

```json
{
  "format_version": 1,
  "name": "Alpine Assault",
  "description": "Two-player mountain pass.",
  "terrain": { "path": "terrain/alpine.cict" },
  "players": [
    {
      "id": "north",
      "name": "North",
      "faction": "faction/vanguard",
      "start": { "x": 100.0, "y": 900.0 },
      "team": 1
    }
  ],
  "objects": [
    {
      "template": "prop/pine",
      "position": { "x": 500.0, "y": 500.0 },
      "rotation": 45.0,
      "scale": 1.25
    }
  ],
  "waypoints": [
    { "name": "centre", "position": { "x": 500.0, "y": 500.0 } }
  ],
  "scripts": ["scripts/mission.cics"]
}
```

## Fields

| Field | Required | Default | Notes |
|---|---|---|---|
| `format_version` | yes | — | Must be 1. |
| `name` | yes | — | Must not be blank. |
| `description` | no | `""` | |
| `terrain.path` | yes | — | Package-relative path to the terrain container. |
| `players` | yes | — | At least one. `id` must be unique and non-blank. |
| `players[].team` | no | `0` | Slots sharing a team start allied; `0` is unallied. |
| `objects` | no | `[]` | |
| `objects[].rotation` | no | `0.0` | Degrees about Z. |
| `objects[].scale` | no | `1.0` | Must be finite and positive. |
| `objects[].owner` | no | absent | Must name a declared player when present. Omitted for neutral scenery. |
| `waypoints` | no | `[]` | `name` must be unique. |
| `scripts` | no | `[]` | Package-relative script paths. Order is dispatch order; no repeats. |

Positions are `{ "x", "y", "z" }` in world units, Z up. `z` defaults to `0.0`, meaning "sit on the
terrain".

## Scripts

`scripts` names the [scripts](script.md) this scenario runs, and the array's order is the contract:
when several scripts handle the same event they run in this sequence, back to back, within one tick.
Determinism needs *an* order, and this is the one a designer can see in a diff and change — never a
directory listing, whose order two machines can disagree about.

Three consequences worth stating, all from [ADR 7002](../adr/7002-script-events.md):

- **A script not listed here does not run**, however it got into the package. There is no directory
  scan, so a mod cannot add behaviour by dropping a file in.
- **Every listed script is compiled at load**, against the closed set of verbs and events the kernel
  declares. A script naming something the engine does not offer fails the *load*, with the file, the
  line, and a list of what was available — rather than failing when a player triggers it.
- **A handler is the subscription.** There is no binding table here, because one would put script
  internals in a second file: renaming a function would break a JSON file the compiler never reads.
  A script subscribes by declaring `on tick(elapsed)`, which the compiler already checks.

## Validation

Beyond what the shape enforces:

- Player and waypoint identifiers must be unique.
- An `owner` must resolve to a declared player.
- A script path must be non-blank, and no path may repeat. A repeat would compile and dispatch the
  same handlers twice, which is never what an author meant and is invisible in the run — the events
  simply fire twice.
- Every coordinate, rotation, and scale must be finite; scale must be positive.
- Authored positions must lie within the terrain's world extent. This is checked by the **package**
  loader rather than here, because only it sees both the scenario and the terrain.

## Unknown fields are rejected

Deliberately. A scenario is hand-edited, and a typo in a key should be a loud error at load rather
than a silently-defaulted value that surfaces later as a gameplay bug.

## Why JSON and not a binary encoding

BSON or a custom format would buy a smaller file and a faster parse. Neither matters: the bulk numeric
data lives in the terrain container, so a scenario is kilobytes, and the package compresses it anyway
— which erases most of the size advantage over JSON's repeated keys.

What JSON buys is decisive during development. A scenario is **diffable**: a review can show that a
placement moved, `git blame` can attribute a balance change, two designers' edits can be merged by
hand, and a map can be repaired in a text editor when the tool that wrote it has a bug. None of that
survives a binary encoding.

Output is pretty-printed with a trailing newline for the same reason.
