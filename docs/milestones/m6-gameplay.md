# M6: Gameplay

One playable skirmish: build a base, produce units, fight, win or lose.

**Status:** Planned.

## Charter

- Object templates: the data format defining what a unit or structure is. Deferred to here from M2 on
  purpose, because the format should be written once its consumers are known.
- Selection and orders: move, attack, attack-move, stop, hold, patrol, and formation movement.
- Pathfinding over the heightfield, with terrain passability, dynamic obstruction from structures, and
  local avoidance between units.
- Combat: weapons, ranges, damage types, armour classes, health, death.
- Economy: a resource, gatherers, and a rate that makes expansion a real decision.
- Construction: build sites, placement validity, progress, cancellation.
- Production: queues, prerequisites, cost.
- Fog of war and shroud, per player, with the visibility state living in the simulation.
- Victory and defeat conditions.
- An AI opponent good enough to be a test harness — it exercises every mechanic without a human.

## Exit condition

A full skirmish against the AI on a map package, start to victory, with no desync between a live run
and its replay, and no simulation state reachable only through presentation.

## Design notes

Fog of war lives in the simulation, not the renderer. It has to: what a player can see determines what
their units may target, so it is a gameplay rule the renderer merely visualises. Getting this backwards
makes vision a rendering artefact and vision-based orders non-deterministic.

Pathfinding is the subsystem most likely to break determinism, because the tempting implementations use
hashed containers and floating-point heuristics. It follows the same rules as everything else in the
kernel: ordered containers, pinned arithmetic, stable tie-breaking by object identifier.

The AI is scoped as a test harness rather than as a good opponent. Being able to run a full match
unattended, repeatably, is worth more at this stage than being challenging.

## Physics

Not in this charter, and [ADR 0008](../adr/0008-physics-engine.md) proposes why: an RTS resolves gameplay
through steering, ranges and seeded rolls rather than through rigid bodies, so physics is *spectacle* — the
destroyed tank that tumbles rather than vanishing. Spectacle may differ between two clients without either
being wrong, which is what puts it in presentation.

The consequence for this milestone is that no gameplay result may come out of a physics engine. Anything
authoritative — a projectile's impact point, whether a collapse kills — is integrated in the kernel under
[ADR 0007](../adr/0007-simulation-arithmetic.md)'s arithmetic, and the engine is *told* the answer rather
than asked for it. That ADR is `proposed` rather than accepted, and settling it is a decision for whoever
writes this charter's detail.

## Explicitly not done

- No campaign, no scripted missions, no cutscenes. A scripting layer is a milestone of its own and wants
  a settled simulation to script against.
- No audio. It touches nothing in the simulation and can land at any time; keeping it out of the
  critical path is the point.
- No balance. Numbers exist to make mechanics testable, not to be fair.
