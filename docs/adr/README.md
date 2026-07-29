# Architecture decision records

A decision with consequences gets a record. Not every decision — one that a later reader would otherwise
reverse by accident, or re-litigate from scratch, or mistake for an oversight.

## The index

**Every ADR adds its row here in the same commit that adds the file.** That rule is the whole collision
mechanism and it is explained below; adding a record without touching this table defeats it.

| Number | Decision | Status |
|---|---|---|
| [0001](0001-native-asset-formats.md) | Native asset formats rather than another game's | accepted |
| [0002](0002-hand-written-archive-readers.md) | Hand-written archive readers | accepted |
| [0003](0003-renderer-boundary.md) | The renderer boundary | accepted |
| [0004](0004-texture-arrays-and-world-space-tiling.md) | Texture arrays and world-space tiling | accepted |
| [0005](0005-antialiasing-strategy.md) | Antialiasing strategy, and why not MSAA | accepted, implemented |
| [0006](0006-atmosphere.md) | Atmosphere | accepted |
| [0007](0007-simulation-arithmetic.md) | Simulation arithmetic: `f64` and a restricted operation set | accepted |
| [0008](0008-physics-engine.md) | Physics is cosmetic, and Rapier rather than Jolt | accepted |
| [6001](6001-audio-backend-boundary.md) | Where the audio backend boundary goes | accepted, implemented |

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
| `7xxx` | Scripting and content behaviour |
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
