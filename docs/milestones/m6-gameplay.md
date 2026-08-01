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
  local avoidance between units. **First slice landed** — [ADR 3001](../adr/3001-pathfinding.md)'s
  grid and search, as `cic_sim::ground`: passability derived from the heightfield by slope and water
  line, one cell per sample interval, and A\* on an 8-connected integer-cost grid with ties broken on
  cell index, no diagonal cutting a corner, and an unreachable target degrading to the nearest cell
  the search closed. Routes are string-pulled and `cic_sim::units` walks them, spending one tick's
  travel across as many legs as it reaches — so a corner costs a unit no time, and the move verb's
  encoding did not change. Corners are then **rounded** into a short interpolated arc, with every
  segment checked against the grid so smoothing cannot reintroduce the cut corners the search
  avoided. Every coefficient — grade, water line, cost class, step costs, corner radius — is a
  `GroundRules` setting rather than a constant, and all of them are in the tick hash, because a
  coefficient that changes which way a unit goes changes the game. The grid's fingerprint is in the
  tick hash too.

  **Dynamic obstruction and local avoidance are the parts still outstanding**, and both are waiting
  on a producer rather than on a decision: a `footprint` and a `passage` stamp the grid when
  something is built or destroyed, and nothing constructs anything yet. Repathing on a grid edit
  (decision 7) arrives with the first edit for the same reason.

  Two things this slice needed that were not pathfinding. A subsystem can now **read its peers**
  during a tick — immutably, with a peer registered earlier already advanced — because movement
  asking the ground where a unit may walk is the first cross-subsystem read the kernel has, and it
  is the same seam [M10](m10-scripting.md)'s host verbs need. And the **water table moved into
  `cic-assets`**: presentation floods a map to that line and the simulation refuses to walk under
  it, so two derivations of it would be a unit wading through what the player sees as a lake.
- Combat: weapons, ranges, damage types, armour classes, health, death. **Specified, not yet built** —
  [mechanics.md §3](../design/mechanics.md#3-combat): integers throughout, four damage types against
  five armour classes as integer percentages, and no multiplier anywhere in the table equal to zero,
  because the bible forbids a faction being helpless against anything.
- Economy: a resource, gatherers, and a rate that makes expansion a real decision. **Decided, not yet
  built** — [ADR 3002](../adr/3002-corridor-economy.md) is the corridor economy, accepted on
  2026-08-01: goods enter at map-edge gates, accumulate at yards, and are carried by killable carriers
  to a delivery point, with one currency earned three different ways. It answers this charter line's
  "makes expansion a real decision" concretely — income is `load value ÷ round-trip time` against a
  fixed map flow, so expansion buys a shorter trip on flow somebody else would otherwise take. A route
  link where somebody's assets are being destroyed carries no freight until the fighting stops, which
  is what puts the economy and the fighting on the same map rather than beside each other.

  The three amendments it raised against [ADR 3001](../adr/3001-pathfinding.md) were accepted with it,
  and **two are built**: the cost ladder now runs metalled, graded, plain, mud, rubble, and a cell's
  class sets the pace on it as well as ranking the route across it — so grading is an income increase
  rather than a routing preference. The third, wrecks stamping a class rather than a footprint, waits
  on combat producing a wreck.

  Accepting the record fixed the design and did not schedule it. Its own build order is shared
  carriage first, faction divergence second, and decision 1 — gates, yards, carriage — is the minimum
  viable version.
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
- No balance. Numbers exist to make mechanics testable, not to be fair. The *framework* for setting
  them has been written ahead of the numbers — [balance.md](../design/balance.md), whose anchors,
  budget line and verification split are what the pass after this milestone uses — so the first real
  number is set by a method rather than establishing one. Nothing in the tree has been through it, and
  nothing in this milestone should be.
