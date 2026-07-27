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
  ]
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

Positions are `{ "x", "y", "z" }` in world units, Z up. `z` defaults to `0.0`, meaning "sit on the
terrain".

## Validation

Beyond what the shape enforces:

- Player and waypoint identifiers must be unique.
- An `owner` must resolve to a declared player.
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
