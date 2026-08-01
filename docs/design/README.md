# Design

Documents that constrain what the game *is*, as distinct from how it is built. Everything here is
upstream of code: a rendering choice can be revisited in an afternoon, but a faction that has been
written inconsistently across a hundred lines of briefing text cannot be.

| Document | Scope |
|---|---|
| [faction-bible.md](faction-bible.md) | Faction character: self-conception, voice, doctrine, aesthetic, and how each faction refers to the others. No plot, no named characters. |
| [mechanics.md](mechanics.md) | The rules every faction plays under: the corridor economy, combat, production, vision, victory. Holds the charter of what this game refuses to be. |
| [faction-mechanics.md](faction-mechanics.md) | How each faction expresses those rules — economy, production, technology, information, defence, and the failure mode each one drifts toward. |
| [balance.md](balance.md) | How a number is chosen and what it is checked against: anchors, the budget line, the asymmetry budget, economic benchmarks, and the verification split. |

The four are ordered by what constrains what. The bible is upstream of everything — where it states a
doctrine, mechanics implements it rather than reinterpreting it. `mechanics.md` is upstream of
`faction-mechanics.md`, which is one section per faction against a shared frame. `balance.md` is
downstream of all three and holds no faction character at all, deliberately: it is arithmetic, and its
job is to be checkable.

The economy those three rest on is recorded as a decision rather than left in prose — see
[ADR 3002](../adr/3002-corridor-economy.md), which carries the rejected alternatives. That record
exists because the failure it names is reached by a sequence of individually reasonable
simplifications, and prose cannot stop that. A rejected option with its reason can.

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

[mechanics.md](mechanics.md) continues that claim rather than restating it, and closes it with
[its own list](mechanics.md#10-what-this-document-obliges-the-engine-to-gain) of seventeen requirements
— six already promised or built, and two of them **amendments to an accepted record**, since writing
the economy against [ADR 3001](../adr/3001-pathfinding.md) found that its cost classes cannot express a
road cheaper than plain ground. That defect would have flattened a whole faction's economy and it was
found by writing prose, which is the argument for doing this before the milestone rather than after.

## Status

Working draft, all four documents.

The economy is **proposed and awaiting review** as [ADR 3002](../adr/3002-corridor-economy.md).
`mechanics.md` is draft with it, and `faction-mechanics.md` and `balance.md` are downstream of both —
so a change to the economy's shape changes all three, which is why the record exists to be argued with
first.

No number in the tree has been through [balance.md](balance.md)'s framework, and that is the right
order rather than a gap: [M6](../milestones/m6-gameplay.md) states that its numbers exist to make
mechanics testable and not to be fair, so the framework is what the pass *after* M6 uses. What it buys
by existing now is that the first real number is set by a method instead of establishing one.

The companion presentation document the bible references — bark families, voice generation, radio
processing, and the diegetic footage formats — has still not been written.

## Licence

**This directory is not covered by the engine's Apache-2.0 licence.** Everything here is reserved — see
[LICENSE-CONTENT](../../LICENSE-CONTENT).

That is a deliberate split rather than an oversight: the engine is meant to be reused and the game's world
is not. Reading it, quoting it to discuss or review the project, and proposing changes to it are all fine.
Shipping it in another game is not. Nothing in this directory is needed to build or run the engine, so a
fork takes the software under Apache-2.0 without inheriting anything here.

The reasoning is in [LICENSING.md](../../LICENSING.md), and the terms a contribution here carries are in
[CONTRIBUTING.md](../../CONTRIBUTING.md).
