# Design

Documents that constrain what the game *is*, as distinct from how it is built. Everything here is
upstream of code: a rendering choice can be revisited in an afternoon, but a faction that has been
written inconsistently across a hundred lines of briefing text cannot be.

| Document | Scope |
|---|---|
| [faction-bible.md](faction-bible.md) | Faction character: self-conception, voice, doctrine, aesthetic, and how each faction refers to the others. No plot, no named characters. |

## Why this lives in the repository

Two reasons, both practical.

The bible is a **specification for generated text**. Briefing copy, unit barks, and UI strings will be
written by several people and several tools over the project's life, and the failure mode is not bad
writing — it is *inconsistent* writing, where two factions sound like the same author. A document in
the tree, versioned alongside the code that ships the strings, is the only version anyone can be
expected to have read.

It is also a **source of engine requirements**, and has already been one. Its doctrine sections are why
terrain heights and layer weights are writable GPU textures rather than a baked mesh: a faction whose
map presence is literally paved has to grade roads across terrain at runtime, and a faction that
converts existing buildings into production needs those buildings to be destructible map objects. Both
were nearly free to design for and expensive to retrofit. See
[m3-renderer.md](../milestones/m3-renderer.md).

Per-instance model tint has the same origin: an army fielded from recovered enemy hulls is one mesh
under many markings.

## Status

Working draft. The companion presentation document it references — bark families, voice generation,
radio processing, and the diegetic footage formats — has not been written yet.

## Licence

**This directory is not covered by the engine's Apache-2.0 licence.** Everything here is reserved — see
[LICENSE-CONTENT](../../LICENSE-CONTENT).

That is a deliberate split rather than an oversight: the engine is meant to be reused and the game's world
is not. Reading it, quoting it to discuss or review the project, and proposing changes to it are all fine.
Shipping it in another game is not. Nothing in this directory is needed to build or run the engine, so a
fork takes the software under Apache-2.0 without inheriting anything here.

The reasoning is in [LICENSING.md](../../LICENSING.md), and the terms a contribution here carries are in
[CONTRIBUTING.md](../../CONTRIBUTING.md).
