# M6: Gameplay

One playable skirmish: build a base, produce units, fight, win or lose.

**Status:** In progress — the template set is the first slice landed, written against its first real
consumers exactly as the deferral intended.

## Charter

- Object templates: the data format defining what a unit or structure is. Deferred to here from M2 on
  purpose, because the format should be written once its consumers are known. **First slice done** —
  see [the specification](../formats/templates.md): identifier, kind, model, display-name key, with
  scenario activation resolving every placement and faction against the set. Health, speed, cost,
  weapons and footprints arrive with the mechanics that read them, for the same reason the format
  itself waited.
- Selection and orders: move, attack, attack-move, stop, hold, patrol, and formation movement.
  **First slice landed** — spawn, move, and stop, as `cic_sim::units`: command payloads decoded by the
  gameplay layer (the kernel keeps them opaque), ownership checked with rejections *counted and
  hashed* so an ignored order is visible rather than silent drift, and straight-line movement in the
  permitted operation set only — a `sqrt` and a division, no trigonometry, arrival snapping exactly to
  the target. Straight-line on purpose: pathfinding is its own charter line below, and a straight line
  is what proves the command-to-motion pipe end to end. The template set gained `speed`, the first
  field to arrive with the mechanic that reads it.
- Pathfinding over the heightfield, with terrain passability, dynamic obstruction from structures, and
  local avoidance between units. **Decided, not yet built** — [ADR 3001](../adr/3001-pathfinding.md):
  passability derived from the heightfield, occluder and passage footprints on templates, and A* on an
  integer-cost grid.
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
[ADR 3001](../adr/3001-pathfinding.md) is that rule applied, and then some: the search runs in integer
arithmetic entirely, so the accumulation questions floats would pose are not managed but absent.

The AI is scoped as a test harness rather than as a good opponent. Being able to run a full match
unattended, repeatably, is worth more at this stage than being challenging.

## Physics

Not in this charter, and [ADR 0008](../adr/0008-physics-engine.md) settles why: an RTS resolves gameplay
through steering, ranges and seeded rolls rather than through rigid bodies, so physics is *spectacle* — the
destroyed tank that tumbles rather than vanishing. Spectacle may differ between two clients without either
being wrong, which is what puts it in presentation.

**No gameplay result may come out of a physics engine.** Anything authoritative — a projectile's impact
point, whether a collapse kills — is integrated in the kernel under
[ADR 0007](../adr/0007-simulation-arithmetic.md)'s arithmetic, and the engine is *told* the answer rather
than asked for it.

The gain is worth stating as plainly as the constraint: because physics decides nothing, it does not have to
be *right*. A wreck may tumble further than its mass allows and a hit may throw debris harder than the round
carried, and neither is a bug. An RTS is played at a distance that rewards legibility over plausibility, and
this is what buys the freedom to choose the first.

The engine will be [Rapier](https://rapier.rs/) rather than Jolt, and that too follows from physics being
cosmetic: Jolt led on a determinism guarantee that is worth nothing once nothing needs guaranteeing, so what
was left to compare was cost, and Rapier is pure Rust. See the ADR for the full comparison and for what would
invert it.

## Explicitly not done

- No campaign, no scripted missions, no cutscenes. A scripting layer is a milestone of its own and wants
  a settled simulation to script against.
- No audio. It touches nothing in the simulation and can land at any time; keeping it out of the
  critical path is the point.
- No balance. Numbers exist to make mechanics testable, not to be fair.
