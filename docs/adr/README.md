# Architecture decision records

A decision with consequences gets a record. Not every decision — one that a later reader would otherwise
reverse by accident, or re-litigate from scratch, or mistake for an oversight.

## The index

**Every ADR adds its row here in the same commit that adds the file.** That rule is the whole collision
mechanism and it is explained below; adding a record without touching this table defeats it.

**CI checks that this table agrees with the records** — `tools/check-adr-index.py`, run by
`.github/workflows/docs.yml`. A record with no row, a row with no record, a status the process does not
define, and a row whose status disagrees with the file all fail the build. That check exists because the
table had drifted: six records read `accepted` here while their implementations had long landed, and two
said different things in the two places. Free prose after the status is fine and several records use it —
only the leading token has to agree.

| Number | Decision | Status |
|---|---|---|
| [0001](0001-native-asset-formats.md) | Native asset formats rather than another game's | accepted, implemented |
| [0002](0002-hand-written-archive-readers.md) | Hand-written archive readers | accepted, implemented |
| [0003](0003-renderer-boundary.md) | The renderer boundary | accepted, implemented |
| [0004](0004-texture-arrays-and-world-space-tiling.md) | Texture arrays and world-space tiling | accepted, implemented |
| [0005](0005-antialiasing-strategy.md) | Antialiasing strategy, and why not MSAA | accepted, implemented |
| [0006](0006-atmosphere.md) | Atmosphere | accepted, implemented |
| [0007](0007-simulation-arithmetic.md) | Simulation arithmetic: `f64` and a restricted operation set | accepted, implemented |
| [0008](0008-physics-engine.md) | Physics is cosmetic, and Rapier rather than Jolt | accepted |
| [2001](2001-block-compressed-textures.md) | Block-compressed textures in DDS, and a converter to make them | accepted, implemented |
| [3001](3001-pathfinding.md) | Pathfinding: derived passability, occluders and passage, integer-cost grid A* | accepted and implemented in full; five amendments, four of them built |
| [3002](3002-corridor-economy.md) | The corridor economy: one currency, three acquisitions, carriage on the map | accepted |
| [4001](4001-hdri-sky.md) | Captured skies: Radiance `.hdr`, an equirectangular lookup, and the light derived from it | accepted, implemented |
| [6001](6001-audio-backend-boundary.md) | Where the audio backend boundary goes | accepted, implemented |
| [7001](7001-scripting-language.md) | A scripting language of this project's own | accepted, implemented |
| [7002](7002-script-events.md) | Script events: subscription is a handler, scripts arrive with the scenario | accepted, implemented |

## Numbering

Records 0001–0008 were allocated from a single counter. Everything from here uses a **family prefix**,
because a counter is what produced the collision this convention exists to stop.

```
docs/adr/<F><NNN>-<slug>.md
          │  └── serial within the family, from 001
          └───── family digit
```

| Family | Covers |
|---|---|
| `0xxx` | Grandfathered. The original sequential run, 0001–0008. Not reused. |
| `1xxx` | Foundation: workspace, process, licensing, the invariants themselves |
| `2xxx` | Resources and asset formats — `cic-core`, `cic-vfs`, `cic-assets` |
| `3xxx` | Simulation, determinism, and networking |
| `4xxx` | Rendering — `cic-render`, `cic-camera` |
| `5xxx` | Interface — `cic-ui` |
| `6xxx` | Audio — `cic-audio` |
| `7xxx` | Scripting and content behaviour — `cic-script` |
| `8xxx` | Tooling and pipeline |

The existing records are not renumbered. They are referenced from commit messages, from merged pull
request descriptions, and from each other, and none of that is worth breaking to make a table tidy.

## Why a counter collided, and why a prefix alone would not have fixed it

Two branches open in the same week both took **0007**, and then both took **0008**. Neither author did
anything wrong and neither could have noticed, which is the part worth understanding.

**Git could not see it.** One branch added `0007-simulation-arithmetic.md` and another added
`0007-audio-backend-boundary.md`. Different paths, so they merge cleanly, with no conflict and no
warning — the repository would have ended up with two ADR 0007s and nothing anywhere objecting. A
collision that produces a conflict is an inconvenience; a collision that produces a clean merge is a
defect that ships. This one was caught by hand, during review, which is not a mechanism.

**A family prefix helps, and is not sufficient.** It makes the number *mean* something and it removes the
common case, because two branches touching different subsystems now allocate from different blocks and
cannot collide at all. Simulation arithmetic and the audio boundary would have been `3001` and `6001`.
But two audio ADRs opened the same week would still both reach for `6001`, and git would still merge
them cleanly.

**The index is what closes it.** Because every record must add a row to one table in this file, two
records claiming the same number edit the same lines — so git raises a conflict and somebody resolves it
before the merge, which is exactly when it is cheap. The prefix makes collisions rare; the shared file
makes the rare ones *loud*.

This is the same idea as a part-number scheme: Intel's early catalogue numbered by family rather than
from a global counter, so the memory line and the processor line could be extended independently without
a central allocator arbitrating every new part. What transfers is that a number carrying a category can
be allocated by parallel work without coordination. What does not transfer is the idea that this removes
the need for a registry — it does not, and the table above is ours.

## Writing one

Take the next free serial in your family from the table, add the row and the file in the same commit, and
follow the shape the existing records use:

- **Context** — what forced the decision, including the constraints already in force.
- **Decision** — numbered, so consequences and later records can cite one.
- **Rationale** — why, including what was considered and set aside. A rejected option with its reason is
  the most valuable part of a record, because it is what stops the next person re-proposing it.
- **Consequences** — what this obliges, breaks, or leaves open. Including the unwelcome ones.
- **What implementing it established** — added after the fact, when the implementation contradicted or
  sharpened the decision. Several records here have one and they are the most-read sections.

A record is written when the decision is made, not when the code lands. Status moves from `proposed` to
`accepted` to `accepted, implemented`, and a superseded record is left in place with a line pointing at
whatever replaced it.

Three practical notes, each of them something that went wrong before it was written down:

- **Write the status as `- Status: <token>`,** where the token is one of `proposed`, `accepted`,
  `accepted, implemented`, or `superseded`, and put any explanation after it. The checker reads the
  leading token and the index has to match it.
- **Advance the status when the implementation lands**, in that commit. Nothing else will notice, and a
  record that reads `accepted` long after its code shipped makes every other `accepted` ambiguous.
- **When the implementation contradicts a decision, say so in the record** rather than quietly leaving
  the decision standing. [ADR 0006](0006-atmosphere.md) is the worked example: its decision 6 was
  reversed outright, and the record now carries both the original decision, struck through, and the
  argument that overturned it. A reversal recorded is a decision the next person can re-open; a reversal
  unrecorded is a document that lies.
